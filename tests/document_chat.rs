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
