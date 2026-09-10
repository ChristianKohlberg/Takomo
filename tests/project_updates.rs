mod common;
use common::TestApp;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn idle_agent_claims_do_not_invalidate_document_lists() {
    let app = TestApp::spawn_without_sweeper().await;
    let runner = app.mint("agent:idle", &["agent:run"], Some(&["tp"]));
    // First authentication persists last_used_at, a real bookkeeping write.
    // Warm it before observing the empty claims themselves.
    let (status, _) = app
        .post(
            &runner,
            "/v1/agent-jobs/claim",
            json!({"service_id":"idle-worker"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, session) = app
        .post(&app.worker, "/v1/projects/tp/session", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let url = format!(
        "{}/v1/sync/project:tp?ticket={}",
        app.base.replace("http://", "ws://"),
        session["token"].as_str().unwrap()
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    assert!(matches!(socket.next().await, Some(Ok(Message::Text(_)))));
    let mut refreshes = 0;
    for _ in 0..3 {
        let (status, job) = app
            .post(
                &runner,
                "/v1/agent-jobs/claim",
                json!({"service_id":"idle-worker"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{job}");
        assert!(job["job"].is_null(), "{job}");
        if let Ok(Some(Ok(Message::Text(_)))) =
            tokio::time::timeout(Duration::from_millis(650), socket.next()).await
        {
            refreshes += 1;
        }
    }
    assert_eq!(
        refreshes, 0,
        "Three empty worker polls caused {refreshes} document refresh batches"
    );
    let (status, created) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Actual change"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(message, Message::Text(text) if text.contains("refresh")));
}

#[tokio::test]
async fn idle_maintenance_does_not_invalidate_project_lists() {
    // Includes the 250ms lease/OAuth/session sweeps and the real 5s search indexer.
    let app = TestApp::spawn().await;
    let (_, session) = app
        .post(&app.worker, "/v1/projects/tp/session", json!({}))
        .await;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "{}/v1/sync/project:tp?ticket={}",
        app.base.replace("http://", "ws://"),
        session["token"].as_str().unwrap()
    ))
    .await
    .unwrap();
    assert!(matches!(socket.next().await, Some(Ok(Message::Text(_)))));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let mut refreshes = 0;
    while let Ok(message) = tokio::time::timeout_at(deadline, socket.next()).await {
        match message {
            Some(Ok(Message::Text(_))) => refreshes += 1,
            other => panic!("unexpected socket closure: {other:?}"),
        }
    }
    assert_eq!(
        refreshes, 0,
        "Six idle seconds caused {refreshes} document refresh batches"
    );
}

#[tokio::test]
async fn project_socket_filters_topics_and_other_projects_but_resyncs_on_connect() {
    let app = TestApp::spawn_without_sweeper().await;
    let (status, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({"id":"other","name":"Other"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // Mint before subscribing; first use still updates token usage after subscription.
    let reader = app.mint("reader:scope", &["read"], Some(&["tp"]));
    let (_, session) = app
        .post(&app.worker, "/v1/projects/tp/session", json!({}))
        .await;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "{}/v1/sync/project:tp?ticket={}",
        app.base.replace("http://", "ws://"),
        session["token"].as_str().unwrap()
    ))
    .await
    .unwrap();
    let initial = socket.next().await.unwrap().unwrap();
    assert!(
        matches!(initial,Message::Text(text) if serde_json::from_str::<serde_json::Value>(&text).unwrap()==json!({"type":"refresh"}))
    );
    assert_eq!(app.get(&reader, "/v1/projects").await.0, StatusCode::OK);
    let (status, _) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"other","title":"Unrelated"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(
        tokio::time::timeout(Duration::from_millis(700), socket.next())
            .await
            .is_err(),
        "Unrelated document or token usage invalidated this project"
    );
    let (status, _) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Relevant"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let msg = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = msg else {
        panic!("expected refresh")
    };
    let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["type"], "refresh");
    let topics = payload["topics"].as_array().expect("specific topics");
    assert!(topics.contains(&json!("document")));
    assert!(!topics.contains(&json!("inbox")));
    assert!(!topics.contains(&json!("projects")));
}
