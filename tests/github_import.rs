mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
use takomo::auth::AuthCtx;
fn human() -> AuthCtx {
    AuthCtx {
        token_id: "test".into(),
        actor: "human:owner".into(),
        scopes: ["admin", "human", "write", "read"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        projects: None,
        rate_limit: 1000,
        user: None,
    }
}
async fn fixture(app: &TestApp) -> (String, String) {
    let (_, map) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Import"}),
        )
        .await;
    let map = map["mindmap"]["id"].as_str().unwrap().to_owned();
    let store = app.open_store();
    store.github_connect(123, "test-account").unwrap();
    let source = json!({"installation":123,"repository":456,"full_name":"test/repo","scope":{"include":["src"],"exclude":[]},"revision":"a".repeat(40),"limits":{"max_files":20,"max_source_bytes":100000,"max_tool_calls":12},"max_sections":3});
    store.set_project_repository("tp", &source).unwrap();
    let job = store
        .enqueue_codebase_import(&human(), "tp", &map, "request-1", &source)
        .unwrap();
    (map, job["id"].as_str().unwrap().to_owned())
}
async fn claim(app: &TestApp, token: &str) -> Value {
    let (status, result) = app
        .post(
            token,
            "/v1/codebase-import-jobs/claim",
            json!({"service_id":"worker"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    result["job"].clone()
}
#[tokio::test]
async fn github_settings_are_reserved_for_unrestricted_human_admins() {
    let app = TestApp::spawn().await;
    let scoped = app.mint(
        "human:scoped",
        &["read", "write", "admin", "human"],
        Some(&["tp"]),
    );
    for token in [&app.worker, &app.human, &scoped] {
        assert_eq!(
            app.get(token, "/v1/integrations/github").await.0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, body) = app.get(&app.admin, "/v1/integrations/github").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.to_string().contains("PRIVATE KEY"));
}
#[tokio::test]
async fn claimed_import_populates_document_and_map_and_deduplicates_results() {
    let app = TestApp::spawn().await;
    let (map, id) = fixture(&app).await;
    let outsider = app.mint("agent:outsider", &["agent:run"], Some(&["other"]));
    assert!(claim(&app, &outsider).await.is_null());
    let runner = app.mint("agent:runner", &["agent:run"], Some(&["tp"]));
    let job = claim(&app, &runner).await;
    assert_eq!(job["id"], id);
    let identity = json!({"service_id":"worker","attempt_id":job["attempt_id"]});
    assert_eq!(
        app.post(
            &runner,
            &format!("/v1/codebase-import-jobs/{id}/heartbeat"),
            identity.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    let draft = json!({"title":"Checkout","summary":"Selected source only.","gaps":[],"sections":[{"key":"orders","parent":null,"title":"Orders","notes":"Orders require items.","sources":[{"path":"src/order.rs","start_line":1,"end_line":2}]}]});
    let mut body = identity;
    body["draft"] = draft;
    let path = format!("/v1/codebase-import-jobs/{id}/result");
    let (status, result) = app.post(&runner, &path, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["reviewed"], false);
    assert_eq!(result["mindmap"], map);
    assert_eq!(app.post(&runner, &path, body).await.1, result);
    let (_, prose) = app
        .get(&app.human, &format!("/v1/mindmaps/{map}/prose"))
        .await;
    assert!(prose["markdown"]
        .as_str()
        .unwrap()
        .contains("Orders require items."));
    let (_, runs) = app
        .get(&app.human, "/v1/projects/tp/codebase-imports")
        .await;
    assert_eq!(runs["items"][0]["status"], "completed");
    // No re-run, even when retrying after successful publication made the map nonempty.
    let existing = app
        .open_store()
        .existing_codebase_import(&human(), "tp", &map, "request-1")
        .unwrap()
        .unwrap();
    assert_eq!(existing["id"], id);
}
#[tokio::test]
async fn disconnect_and_expiry_stop_work_without_automatic_reexecution() {
    let app = TestApp::spawn().await;
    let (_, id) = fixture(&app).await;
    let runner = app.mint("agent:runner", &["agent:run"], Some(&["tp"]));
    let job = claim(&app, &runner).await;
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute(
        "UPDATE codebase_import_jobs SET lease_expires_at=0 WHERE id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(
        app.post(
            &runner,
            &format!("/v1/codebase-import-jobs/{id}/heartbeat"),
            json!({"service_id":"worker","attempt_id":job["attempt_id"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(claim(&app, &runner).await.is_null());
    app.open_store().github_disconnect(123).unwrap();
    assert!(app.open_store().project_repository("tp").unwrap().is_none());
}
#[tokio::test]
async fn workers_cannot_choose_sources_or_write_outside_the_job_scope() {
    let app = TestApp::spawn().await;
    let (map, id) = fixture(&app).await;
    let runner = app.mint("agent:runner", &["agent:run"], Some(&["tp"]));
    let job = claim(&app, &runner).await;
    let body = json!({"service_id":"worker","attempt_id":job["attempt_id"],"draft":{"title":"Invalid","summary":"Invalid source.","gaps":[],"sections":[{"key":"bad","parent":null,"title":"Secret","notes":"Must be rejected.","sources":[{"path":"private/secret","start_line":1,"end_line":1}]}]}});
    assert_eq!(
        app.post(
            &runner,
            &format!("/v1/codebase-import-jobs/{id}/result"),
            body
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (_, view) = app.get(&app.human, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(view["nodes"], json!([]));
}
