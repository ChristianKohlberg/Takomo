mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
async fn fixture(app: &TestApp) -> (String, String) {
    let (s, m) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Review source"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{m}");
    let map = m["mindmap"]["id"].as_str().unwrap().to_owned();
    let (s, n) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"text":"Payments","notes":"Keep this prose."}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    (map, n["nodes"][0]["id"].as_str().unwrap().to_owned())
}
fn send(section: &str, id: &str) -> Value {
    json!({"request_id":id,"title":"Check payment behaviour","comments":[{"id":format!("comment-{id}"),"section_id":section,"anchor":{"quote":"Payments","start":{},"end":{}},"text":"What happens on retry?"}]})
}
async fn action(
    app: &TestApp,
    token: &str,
    v: &Value,
    name: &str,
    text: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = json!({"request_id":format!("{name}-{}",v["version"]),"version":v["version"],"action":name,"thread_id":v["thread_ids"][0]});
    if let Some(t) = text {
        req["text"] = json!(t)
    }
    app.post(
        token,
        &format!("/v1/document-reviews/{}/actions", v["id"].as_str().unwrap()),
        req,
    )
    .await
}
#[tokio::test]
async fn batch_send_shared_queue_reply_resolution_and_retry() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, section) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/reviews");
    let req = send(&section, "first");
    let (s, v) = app.post(&app.human, &path, req.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["snapshot"][0]["messages"][0]["text"],
        "What happens on retry?"
    );
    assert_eq!(v["needs_me"], true);
    let (s, retry) = app.post(&app.human, &path, req.clone()).await;
    assert_eq!(s, StatusCode::OK, "{retry}");
    assert_eq!(retry, v);
    let mut changed = req;
    changed["title"] = json!("Different");
    assert_eq!(
        app.post(&app.human, &path, changed).await.0,
        StatusCode::CONFLICT
    );
    let (s, page) = app
        .get(&app.human, "/v1/document-reviews?queue=shared&limit=1")
        .await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["id"], v["id"]);
    assert_eq!(
        action(&app, &app.human, &v, "close", None).await.0,
        StatusCode::CONFLICT
    );
    let (s, replied) = action(
        &app,
        &app.human,
        &v,
        "reply",
        Some("The charge is deduplicated."),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{replied}");
    assert_eq!(
        replied["snapshot"][0]["messages"].as_array().unwrap().len(),
        2
    );
    let (s, retry) = action(
        &app,
        &app.human,
        &v,
        "reply",
        Some("The charge is deduplicated."),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(retry, replied);
    let (s, resolved) = action(&app, &app.human, &replied, "resolve", None).await;
    assert_eq!(s, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["status"], "open");
    assert_eq!(resolved["snapshot"][0]["resolved"], true);
    let (s, closed) = action(&app, &app.human, &resolved, "close", None).await;
    assert_eq!(s, StatusCode::OK, "{closed}");
    assert_eq!(closed["status"], "closed");
    let (s, read) = app
        .get(
            &app.human,
            &format!("/v1/document-reviews/{}", v["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(read, closed);
    let (_, source) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert!(source.to_string().contains("Keep this prose."), "{source}");
    assert_eq!(
        app.get(&app.human, "/v1/document-reviews?limit=0").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
#[tokio::test]
async fn permissions_validation_and_transactional_rollback() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, section) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/reviews");
    let no_project = app.mint(
        "human:outsider",
        &["read", "human", "write"],
        Some(&["other"]),
    );
    let read_only = app.mint("human:reader", &["read", "human"], Some(&["tp"]));
    let req = send(&section, "safe");
    for token in [&app.worker, &no_project, &read_only] {
        assert_eq!(
            app.post(token, &path, req.clone()).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let mut invalid = req.clone();
    invalid["recipients"] = json!(["missing-person"]);
    let before = app.open_store().load_collab_updates(&map).unwrap();
    assert!(!app.post(&app.human, &path, invalid).await.0.is_success());
    assert_eq!(before, app.open_store().load_collab_updates(&map).unwrap());
    let (s, v) = app.post(&app.human, &path, req).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        app.get(
            &no_project,
            &format!("/v1/document-reviews/{}", v["id"].as_str().unwrap())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.get(&no_project, "/v1/document-reviews?queue=all")
            .await
            .1["total"],
        0
    );
    let mut stale = v.clone();
    stale["version"] = json!(99);
    assert_eq!(
        action(&app, &app.human, &stale, "reply", Some("Must roll back"))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let read = app
        .get(
            &app.human,
            &format!("/v1/document-reviews/{}", v["id"].as_str().unwrap()),
        )
        .await
        .1;
    assert_eq!(read["snapshot"][0]["messages"].as_array().unwrap().len(), 1);
    let mut wrong = send("removed", "wrong");
    wrong["comments"][0]["anchor"] = json!({});
    assert!(!app.post(&app.human, &path, wrong).await.0.is_success());
}
#[tokio::test]
async fn addressed_review_change_ownership_and_existing_thread_promotion() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, section) = fixture(&app).await;
    let (s, user) = app
        .post(
            &app.admin,
            "/v1/users",
            json!({"handle":"review-owner","name":"Owner","projects":["tp"]}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{user}");
    let uid = user["id"].as_str().unwrap();
    let owner = app.mint_as_user("human:owner", &["human", "write", "read"], uid);
    let path = format!("/v1/mindmaps/{map}/reviews");
    let mut req = send(&section, "change");
    req["kind"] = json!("change");
    req["recipients"] = json!([uid]);
    let (s, v) = app.post(&app.human, &path, req).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["needs_me"], false);
    assert_eq!(
        app.get(&owner, "/v1/document-reviews?queue=needs_me")
            .await
            .1["total"],
        1
    );
    assert_eq!(
        action(&app, &app.human, &v, "start", None).await.0,
        StatusCode::FORBIDDEN
    );
    let (s, started) = action(&app, &owner, &v, "start", None).await;
    assert_eq!(s, StatusCode::OK, "{started}");
    let (s, ready) = action(&app, &owner, &started, "ready", None).await;
    assert_eq!(s, StatusCode::OK, "{ready}");
    assert_eq!(
        action(&app, &owner, &ready, "close", None).await.0,
        StatusCode::FORBIDDEN
    );
    let (s, closed) = action(&app, &app.human, &ready, "close", None).await;
    assert_eq!(s, StatusCode::OK, "{closed}");
    assert_eq!(closed["status"], "closed");
    let req = json!({"request_id":"promote","kind":"question","title":"One follow-up","thread_ids":v["thread_ids"]});
    let (s, q) = app.post(&app.human, &path, req).await;
    assert_eq!(s, StatusCode::OK, "{q}");
    assert_eq!(q["thread_ids"], v["thread_ids"]);
    assert_eq!(q["snapshot"], v["snapshot"]);
    assert!(!action(&app, &owner, &q, "close", None).await.0.is_success());
    let (s, answered) = action(&app, &owner, &q, "close", Some("Here is the answer.")).await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["status"], "closed");
    let old = app
        .get(
            &app.human,
            &format!("/v1/document-reviews/{}", v["id"].as_str().unwrap()),
        )
        .await
        .1;
    assert_eq!(old["snapshot"][0]["messages"].as_array().unwrap().len(), 2);
}
#[tokio::test]
async fn simultaneous_sends_are_idempotent_and_pages_are_counted() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, section) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/reviews");
    let req = send(&section, "concurrent");
    let (a, b) = tokio::join!(
        app.post(&app.human, &path, req.clone()),
        app.post(&app.human, &path, req)
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(a.1["id"], b.1["id"]);
    assert_eq!(
        app.post(&app.human, &path, send(&section, "second"))
            .await
            .0,
        StatusCode::OK
    );
    let (_, page) = app
        .get(&app.human, "/v1/document-reviews?queue=mine&limit=1")
        .await;
    assert_eq!(page["total"], 2);
    assert_eq!(page["truncated"], true);
    let (_, last) = app
        .get(
            &app.human,
            "/v1/document-reviews?queue=mine&limit=1&offset=1",
        )
        .await;
    assert_eq!(last["total"], 2);
    assert_eq!(last["truncated"], false);
    assert_ne!(page["items"][0]["id"], last["items"][0]["id"]);
}

#[tokio::test]
async fn archive_guards_and_recipient_acknowledgement_are_separate_from_replies() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, section) = fixture(&app).await;
    let path = format!("/v1/mindmaps/{map}/reviews");
    let (_, v) = app.post(&app.human, &path, send(&section, "ack")).await;
    let (s, reply) = action(&app, &app.human, &v, "reply", Some("First thought")).await;
    assert_eq!(s, StatusCode::OK, "{reply}");
    assert_eq!(reply["needs_me"], true);
    assert_eq!(reply["responded"], false);
    let (s, done) = action(&app, &app.human, &reply, "reviewed", None).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["needs_me"], false);
    assert_eq!(done["responded"], true);
    assert_eq!(done["status"], "open");
    let (s, archived) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{archived}");
    let before = app.open_store().load_collab_updates(&map).unwrap();
    assert!(!app
        .post(&app.human, &path, send(&section, "archived"))
        .await
        .0
        .is_success());
    assert!(
        !action(&app, &app.human, &done, "reply", Some("Cannot write"))
            .await
            .0
            .is_success()
    );
    assert_eq!(before, app.open_store().load_collab_updates(&map).unwrap());
    assert_eq!(
        app.get(
            &app.human,
            &format!("/v1/document-reviews/{}", v["id"].as_str().unwrap())
        )
        .await
        .0,
        StatusCode::OK
    );
}
