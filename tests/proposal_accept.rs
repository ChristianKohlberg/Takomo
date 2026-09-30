//! An agent accepts or rejects a proposal over HTTP.
//!
//! `POST /v1/mindmaps/{id}/proposals/{proposal}/accept|reject` and the document
//! twins. The claims these tests hold the server to:
//!
//! - an accept applies the ops to the LIVE replica — a peer already connected
//!   receives the change over its socket, and the change is durable before the
//!   response says it happened;
//! - the decision is recorded like a browser's (`decided_by` = the caller,
//!   `decided_at`), and only a pending proposal can be decided;
//! - inline markdown lands as marks and reads back as markdown, so reading a
//!   section and proposing it back unchanged is still refused as a no-op;
//! - what the server cannot build exactly (an HTML table) is refused and
//!   changes nothing; an accept with nothing left to apply leaves it pending.
mod common;

use common::TestApp;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use yrs::encoding::read::{Cursor, Read as _};
use yrs::types::text::YChange;
use yrs::updates::decoder::Decode;
use yrs::{
    Any, GetString, Map, Out, Text, Transact, Update, Xml, XmlElementPrelim, XmlFragment, XmlOut,
    XmlTextPrelim,
};

fn record(id: &str, ops: Value) -> String {
    json!({
        "id": id, "node": null, "status": "pending", "author": "agent:w2",
        "instruction": "", "summary": "", "created_at": 1, "skipped": [], "ops": ops,
    })
    .to_string()
}

