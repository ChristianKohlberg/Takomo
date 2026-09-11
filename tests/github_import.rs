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
    store
        .github_connect(
            123,
            "test-account",
            "https://github.com/settings/installations/123",
        )
        .unwrap();
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

#[test]
fn installation_settings_resolve_to_the_correct_account_and_reject_invalid_metadata() {
    use takomo::github::installation_management_url;
    assert_eq!(
        installation_management_url(&json!({"id":123,"account":{"type":"User","login":"person"}}))
            .unwrap(),
        "https://github.com/settings/installations/123"
    );
    assert_eq!(
        installation_management_url(
            &json!({"id":456,"account":{"type":"Organization","login":"team-name"}})
        )
        .unwrap(),
        "https://github.com/organizations/team-name/settings/installations/456"
    );
    assert!(installation_management_url(
        &json!({"id":456,"account":{"type":"Organization","login":"../attacker"}})
    )
    .is_err());
}

#[tokio::test]
async fn existing_preview_connections_gain_management_metadata_without_losing_project_links() {
    let app = TestApp::spawn().await;
    fixture(&app).await;
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute(
        "ALTER TABLE github_connections DROP COLUMN management_url",
        [],
    )
    .unwrap();
    let restored = app.open_store();
    let connections = restored.github_connections().unwrap();
    assert_eq!(connections[0]["id"], 123);
    assert_eq!(connections[0]["management_url"], "");
    assert_eq!(
        restored.project_repository("tp").unwrap().unwrap()["repository"],
        456
    );
}

#[tokio::test]
async fn extraction_is_visible_in_shared_queue_with_scoped_durable_usage() {
    let app = TestApp::spawn().await;
    let (_, id) = fixture(&app).await;
    let runner = app.mint("agent:runner", &["agent:run"], Some(&["tp"]));
    let outsider = app.mint("human:other", &["read"], Some(&["other"]));
    let job = claim(&app, &runner).await;
    let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":10,"total_tokens":130});
    let heartbeat = json!({"service_id":"worker","attempt_id":job["attempt_id"],"telemetry":{"usage":usage,"phase":"drafting","thread_id":"thread-one","turn_id":"turn-one"}});
    let path = format!("/v1/codebase-import-jobs/{id}/heartbeat");
    assert_eq!(
        app.post(&runner, &path, heartbeat.clone()).await.0,
        StatusCode::OK
    );
    assert_eq!(
        app.post(&runner, &path, heartbeat.clone()).await.0,
        StatusCode::OK
    );
    let (_, list) = app
        .get(&app.human, "/v1/agent-jobs?project=tp&status=running")
        .await;
    assert_eq!(list["counts"]["running"], 1);
    assert_eq!(list["items"][0]["kind"], "codebase_import");
    assert_eq!(list["items"][0]["telemetry"]["usage"], usage);
    let detail = format!("/v1/agent-jobs/{id}");
    assert_eq!(app.get(&outsider, &detail).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.get(&outsider, "/v1/agent-jobs").await.1["total"], 0);
    let mut stale = heartbeat.clone();
    stale["attempt_id"] = json!("stale");
    assert_eq!(
        app.post(&runner, &path, stale).await.0,
        StatusCode::CONFLICT
    );
    let mut invalid = heartbeat.clone();
    invalid["telemetry"]["usage"]["total_tokens"] = json!(-1);
    assert_eq!(
        app.post(&runner, &path, invalid).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut result = heartbeat;
    result["error"] = json!("Interrupted test run");
    assert_eq!(
        app.post(
            &runner,
            &format!("/v1/codebase-import-jobs/{id}/result"),
            result
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, saved) = app.get(&app.human, &detail).await;
    assert_eq!(saved["job"]["status"], "failed");
    assert_eq!(saved["job"]["telemetry"]["usage"], usage);
    assert!(saved["job"]["finished_at"].as_i64().is_some());
    assert_eq!(
        app.get(&app.human, "/v1/agent-jobs?status=running").await.1["total"],
        0
    );
}

#[tokio::test]
async fn unified_queue_limits_and_counts_both_sources_and_preserves_legacy_unknown_usage() {
    let app = TestApp::spawn().await;
    let (map, id) = fixture(&app).await;
    let db = rusqlite::Connection::open(app.db_path()).unwrap();
    db.execute("INSERT INTO agent_conversations(id,mindmap,node,project,created_at) VALUES('mixed-conversation',?1,'section','tp',1)",[map]).unwrap();
    db.execute("INSERT INTO agent_jobs(id,conversation_id,requested_by,request_id,prompt,snapshot,source_revision,status,created_at) VALUES('aj-old','mixed-conversation','human','req','prompt','snapshot','rev','completed',1)",[]).unwrap();
    let (_, list) = app
        .get(&app.human, "/v1/agent-jobs?project=tp&limit=1")
        .await;
    assert_eq!(list["total"], 2);
    assert_eq!(list["counts"]["completed"], 1);
    assert_eq!(list["counts"]["queued"], 1);
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["items"][0]["id"], id);
    assert!(list["items"][0]["telemetry"].is_null());
    for field in ["prompt", "snapshot", "response"] {
        assert!(list["items"][0].get(field).is_none());
    }
    let (_, filtered) = app
        .get(&app.human, "/v1/agent-jobs?project=tp&status=completed")
        .await;
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["items"][0]["id"], "aj-old");
    assert!(filtered["items"][0]["telemetry"].is_null());
}
