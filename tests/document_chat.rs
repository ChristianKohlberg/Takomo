mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn fixture(app: &TestApp) -> (String, String, String) {
    let (status, map) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Checkout"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{map}");
    let map = map["mindmap"]["id"].as_str().unwrap().to_owned();
    let (status, first) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"text":"Payment","notes":"Charge once."}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let first = first["nodes"][0]["id"].as_str().unwrap().to_owned();
    let (status, second) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"text":"Retry","notes":"Retry is safe.","parent":first}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    (
        map,
        first,
        second["nodes"][0]["id"].as_str().unwrap().to_owned(),
    )
}
fn message(id: &str, ids: Vec<&str>, whole: bool) -> Value {
    json!({"message":"Draft checks and explain gaps","request_id":id,"action":"draft_tests","section_ids":ids,"whole_document":whole})
}
async fn claim(app: &TestApp, token: &str, service: &str, kinds: Option<Vec<&str>>) -> Value {
    let mut body = json!({"service_id":service});
    if let Some(kinds) = kinds {
        body["supported_kinds"] = json!(kinds);
    }
    let (status, value) = app.post(token, "/v1/agent-jobs/claim", body).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value["job"].clone()
}
async fn complete(app: &TestApp, token: &str, job: &Value, service: &str) {
    let (status,result)=app.post(token,&format!("/v1/agent-jobs/{}/result",job["id"].as_str().unwrap()),json!({"service_id":service,"attempt_id":job["attempt_id"],"status":"completed","thread_id":"document-thread","turn_id":job["id"],"message":"Draft: repeated payment submits create one charge. No check was saved or executed."})).await;
    assert_eq!(status, StatusCode::OK, "{result}");
}
#[tokio::test]
async fn document_turns_keep_context_history_thread_affinity_and_readonly_results() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, first, second) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/conversation");
    let send = format!("{path}/messages");
    let runner = app.mint("agent:document", &["agent:run"], Some(&["tp"]));
    assert!(app.get(&app.human, &path).await.1["conversation"].is_null());
    // A fresh server-side edit is captured; caller cannot supply snapshot text.
    assert_eq!(
        app.patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{first}"),
            json!({"notes":"Charge once even after retries."})
        )
        .await
        .0,
        StatusCode::OK
    );
    let before = app.open_store().load_collab_updates(&map).unwrap();
    let req = message("one", vec![&second, &first], false);
    let (status, queued) = app.post(&app.human, &send, req.clone()).await;
    assert_eq!(status, StatusCode::OK, "{queued}");
    assert_eq!(queued["jobs"][0]["section_ids"], json!([first, second]));
    assert_eq!(queued["jobs"][0]["section_count"], 2);
    assert!(
        claim(&app, &runner, "old", None).await.is_null(),
        "old worker consumed unsupported document action"
    );
    let job = claim(&app, &runner, "capable", Some(vec!["document_chat"])).await;
    assert_eq!(job["kind"], "document_chat");
    let snapshot: Value = serde_json::from_str(job["snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(
        snapshot["sections"][0]["notes"],
        "Charge once even after retries."
    );
    assert_eq!(snapshot["sections"][1]["parent_id"], first);
    assert_eq!(snapshot["action"], "draft_tests");
    assert_eq!(
        app.post(&app.human, &send, message("busy", vec![&first], false))
            .await
            .0,
        StatusCode::CONFLICT
    );
    complete(&app, &runner, &job, "capable").await;
    assert_eq!(
        app.open_store().load_collab_updates(&map).unwrap(),
        before,
        "draft response changed document"
    );
    assert_eq!(
        app.open_store().document_conversation(&map).unwrap(),
        app.get(&app.human, &path).await.1,
        "history was not durable"
    );
    assert_eq!(
        app.post(&app.human, &send, message("two", vec![], true))
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        claim(&app, &runner, "other", Some(vec!["document_chat"]))
            .await
            .is_null(),
        "session changed workers"
    );
    let next = claim(&app, &runner, "capable", Some(vec!["document_chat"])).await;
    assert_eq!(next["thread_id"], "document-thread");
    complete(&app, &runner, &next, "capable").await;
    let view = app.get(&app.human, &path).await.1;
    assert_eq!(view["messages"].as_array().unwrap().len(), 4);
    assert_eq!(view["jobs"][1]["whole_document"], true);
    let inspect = app
        .get(
            &app.human,
            &format!("/v1/agent-jobs/{}", job["id"].as_str().unwrap()),
        )
        .await
        .1;
    assert_eq!(inspect["job"]["kind"], "document_chat");
    assert_eq!(inspect["job"]["section_title"], "Checkout");
    assert!(inspect["job"].get("token_id").is_none());
}
#[tokio::test]
async fn retries_recover_original_turn_after_edits_or_deleted_sections_but_reject_changed_intent() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, first, second) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/conversation/messages");
    let req = message("retry", vec![&second], false);
    let original = app.post(&app.human, &path, req.clone()).await.1;
    assert_eq!(
        app.patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{second}"),
            json!({"notes":"Changed after request"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(app.post(&app.human, &path, req.clone()).await.1, original);
    assert!(app
        .delete(&app.worker, &format!("/v1/mindmaps/{map}/nodes/{second}"))
        .await
        .0
        .is_success());
    let (status, retried) = app.post(&app.human, &path, req.clone()).await;
    assert_eq!(status, StatusCode::OK, "{retried}");
    assert_eq!(retried, original);
    assert_eq!(
        app.post(&app.human, &path, message("retry", vec![&first], false))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut changed = req;
    changed["action"] = json!("grill");
    assert_eq!(
        app.post(&app.human, &path, changed).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.post(&app.human, &path, message("new", vec![&second], false))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn document_scope_auth_bounds_and_capabilities_are_enforced() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, first, _) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/conversation");
    let send = format!("{path}/messages");
    let reader = app.mint("human:reader", &["read"], Some(&["tp"]));
    let outside = app.mint(
        "human:outside",
        &["read", "write", "human"],
        Some(&["elsewhere"]),
    );
    assert_eq!(app.get(&reader, &path).await.0, StatusCode::OK);
    for token in [&reader, &app.worker, &outside] {
        assert_eq!(
            app.post(token, &send, message("no", vec![&first], false))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(app.get(&outside, &path).await.0, StatusCode::FORBIDDEN);
    for req in [
        message("empty", vec![], false),
        message("ambiguous", vec![&first], true),
        message("duplicate", vec![&first, &first], false),
    ] {
        assert_eq!(
            app.post(&app.human, &send, req).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let mut injected = message("snapshot", vec![&first], false);
    injected["snapshot"] = json!("caller supplied source");
    assert_eq!(
        app.post(&app.human, &send, injected).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let runner = app.mint("agent:runner", &["agent:run"], Some(&["tp"]));
    assert_eq!(
        app.post(
            &runner,
            "/v1/agent-jobs/claim",
            json!({"service_id":"unknown","supported_kinds":["unsupported"]})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Each section stays below the existing 8000-character notes limit, while
    // the combined plain + structured snapshot exceeds our 100 KB bound.
    for index in 0..8 {
        let (status, created) = app
            .post(
                &app.worker,
                &format!("/v1/mindmaps/{map}/nodes"),
                json!({"text":format!("Context {index}"),"notes":"x".repeat(7000)}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
    }
    assert_eq!(
        app.post(&app.human, &send, message("large", vec![], true))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(app.get(&reader, &path).await.1["conversation"].is_null());
}

#[tokio::test]
async fn snapshots_capture_live_crdt_table_cells_and_code_language_without_rewriting() {
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    use yrs::{
        encoding::write::Write, updates::decoder::Decode, Doc, Transact, Update, Xml,
        XmlElementPrelim, XmlFragment, XmlTextPrelim,
    };
    let app = TestApp::spawn_without_sweeper().await;
    let (map, first, _) = fixture(&app).await;
    let doc = Doc::new();
    for update in app.open_store().load_collab_updates(&map).unwrap() {
        doc.transact_mut()
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    let frag = takomo::store::mindmapdoc::read_section_prose(&doc, &first).unwrap();
    let update = {
        let mut txn = doc.transact_mut();
        let len = frag.len(&txn);
        frag.remove_range(&mut txn, 0, len);
        let table = frag.push_back(&mut txn, XmlElementPrelim::empty("table"));
        let row = table.push_back(&mut txn, XmlElementPrelim::empty("tableRow"));
        for text in ["Input", "Expected"] {
            let cell = row.push_back(&mut txn, XmlElementPrelim::empty("tableCell"));
            let p = cell.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            p.push_back(&mut txn, XmlTextPrelim::new(text));
        }
        let code = frag.push_back(&mut txn, XmlElementPrelim::empty("codeBlock"));
        code.insert_attribute(&mut txn, "language", "d2");
        code.push_back(&mut txn, XmlTextPrelim::new("live -> captured"));
        txn.encode_update_v1()
    };
    let (status, session) = app
        .post(
            &app.human,
            &format!("/v1/mindmaps/{map}/session"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let url = format!(
        "{}/v1/docsync/{map}?ticket={}",
        app.base.replacen("http://", "ws://", 1),
        session["token"].as_str().unwrap()
    );
    let (mut peer, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut frame = vec![];
    frame.write_var(0u64);
    frame.write_var(2u64);
    frame.write_buf(&update);
    peer.send(Message::Binary(frame.into())).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let view = app.get(&app.human, &format!("/v1/mindmaps/{map}")).await.1;
            if view["nodes"].as_array().unwrap().iter().any(|n| {
                n["notes"]
                    .as_str()
                    .is_some_and(|s| s.contains("live -> captured"))
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let (status, queued) = app
        .post(
            &app.human,
            &format!("/v1/mindmaps/{map}/conversation/messages"),
            message("rich", vec![&first], false),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{queued}");
    let runner = app.mint("agent:rich", &["agent:run"], Some(&["tp"]));
    let job = claim(&app, &runner, "rich", Some(vec!["document_chat"])).await;
    let snapshot: Value = serde_json::from_str(job["snapshot"].as_str().unwrap()).unwrap();
    let xml = snapshot["sections"][0]["prose_xml"].as_str().unwrap();
    for part in [
        "tableRow",
        "tableCell",
        "Input",
        "Expected",
        "codeBlock",
        "language=\"d2\"",
        "live -> captured",
    ] {
        assert!(xml.contains(part), "missing {part}: {xml}");
    }
    assert!(snapshot["sections"][0]["notes"]
        .as_str()
        .unwrap()
        .contains("live -> captured"));
}

fn workspace(id: &str, context: Value) -> Value {
    json!({"message":"Review this requirement","request_id":id,"action":"grill","context":context})
}
fn evidence_for(job: &Value, ids: &[&str], total: usize) -> Value {
    let snapshot: Value = serde_json::from_str(job["snapshot"].as_str().unwrap()).unwrap();
    json!({"document":{"sources":ids.iter().map(|id|{
        let section=snapshot["sections"].as_array().unwrap().iter().find(|s|s["id"]==*id).unwrap();
        json!({"section_id":id,"version":section["version"]})
    }).collect::<Vec<_>>(),"coverage":{"read_section_ids":ids,"total_sections":total,"complete":ids.len()==total}}})
}
#[tokio::test]
async fn workspace_pins_quotes_permissions_and_default_context() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, first, second) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/conversation");
    let send = format!("{path}/messages");
    let reader = app.mint("reader", &["read"], Some(&["tp"]));
    let foreign = app.mint("other", &["read", "write", "human"], Some(&["other"]));
    for token in [&reader, &app.worker, &foreign] {
        assert_eq!(
            app.patch(token, &path, json!({"pinned_section_ids":[first]}))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            app.post(token, &send, workspace("denied", json!({})))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, pins) = app
        .patch(&app.human, &path, json!({"pinned_section_ids":[first]}))
        .await;
    assert_eq!(status, StatusCode::OK, "{pins}");
    assert!(pins["conversation"].is_null());
    assert_eq!(
        app.get(&reader, &path).await.1["pinned_section_ids"],
        json!([first])
    );
    assert_eq!(
        app.open_store().document_conversation(&map).unwrap()["pinned_section_ids"],
        json!([first])
    );
    assert_eq!(
        app.patch(&app.human, &path, json!({"pinned_section_ids":["missing"]}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let mut mixed = workspace("mixed", json!({}));
    mixed["whole_document"] = json!(true);
    assert_eq!(
        app.post(&app.human, &send, mixed).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let forged = workspace(
        "quote",
        json!({"mode":"selected","section_ids":[first],"quote":{"section_id":first,"text":"Invented requirement"}}),
    );
    assert_eq!(
        app.post(&app.human, &send, forged).await.0,
        StatusCode::CONFLICT
    );
    let context = json!({"mode":"selected","section_ids":[first],"pinned_section_ids":[second],"quote":{"section_id":first,"text":"Charge  once."}});
    let req = workspace("good", context.clone());
    let (status, queued) = app.post(&app.human, &send, req.clone()).await;
    assert_eq!(status, StatusCode::OK, "{queued}");
    assert_eq!(
        queued["jobs"][0]["context"]["quote"]["text"],
        "Charge  once."
    );
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{first}"),
        json!({"notes":"New wording."}),
    )
    .await;
    assert_eq!(
        app.post(&app.human, &send, req).await.1,
        queued,
        "retry revalidated stale quote"
    );
    let mut changed_mode = workspace("good", context.clone());
    changed_mode["context"]["mode"] = json!("automatic");
    changed_mode["context"]["section_ids"] = json!([]);
    changed_mode["context"]["quote"] = Value::Null;
    assert_eq!(
        app.post(&app.human, &send, changed_mode).await.0,
        StatusCode::CONFLICT
    );
    let mut changed_quote = workspace("good", context.clone());
    changed_quote["context"]["quote"]["text"] = json!("Charge once");
    assert_eq!(
        app.post(&app.human, &send, changed_quote).await.0,
        StatusCode::CONFLICT
    );
    // A retry is resolved before validating whether its old quote section still exists.
    let deleted = app
        .delete(&app.worker, &format!("/v1/mindmaps/{map}/nodes/{first}"))
        .await;
    assert_eq!(deleted.0, StatusCode::OK);
    assert_eq!(
        app.post(&app.human, &send, workspace("good", context.clone()))
            .await
            .1,
        queued
    );
    let mut changed = workspace("good", context);
    changed["context"]["pinned_section_ids"] = json!([]);
    assert_eq!(
        app.post(&app.human, &send, changed).await.0,
        StatusCode::CONFLICT
    );
    let runner = app.mint("runner", &["agent:run"], Some(&["tp"]));
    assert!(claim(&app, &runner, "old", Some(vec!["document_chat"]))
        .await
        .is_null());
    let job = claim(&app, &runner, "workspace", Some(vec!["document_workspace"])).await;
    assert_eq!(job["kind"], "document_workspace");
    assert_eq!(job["migrate_thread"], false);
    let result = json!({"service_id":"workspace","attempt_id":job["attempt_id"],"status":"completed","thread_id":"workspace-thread","turn_id":"one","message":"Read both sources.","evidence":evidence_for(&job,&[&first,&second],2)});
    assert_eq!(
        app.post(
            &runner,
            &format!("/v1/agent-jobs/{}/result", job["id"].as_str().unwrap()),
            result
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, automatic) = app
        .post(
            &app.human,
            &send,
            json!({"message":"What is ambiguous?","request_id":"default","action":"discuss"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{automatic}");
    assert_eq!(automatic["jobs"][1]["context"]["mode"], "automatic");
}

#[tokio::test]
async fn workspace_evidence_cannot_forge_sources_scope_versions_or_coverage() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, first, second) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/conversation");
    let (status, queued) = app
        .post(
            &app.human,
            &format!("{path}/messages"),
            workspace("one", json!({"mode":"selected","section_ids":[first]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{queued}");
    let runner = app.mint("runner", &["agent:run"], Some(&["tp"]));
    let job = claim(&app, &runner, "workspace", Some(vec!["document_workspace"])).await;
    let url = format!("/v1/agent-jobs/{}/result", job["id"].as_str().unwrap());
    let base = json!({"service_id":"workspace","attempt_id":job["attempt_id"],"status":"completed","thread_id":"workspace-thread","turn_id":"one","message":"[Payment](takomo-section:test)"});
    for bad in [
        Value::Null,
        evidence_for(&job, &[&second], 1),
        {
            let mut e = evidence_for(&job, &[&first], 1);
            e["document"]["sources"][0]["version"] = json!("forged");
            e
        },
        {
            let mut e = evidence_for(&job, &[&first], 1);
            e["document"]["coverage"]["total_sections"] = json!(2);
            e
        },
    ] {
        let mut result = base.clone();
        result["evidence"] = bad;
        assert_eq!(
            app.post(&runner, &url, result).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    app.patch(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes/{first}"),
        json!({"text":"Renamed","notes":"Changed requirement."}),
    )
    .await;
    let mut result = base;
    result["evidence"] = evidence_for(&job, &[&first], 1);
    let (status, saved) = app.post(&runner, &url, result).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let history = app.get(&app.human, &path).await.1;
    assert_eq!(history["jobs"][0]["sources"][0]["title"], "Payment");
    assert_eq!(history["jobs"][0]["coverage"]["complete"], true);
    assert_eq!(history["jobs"][0]["section_count"], 1);
}

#[tokio::test]
async fn workspace_migrates_legacy_thread_once_and_retains_history_and_affinity() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, first, _) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/conversation");
    let runner = app.mint("runner", &["agent:run"], Some(&["tp"]));
    app.post(
        &app.human,
        &format!("{path}/messages"),
        message("old", vec![&first], false),
    )
    .await;
    let old = claim(&app, &runner, "stable", Some(vec!["document_chat"])).await;
    complete(&app, &runner, &old, "stable").await;
    app.post(
        &app.human,
        &format!("{path}/messages"),
        workspace("new", json!({"mode":"automatic"})),
    )
    .await;
    assert!(
        claim(&app, &runner, "other", Some(vec!["document_workspace"]))
            .await
            .is_null()
    );
    let job = claim(&app, &runner, "stable", Some(vec!["document_workspace"])).await;
    assert_eq!(job["migrate_thread"], true);
    assert_eq!(job["thread_id"], "document-thread");
    let heartbeat = format!("/v1/agent-jobs/{}/heartbeat", job["id"].as_str().unwrap());
    let mut hb =
        json!({"service_id":"stable","attempt_id":job["attempt_id"],"thread_id":"new-thread"});
    assert_eq!(
        app.post(&runner, &heartbeat, hb.clone()).await.0,
        StatusCode::CONFLICT
    );
    let old_heartbeat =
        json!({"service_id":"stable","attempt_id":job["attempt_id"],"thread_id":"document-thread"});
    assert_eq!(
        app.post(&runner, &heartbeat, old_heartbeat).await.0,
        StatusCode::CONFLICT,
        "legacy heartbeat incorrectly marked old thread as tool-enabled"
    );
    let migration = json!({"previous_thread_id":"document-thread","new_thread_id":"new-thread","retained_turns":1,"omitted_turns":0});
    hb["evidence"] = json!({"document_migration":migration});
    let (status, value) = app.post(&runner, &heartbeat, hb).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let mut evidence = evidence_for(&job, &[&first], 2);
    evidence["document_migration"] = migration;
    let result = json!({"service_id":"stable","attempt_id":job["attempt_id"],"thread_id":"new-thread","turn_id":"next","status":"completed","message":"Partial review; one source remains unread.","evidence":evidence});
    let (status, value) = app
        .post(
            &runner,
            &format!("/v1/agent-jobs/{}/result", job["id"].as_str().unwrap()),
            result,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let history = app.get(&app.human, &path).await.1;
    assert_eq!(history["messages"].as_array().unwrap().len(), 4);
    assert_eq!(history["jobs"][1]["migration"]["retained_turns"], 1);
    app.post(
        &app.human,
        &format!("{path}/messages"),
        workspace("third", json!({"mode":"whole_document"})),
    )
    .await;
    let next = claim(&app, &runner, "stable", Some(vec!["document_workspace"])).await;
    assert_eq!(next["migrate_thread"], false);
    assert_eq!(next["thread_id"], "new-thread");
}