/// A document with two paragraphs and three pending proposals, written to the
/// log before any room opens — the state a document is in after an agent
/// proposed and everybody went home.
async fn seeded_document(app: &TestApp) -> String {
    let (status, doc) = app
        .post(
            &app.admin,
            "/v1/projects/tp/documents",
            json!({ "title": "Accept me" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    let replica = yrs::Doc::new();
    let prose = replica.get_or_insert_xml_fragment("prose");
    let proposals = replica.get_or_insert_map("proposals");
    let update = {
        let mut txn = replica.transact_mut();
        for (block, text) in [("blk_a", "Alpha"), ("blk_b", "Beta")] {
            let p = prose.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            p.insert_attribute(&mut txn, "id", block);
            p.push_back(&mut txn, XmlTextPrelim::new(text));
        }
        proposals.insert(
            &mut txn,
            "prop-good",
            record(
                "prop-good",
                json!([
                    {"op": "replace", "id": "blk_a", "markdown": "Now **bold** and [linked](https://e.io)."},
                    {"op": "insert_after", "id": "blk_a", "markdown": "- one\n- two"},
                ]),
            ),
        );
        proposals.insert(
            &mut txn,
            "prop-table",
            record(
                "prop-table",
                json!([{"op": "replace", "id": "blk_b", "markdown": "<table><tr><td>x</td></tr></table>"}]),
            ),
        );
        proposals.insert(
            &mut txn,
            "prop-stale",
            record(
                "prop-stale",
                json!([{"op": "replace", "id": "blk_gone", "markdown": "x"}]),
            ),
        );
        proposals.insert(
            &mut txn,
            "prop-no",
            record("prop-no", json!([{"op": "delete", "id": "blk_b"}])),
        );
        txn.encode_update_v1()
    };
    app.open_store()
        .append_collab_update(&id, &update, "test")
        .unwrap();
    id
}

fn persisted(app: &TestApp, id: &str) -> yrs::Doc {
    let doc = yrs::Doc::new();
    for update in app.open_store().load_collab_updates(id).unwrap() {
        doc.transact_mut()
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    doc
}

fn proposal(doc: &yrs::Doc, id: &str) -> Value {
    let map = doc.get_or_insert_map("proposals");
    let txn = doc.transact();
    match map.get(&txn, id) {
        Some(Out::Any(Any::String(s))) => serde_json::from_str(&s).unwrap(),
        other => panic!("no proposal {id}: {other:?}"),
    }
}

#[tokio::test]
async fn an_agent_accepts_a_document_proposal_on_the_live_replica() {
    let app = TestApp::spawn().await;
    let id = seeded_document(&app).await;

    // A browser, already connected, holding the document as it was.
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
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let replica = persisted(&app, &id);

    let (status, out) = app
        .post(
            &app.worker,
            &format!("/v1/documents/{id}/proposals/prop-good/accept"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["status"], "accepted", "{out}");
    assert_eq!(out["applied"], json!(2), "{out}");
    assert_eq!(out["skipped"], json!([]), "{out}");
    assert_eq!(out["proposal"]["decided_by"], "agent:w1", "{out}");
    assert!(out["proposal"]["decided_at"].as_i64().unwrap() > 0, "{out}");

    // The connected peer receives it — this is what "on the live replica" buys.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let Message::Binary(bytes) = socket.next().await.unwrap().unwrap() else {
                continue;
            };
            let mut reader = Cursor::new(bytes.as_ref());
            if reader.read_var::<u64>().unwrap() != 0 {
                continue;
            }
            let kind: u64 = reader.read_var().unwrap();
            if kind != 1 && kind != 2 {
                continue;
            }
            let payload = reader.read_buf().unwrap();
            replica
                .transact_mut()
                .apply_update(Update::decode_v1(payload).unwrap())
                .unwrap();
            if proposal(&replica, "prop-good")["status"] == "accepted" {
                break;
            }
        }
    })
    .await
    .expect("the connected peer receives the accepted change");

    // Durable before the response, and structured the way the editor writes it:
    // the replaced block keeps its id, the bold is a mark, the list is a list.
    for doc in [&replica, &persisted(&app, &id)] {
        let prose = doc.get_or_insert_xml_fragment("prose");
        let txn = doc.transact();
        let blocks: Vec<_> = prose.children(&txn).collect();
        assert_eq!(blocks.len(), 3);
        let XmlOut::Element(first) = &blocks[0] else {
            panic!("element")
        };
        assert_eq!(first.tag().as_ref(), "paragraph");
        assert_eq!(
            first.get_attribute(&txn, "id"),
            Some(Out::Any(Any::from("blk_a")))
        );
        let Some(XmlOut::Text(text)) = first.get(&txn, 0) else {
            panic!("text")
        };
        let diff = text.diff(&txn, YChange::identity);
        assert_eq!(diff[1].insert.clone().to_string(&txn), "bold");
        assert!(diff[1].attributes.as_ref().unwrap().contains_key("bold"));
        let link = diff[3].attributes.as_ref().unwrap();
        assert!(
            matches!(link.get("link"), Some(Any::Map(m)) if m.get("href") == Some(&Any::from("https://e.io"))),
            "{link:?}"
        );
        let XmlOut::Element(list) = &blocks[1] else {
            panic!("element")
        };
        assert_eq!(list.tag().as_ref(), "bulletList");
        let fresh = list.get_attribute(&txn, "id").unwrap().to_string(&txn);
        assert!(fresh.starts_with("blk_") && fresh != "blk_a", "{fresh}");
        assert_eq!(list.get_string(&txn).matches("<listItem>").count(), 2);
    }

    // Only a pending proposal is decided, in either direction.
    for verb in ["accept", "reject"] {
        let (status, again) = app
            .post(
                &app.worker2,
                &format!("/v1/documents/{id}/proposals/prop-good/{verb}"),
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{again}");
        assert_eq!(again["code"], "conflict.proposal_decided", "{again}");
    }
    assert_eq!(
        proposal(&persisted(&app, &id), "prop-good")["decided_by"],
        "agent:w1",
        "a refused second decision must not overwrite the first"
    );
}

#[tokio::test]
async fn refusals_change_nothing_and_rejecting_only_marks_the_record() {
    let app = TestApp::spawn().await;
    let id = seeded_document(&app).await;
    let before = persisted(&app, &id)
        .get_or_insert_xml_fragment("prose")
        .get_string(&persisted(&app, &id).transact());
    let prose_now = || {
        let doc = persisted(&app, &id);
        let frag = doc.get_or_insert_xml_fragment("prose");
        let txn = doc.transact();
        frag.get_string(&txn)
    };

    // An HTML table is not something the server can build exactly.
    let (status, out) = app
        .post(
            &app.worker,
            &format!("/v1/documents/{id}/proposals/prop-table/accept"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{out}");
    assert_eq!(out["code"], "validation.proposal_unsupported", "{out}");

    // Nothing left to apply is not an acceptance.
    let (status, out) = app
        .post(
            &app.worker,
            &format!("/v1/documents/{id}/proposals/prop-stale/accept"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{out}");
    assert_eq!(out["code"], "conflict.proposal_stale", "{out}");
    assert!(
        out["message"].as_str().unwrap().contains("blk_gone"),
        "{out}"
    );

    // Unknown proposal, and a caller who may only read.
    let (status, out) = app
        .post(
            &app.worker,
            &format!("/v1/documents/{id}/proposals/prop-nope/accept"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{out}");
    assert_eq!(out["code"], "notfound.proposal", "{out}");
    let reader = app.mint("agent:reader", &["read"], None);
    let (status, out) = app
        .post(
            &reader,
            &format!("/v1/documents/{id}/proposals/prop-no/reject"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    assert_eq!(out["code"], "auth.scope", "{out}");

    // Rejecting marks the record and leaves the prose alone — even a delete.
    let (status, out) = app
        .post(
            &app.worker,
            &format!("/v1/documents/{id}/proposals/prop-no/reject"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["status"], "rejected", "{out}");
    assert_eq!(out["applied"], json!(0), "{out}");

    let saved = persisted(&app, &id);
    for (pid, status) in [
        ("prop-table", "pending"),
        ("prop-stale", "pending"),
        ("prop-no", "rejected"),
    ] {
        assert_eq!(proposal(&saved, pid)["status"], status, "{pid}");
    }
    assert_eq!(proposal(&saved, "prop-no")["decided_by"], "agent:w1");
    assert_eq!(
        prose_now(),
        before,
        "no refusal and no rejection may touch the prose"
    );
}

async fn plan_with_section(app: &TestApp) -> (String, String, String) {
    let (_, made) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (s, out) = app
        .post(
            &app.admin,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "API", "notes": "Versioning is undecided." }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{out}");
    let node = out["nodes"][0]["id"].as_str().unwrap().to_string();
    let (_, read) = app
        .get(&app.admin, &format!("/v1/mindmaps/{map}/prose?node={node}"))
        .await;
    let md = read["markdown"].as_str().unwrap();
    let block = md
        .split("<!-- ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .expect("a block id")
        .to_string();
    (map, node, block)
}

async fn propose(app: &TestApp, map: &str, node: &str, ops: Value) -> String {
    let (s, out) = app
        .post(
            &app.worker2,
            &format!("/v1/mindmaps/{map}/proposals"),
            json!({ "node": node, "operations": ops, "summary": "why" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{out}");
    out["proposal"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn an_agent_accepts_a_plan_proposal_and_reads_the_formatting_back() {
    let app = TestApp::spawn().await;
    let (map, node, block) = plan_with_section(&app).await;

    let text = "Decided: **v1** forever, _no_ `v2`, see [the spec](https://x.io/a_b).";
    let good = propose(
        &app,
        &map,
        &node,
        json!([
            {"op": "replace", "id": block, "markdown": text},
            {"op": "insert_after", "id": block, "markdown": "- **one**\n- two"},
        ]),
    )
    .await;
    let (s, out) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/proposals/{good}/accept"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["node"], json!(node), "{out}");
    assert_eq!(out["applied"], json!(2), "{out}");
    assert_eq!(out["proposal"]["decided_by"], "agent:w1", "{out}");

    // An agent reading the section sees the formatting a reader sees.
    let (_, read) = app
        .get(&app.admin, &format!("/v1/mindmaps/{map}/prose?node={node}"))
        .await;
    let md = read["markdown"].as_str().unwrap();
    assert!(md.contains(&format!("<!-- {block} -->\n{text}")), "{md}");
    assert!(md.contains("- **one**\n- two"), "{md}");

    // So proposing it back unchanged is still recognised as a no-op.
    let (s, same) = app
        .post(
            &app.worker2,
            &format!("/v1/mindmaps/{map}/proposals"),
            json!({ "node": node, "operations": [{"op": "replace", "id": block, "markdown": text}] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{same}");
    assert_eq!(same["code"], "validation.document_unchanged", "{same}");

    // The decision is on the record and in the plan's history.
    let (_, listed) = app
        .get(
            &app.admin,
            &format!("/v1/mindmaps/{map}/proposals?node={node}&status=accepted"),
        )
        .await;
    assert_eq!(listed["total"], json!(1), "{listed}");
    let (_, trace) = app
        .get(&app.admin, &format!("/v1/mindmaps/{map}/trace?node={node}"))
        .await;
    let accepted = trace["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "accepted")
        .unwrap_or_else(|| panic!("no accepted entry: {trace}"));
    assert_eq!(accepted["actor"], "agent:w1", "{accepted}");
}

#[tokio::test]
async fn a_plan_accept_skips_and_reports_ops_whose_block_is_gone() {
    let app = TestApp::spawn().await;
    let (map, node, block) = plan_with_section(&app).await;

    let add = propose(
        &app,
        &map,
        &node,
        json!([{"op": "insert_after", "id": block, "markdown": "Second."}]),
    )
    .await;
    app.post(
        &app.worker,
        &format!("/v1/mindmaps/{map}/proposals/{add}/accept"),
        json!({}),
    )
    .await;
    let (_, read) = app
        .get(&app.admin, &format!("/v1/mindmaps/{map}/prose?node={node}"))
        .await;
    let second = read["markdown"]
        .as_str()
        .unwrap()
        .split("<!-- ")
        .nth(2)
        .and_then(|r| r.split(' ').next())
        .unwrap()
        .to_string();

    // Two proposals made against the same state; the first deletes a block the
    // second also addresses.
    let delete = propose(&app, &map, &node, json!([{"op": "delete", "id": second}])).await;
    let both = propose(
        &app,
        &map,
        &node,
        json!([
            {"op": "replace", "id": second, "markdown": "Rewritten."},
            {"op": "insert_after", "id": block, "markdown": "Still lands."},
        ]),
    )
    .await;
    let (s, out) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/proposals/{delete}/accept"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    let (s, out) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/proposals/{both}/accept"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["applied"], json!(1), "{out}");
    let expected = json!([format!(
        "replace {second}: that block is no longer in the document"
    )]);
    assert_eq!(out["skipped"], expected, "{out}");
    assert_eq!(out["proposal"]["dropped"], expected, "{out}");

    let (_, read) = app
        .get(&app.admin, &format!("/v1/mindmaps/{map}/prose?node={node}"))
        .await;
    let md = read["markdown"].as_str().unwrap();
    assert!(
        md.contains("Still lands.") && !md.contains("Rewritten."),
        "{md}"
    );

    // A rejection is recorded in the history too.
    let later = propose(
        &app,
        &map,
        &node,
        json!([{"op": "replace", "id": block, "markdown": "Nope."}]),
    )
    .await;
    let (s, out) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/proposals/{later}/reject"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    let (_, trace) = app
        .get(&app.admin, &format!("/v1/mindmaps/{map}/trace?node={node}"))
        .await;
    assert!(
        trace["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "rejected"),
        "{trace}"
    );
}
