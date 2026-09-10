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

#[tokio::test]
async fn awareness_echo_keeps_a_single_peer_alive_without_persisting_presence() {
    use futures::SinkExt;
    let app = TestApp::spawn_without_sweeper().await;
    let (_, map) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Presence"}),
        )
        .await;
    let id = map["mindmap"]["id"].as_str().unwrap();
    let (_, session) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{id}/session"),
            json!({}),
        )
        .await;
    let url = format!(
        "{}/v1/sync/{id}?ticket={}",
        app.base.replace("http://", "ws://"),
        session["token"].as_str().unwrap()
    );
    let (mut sender, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let (_, session) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{id}/session"),
            json!({}),
        )
        .await;
    let (mut peer, _) = tokio_tungstenite::connect_async(format!(
        "{}/v1/sync/{id}?ticket={}",
        app.base.replace("http://", "ws://"),
        session["token"].as_str().unwrap()
    ))
    .await
    .unwrap();
    // A valid awareness payload: one client, clock one, empty JSON state.
    let frame = vec![1, 6, 1, 7, 1, 2, b'{', b'}'];
    let count = || {
        rusqlite::Connection::open(app.db_path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM crdt_updates WHERE object_kind='mindmap' AND object_id=?1",
                [id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    let before = count();
    sender
        .send(Message::Binary(frame.clone().into()))
        .await
        .unwrap();
    for socket in [&mut sender, &mut peer] {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match socket
                    .next()
                    .await
                    .expect("socket open")
                    .expect("valid frame")
                {
                    Message::Binary(bytes) if bytes.as_ref() == frame.as_slice() => break,
                    Message::Close(_) => panic!("socket closed before awareness echo"),
                    _ => {}
                }
            }
        })
        .await
        .expect("sender echo and peer relay");
    }
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(count(), before);
}

#[tokio::test]
async fn writable_reconnect_replays_do_not_append_crdt_rows() {
    use futures::SinkExt;
    use yrs::encoding::write::Write;
    let app = TestApp::spawn_without_sweeper().await;
    let (_, map) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Reconnect"}),
        )
        .await;
    let id = map["mindmap"]["id"].as_str().unwrap();
    let (_, session) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{id}/session"),
            json!({}),
        )
        .await;
    let url = format!(
        "{}/v1/sync/{id}?ticket={}",
        app.base.replace("http://", "ws://"),
        session["token"].as_str().unwrap()
    );
    let stored = app.open_store().load_collab_updates(id).unwrap();
    let full = yrs::merge_updates_v1(&stored).unwrap();
    let mut frame = vec![0, 1]; // Sync step 2: replay the client's already-synced state.
    frame.write_var(full.len() as u64);
    frame.extend(full);
    let seq = || {
        rusqlite::Connection::open(app.db_path())
            .unwrap()
            .query_row(
                "SELECT coalesce(max(seq),0) FROM crdt_updates WHERE object_id=?1",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    let before = seq();
    for sequence in 1..=3u8 {
        let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        socket
            .send(Message::Binary(frame.clone().into()))
            .await
            .unwrap();
        socket
            .send(Message::Binary(vec![4, sequence].into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let message = socket
                    .next()
                    .await
                    .expect("socket open")
                    .expect("valid frame");
                if let Message::Binary(bytes) = message {
                    if bytes.as_ref() == [4, sequence, 1] {
                        break;
                    }
                }
            }
        })
        .await
        .expect("durability barrier confirms preceding replay handled");
        assert_eq!(
            seq(),
            before,
            "reconnecting must not persist a duplicate update"
        );
        socket.close(None).await.unwrap();
    }
}

#[tokio::test]
async fn lexical_projection_completion_emits_search_only_without_embeddings() {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, map) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Index"}),
        )
        .await;
    let id = map["mindmap"]["id"].as_str().unwrap();
    assert_eq!(
        app.get(&app.worker, &format!("/v1/mindmaps/{id}/search/status"))
            .await
            .0,
        StatusCode::OK
    );
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
    socket.next().await.unwrap().unwrap();
    let (status, result) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{id}/nodes"),
            json!({"nodes":[{"text":"Fresh section"}]}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    let dirty = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(dirty,Message::Text(text) if text.contains("search")),
        "dirty status must notify"
    );
    let (status, result) = app
        .get(&app.worker, &format!("/v1/mindmaps/{id}/search/status"))
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["configured"], false);
    assert_eq!(result["projection"], "current");
    let completed = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = completed else {
        panic!("refresh frame")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        json!({"type":"refresh","topics":["search"]})
    );
    app.get(&app.worker, &format!("/v1/mindmaps/{id}/search/status"))
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(600), socket.next())
            .await
            .is_err(),
        "clean status reads stay quiet"
    );
}
