mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::json;
const POLL: &str = "/v1/agent-services/codex/poll";
const LIST: &str = "/v1/integrations/codex";
#[tokio::test]
async fn connection_control_is_admin_only_and_reports_are_bound_to_worker_tokens() {
    let app = TestApp::spawn().await;
    let runner = app.mint("agent:one", &["agent:run"], Some(&["tp"]));
    let other = app.mint("agent:two", &["agent:run"], Some(&["tp"]));
    let scoped = app.mint(
        "human:scoped",
        &["human", "admin", "read", "write"],
        Some(&["tp"]),
    );
    let (status, worker) = app
        .post(
            &runner,
            POLL,
            json!({"service_id":"worker","report":{"status":"disconnected"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{worker}");
    let path = format!("{LIST}/{}", worker["id"].as_str().unwrap());
    for token in [&runner, &scoped, &app.human] {
        assert_eq!(app.get(token, LIST).await.0, StatusCode::FORBIDDEN);
        assert_eq!(
            app.post(token, &path, json!({"action":"login","request_id":"req"}))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        app.post(
            &app.admin,
            &path,
            json!({"action":"login","request_id":"req"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, foreign) = app
        .post(
            &other,
            POLL,
            json!({"service_id":"worker","command_id":"req","report":{"status":"connected"}}),
        )
        .await;
    assert_ne!(foreign["id"], worker["id"]);
    assert!(foreign["action"].is_null());
    let (_, own) = app
        .post(&runner, POLL, json!({"service_id":"worker"}))
        .await;
    assert_eq!(own["action"], "login");
    assert_eq!(own["projects"], json!(["tp"]));
    let report = json!({"service_id":"worker","command_id":"req","report":{"status":"login_pending","device":{"verification_url":"https://auth.openai.com/codex/device","user_code":"ABCD-1234"}}});
    assert_eq!(app.post(&runner, POLL, report).await.0, StatusCode::OK);
    assert_eq!(
        app.post(
            &app.admin,
            &path,
            json!({"action":"cancel","request_id":"cancel"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, late) = app
        .post(
            &runner,
            POLL,
            json!({"service_id":"worker","command_id":"req","report":{"status":"connected"}}),
        )
        .await;
    assert_eq!(late["action"], "cancel");
    assert_eq!(late["command_id"], "cancel");
    assert!(late["report"]["device"].is_null());
    let (_, done) = app
        .post(
            &runner,
            POLL,
            json!({"service_id":"worker","command_id":"cancel","report":{"status":"disconnected"}}),
        )
        .await;
    assert!(done["action"].is_null());
    let raw = app.get(&app.admin, LIST).await.1.to_string();
    assert!(!raw.contains(&runner));
    assert!(!raw.contains("token_id"));
}
#[tokio::test]
async fn connection_reports_reject_secrets_urls_and_expired_commands() {
    let app = TestApp::spawn().await;
    let runner = app.mint("agent:one", &["agent:run"], Some(&["tp"]));
    for (report, expected) in [
        (
            json!({"status":"connected","access_token":"secret"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"status":"login_pending","device":{"verification_url":"https://attacker.invalid","user_code":"ABCD"}}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        assert_eq!(
            app.post(
                &runner,
                POLL,
                json!({"service_id":"worker","report":report})
            )
            .await
            .0,
            expected
        );
    }
    let (_, worker) = app
        .post(&runner, POLL, json!({"service_id":"worker"}))
        .await;
    let path = format!("{LIST}/{}", worker["id"].as_str().unwrap());
    app.post(
        &app.admin,
        &path,
        json!({"action":"login","request_id":"expired"}),
    )
    .await;
    let db = rusqlite::Connection::open(app.db_path()).unwrap();
    db.execute("UPDATE codex_connections SET expires_at=0", [])
        .unwrap();
    let (_, expired) = app
        .post(
            &runner,
            POLL,
            json!({"service_id":"worker","command_id":"expired","report":{"status":"connected"}}),
        )
        .await;
    assert!(expired["action"].is_null());
    assert_eq!(expired["report"]["status"], "error");
}
