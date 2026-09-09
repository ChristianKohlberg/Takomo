mod common;

use common::TestApp;
use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use yrs::encoding::read::{Cursor, Read as _};
use yrs::encoding::write::Write as _;
use yrs::updates::decoder::Decode;
use yrs::{Doc, Map, ReadTxn, StateVector, Transact, Update, XmlElementPrelim, XmlFragment};

async fn seed(app: &TestApp, title: &str) -> Value {
    let (status, doc) = app
        .post(
            &app.admin,
            "/v1/projects/tp/documents",
            json!({
                "title": title, "path": "plans", "status": "review", "metadata": {"keep": true}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{doc}");
    let replica = Doc::new();
    let prose = replica.get_or_insert_xml_fragment("prose");
    let proposals = replica.get_or_insert_map("proposals");
    let update = {
        let mut txn = replica.transact_mut();
        prose.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        proposals.insert(&mut txn, "old-proposal", "old content");
        txn.encode_update_v1()
    };
    app.open_store()
        .append_collab_update(doc["id"].as_str().unwrap(), &update, "test")
        .unwrap();
    doc
}

fn persisted(app: &TestApp, id: &str) -> Doc {
    let doc = Doc::new();
    for update in app.open_store().load_collab_updates(id).unwrap() {
        doc.transact_mut()
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    doc
}

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.write_var(0u64);
    out.write_var(2u64);
    out.write_buf(payload);
    out
}

#[tokio::test]
async fn document_reset_persists_and_broadcasts_tombstones_preserving_identity() {
    let app = TestApp::spawn().await;
    let before = seed(&app, "Keep this title").await;
    let other = seed(&app, "Untouched").await;
    let id = before["id"].as_str().unwrap();
    let replica = persisted(&app, id);
    let old_update = replica
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let (_, session) = app
        .post(
            &app.admin,
            &format!("/v1/documents/{id}/session"),
            json!({}),
        )
        .await;
    let url = format!(
        "{}/v1/docsync/{id}?ticket={}",
        app.base.replace("http://", "ws://"),
        session["token"].as_str().unwrap()
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(url.clone()).await.unwrap();
    let (mut witness, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    // A peer's edit the room has applied but not yet flushed. It reaches the
    // replica before the queue, so the reset must carry its tombstone rather
    // than drop it from the queue and leave it revivable.
    let live_update = {
        // The root is fetched BEFORE the write transaction opens: `get_or_insert_map`
        // takes the document's own lock, and taking it under `transact_mut` deadlocks.
        let proposals = replica.get_or_insert_map("proposals");
        let mut txn = replica.transact_mut();
        proposals.insert(&mut txn, "live-proposal", "typed just now");
        txn.encode_update_v1()
    };
    socket
        .send(Message::Binary(frame(&live_update).into()))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Message::Binary(bytes) = witness.next().await.unwrap().unwrap() {
                let mut reader = Cursor::new(bytes.as_ref());
                if reader.read_var::<u64>().unwrap() != 0 {
                    continue;
                }
                let sync_kind: u64 = reader.read_var().unwrap();
                if sync_kind == 2 && reader.read_buf().unwrap() == live_update.as_slice() {
                    break;
                }
            }
        }
    })
    .await
    .expect("the room relays the live edit, so it has applied it");

    let (status, after) = app
        .post(
            &app.admin,
            &format!("/v1/documents/{id}/reset"),
            json!({"confirm_id": id}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    for key in [
        "id",
        "project",
        "title",
        "path",
        "status",
        "initiative",
        "metadata",
        "created_at",
        "created_by",
    ] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert_eq!(
        after["version"].as_i64(),
        Some(before["version"].as_i64().unwrap() + 1)
    );

    // The HTTP success is already durable, without waiting for the room flusher.
    let saved = persisted(&app, id);
    assert_eq!(
        saved
            .get_or_insert_xml_fragment("prose")
            .len(&saved.transact()),
        0
    );
    assert_eq!(
        saved.get_or_insert_map("proposals").len(&saved.transact()),
        0
    );
    assert!(saved
        .get_or_insert_map("document_control")
        .get(&saved.transact(), "reset")
        .is_some());
    // The unflushed edit is tombstoned in the committed state: the log holds
    // exactly the reset row, and replaying the edit's own update revives nothing.
    assert_eq!(app.open_store().load_collab_updates(id).unwrap().len(), 1);
    saved
        .transact_mut()
        .apply_update(Update::decode_v1(&live_update).unwrap())
        .unwrap();
    assert_eq!(
        saved.get_or_insert_map("proposals").len(&saved.transact()),
        0
    );
    let other_doc = persisted(&app, other["id"].as_str().unwrap());
    assert_eq!(
        other_doc
            .get_or_insert_xml_fragment("prose")
            .len(&other_doc.transact()),
        1
    );

    // The open browser sees the deletion on its existing replica, and merging
    // its stale pre-reset state afterwards cannot bring those items back.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Binary(bytes) = message {
                let mut reader = Cursor::new(bytes.as_ref());
                let kind: u64 = reader.read_var().unwrap();
                if kind != 0 {
                    continue;
                }
                let sync_kind: u64 = reader.read_var().unwrap();
                if sync_kind == 1 || sync_kind == 2 {
                    let payload = reader.read_buf().unwrap();
                    replica
                        .transact_mut()
                        .apply_update(Update::decode_v1(payload).unwrap())
                        .unwrap();
                    if replica
                        .get_or_insert_map("document_control")
                        .get(&replica.transact(), "reset")
                        .is_some()
                    {
                        break;
                    }
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        replica
            .get_or_insert_xml_fragment("prose")
            .len(&replica.transact()),
        0
    );
    replica
        .transact_mut()
        .apply_update(Update::decode_v1(&old_update).unwrap())
        .unwrap();
    assert_eq!(
        replica
            .get_or_insert_xml_fragment("prose")
            .len(&replica.transact()),
        0
    );
    socket
        .send(Message::Binary(frame(&old_update).into()))
        .await
        .unwrap();
    socket.close(None).await.unwrap();
    witness.close(None).await.unwrap();
    // A second reset sees the same empty live room; stale content is not revived.
    let (status, _) = app
        .post(
            &app.admin,
            &format!("/v1/documents/{id}/reset"),
            json!({"confirm_id": id}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let final_doc = persisted(&app, id);
    assert_eq!(
        final_doc
            .get_or_insert_xml_fragment("prose")
            .len(&final_doc.transact()),
        0
    );
}

#[tokio::test]
async fn document_reset_requires_admin_project_access_confirmation_and_active_document() {
    let app = TestApp::spawn().await;
    let doc = seed(&app, "Protected").await;
    let id = doc["id"].as_str().unwrap();
    let path = format!("/v1/documents/{id}/reset");
    for token in [&app.worker, &app.human] {
        assert_eq!(
            app.post(token, &path, json!({"confirm_id": id})).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let limited = app.mint("limited", &["read", "write", "admin"], Some(&["another"]));
    assert_eq!(
        app.post(&limited, &path, json!({"confirm_id": id})).await.0,
        StatusCode::FORBIDDEN
    );
    for (body, expected) in [
        (json!({}), StatusCode::BAD_REQUEST),
        (
            json!({"confirm_id": "wrong"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"confirm_id": id, "unexpected": true}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(app.post(&app.admin, &path, body).await.0, expected);
    }
    let (_, mismatch) = app
        .post(&app.admin, &path, json!({"confirm_id": "wrong"}))
        .await;
    assert_eq!(mismatch["code"], "validation.confirm_id", "{mismatch}");
    assert_eq!(
        app.post(&app.admin, "/v1/projects/tp/archive", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        app.post(&app.admin, &path, json!({"confirm_id": id}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    app.open_store().archive_document(id, "test").unwrap();
    assert_eq!(
        app.post(&app.admin, &path, json!({"confirm_id": id}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let still_there = persisted(&app, id);
    assert_eq!(
        still_there
            .get_or_insert_xml_fragment("prose")
            .len(&still_there.transact()),
        1
    );
    assert_eq!(
        still_there
            .get_or_insert_map("proposals")
            .len(&still_there.transact()),
        1
    );
}

#[tokio::test]
async fn specification_reset_clears_shared_content_and_preserves_history_and_linked_work() {
    let app = TestApp::spawn().await;
    let (status, created) = app.post(&app.admin, "/v1/mindmaps", json!({
        "project": "tp", "title": "Keep the plan", "summary": "Clear summary", "metadata": {"path": "plans"}
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["mindmap"]["id"].as_str().unwrap();
    let (status, node) = app
        .post(
            &app.admin,
            &format!("/v1/mindmaps/{id}/nodes"),
            json!({"text":"Old section", "notes":"Old prose"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{node}");
    let (_, history) = app
        .get(&app.admin, &format!("/v1/mindmaps/{id}/versions"))
        .await;
    let version = history["head"].as_i64().unwrap();
    let version_path = format!("/v1/mindmaps/{id}/versions/{version}");
    let (_, historical) = app.get(&app.admin, &version_path).await;
    let node_id = node["nodes"][0]["id"].as_str().unwrap();
    let (status, ticket) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({"project":"tp","title":"Keep work"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    let (status, check) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({"title":"Keep check","node":node_id}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{check}");
    let check_id = check["id"].as_str().unwrap();
    let (_, check_before) = app.get(&app.admin, &format!("/v1/checks/{check_id}")).await;
    let ticket_id = ticket["id"].as_str().unwrap();
    let (_, ticket_before) = app
        .get(&app.admin, &format!("/v1/tickets/{ticket_id}"))
        .await;

    // The active room predates an out-of-process writer. Reset must include
    // that committed content, rather than replacing a stale cached snapshot.
    let (_, session) = app
        .post(&app.admin, &format!("/v1/mindmaps/{id}/session"), json!({}))
        .await;
    let url = format!(
        "{}/v1/docsync/{id}?ticket={}",
        app.base.replace("http://", "ws://"),
        session["token"].as_str().unwrap()
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    socket.next().await.unwrap().unwrap();
    let external = persisted(&app, id);
    let nodes = external.get_or_insert_map("nodes");
    let proposals = external.get_or_insert_map("proposals");
    let comments = external.get_or_insert_map("documentComments");
    let relationships = external.get_or_insert_map("relationships");
    let extra = {
        let mut txn = external.transact_mut();
        nodes.insert(&mut txn, "external-node", "External content");
        proposals.insert(&mut txn, "proposal", "Old proposal");
        comments.insert(&mut txn, "comment", "Old comment");
        relationships.insert(&mut txn, "link", "Old relationship");
        txn.encode_update_v1()
    };
    app.open_store()
        .append_collab_update(id, &extra, "external")
        .unwrap();
    let path = format!("/v1/mindmaps/{id}/reset");
    let (status, after) = app.post(&app.admin, &path, json!({"confirm_id":id})).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    for key in [
        "id",
        "project",
        "title",
        "metadata",
        "status",
        "created_by",
        "created_at",
    ] {
        assert_eq!(after["mindmap"][key], created["mindmap"][key], "{key}");
    }
    assert_eq!(after["mindmap"]["summary"], "");
    assert_eq!(after["mindmap"]["nodes"], 0);
    let cleared = persisted(&app, id);
    for field in ["nodes", "proposals", "relationships", "documentComments"] {
        assert_eq!(
            cleared.get_or_insert_map(field).len(&cleared.transact()),
            0,
            "{field}"
        );
    }
    // Every known old insertion stays deleted even if an old peer resends it.
    let stale = external
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    cleared
        .transact_mut()
        .apply_update(Update::decode_v1(&stale).unwrap())
        .unwrap();
    assert_eq!(
        cleared.get_or_insert_map("nodes").len(&cleared.transact()),
        0
    );
    assert_eq!(app.get(&app.admin, &version_path).await.1, historical);
    // The plan's own history says so: one plan-wide act for the reset, beside
    // the section's earlier authoring rather than instead of it.
    let (_, trace) = app
        .get(&app.admin, &format!("/v1/mindmaps/{id}/trace"))
        .await;
    let entries = trace["items"].as_array().unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|e| e["kind"] == "pruned" && e["node"].is_null())
            .count(),
        1,
        "{trace}"
    );
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == "authored" && e["node"] == node_id),
        "{trace}"
    );
    assert_eq!(
        app.get(&app.admin, &format!("/v1/tickets/{ticket_id}"))
            .await
            .1,
        ticket_before
    );
    assert_eq!(
        app.get(&app.admin, &format!("/v1/checks/{check_id}"))
            .await
            .1,
        check_before
    );
    socket.close(None).await.unwrap();
    let (status, new_node) = app
        .post(
            &app.admin,
            &format!("/v1/mindmaps/{id}/nodes"),
            json!({"text":"Fresh section"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{new_node}");
    let (_, reopened) = app.get(&app.admin, &format!("/v1/mindmaps/{id}")).await;
    assert_eq!(reopened["nodes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn specification_reset_rejects_wrong_identity_scope_and_archived_project() {
    let app = TestApp::spawn().await;
    let (_, created) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Protected"}),
        )
        .await;
    let id = created["mindmap"]["id"].as_str().unwrap();
    let path = format!("/v1/mindmaps/{id}/reset");
    for token in [&app.worker, &app.human] {
        assert_eq!(
            app.post(token, &path, json!({"confirm_id":id})).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let limited = app.mint("limited", &["read", "write", "admin"], Some(&["other"]));
    assert_eq!(
        app.post(&limited, &path, json!({"confirm_id":id})).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, mismatch) = app
        .post(&app.admin, &path, json!({"confirm_id":"wrong"}))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(mismatch["code"], "validation.confirm_id", "{mismatch}");
    assert_eq!(
        app.post(&app.admin, &path, json!({})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.post(&app.admin, "/v1/projects/tp/archive", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        app.post(&app.admin, &path, json!({"confirm_id":id}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.get(&app.admin, &format!("/v1/mindmaps/{id}")).await.1["mindmap"]["title"],
        "Protected"
    );
}
