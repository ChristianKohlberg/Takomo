mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn reading_migrated_prose_is_not_a_write_for_writers_or_readers() {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, map) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Read feedback"}),
        )
        .await;
    let map = map["mindmap"]["id"].as_str().unwrap();
    let path = format!("/v1/mindmaps/{map}");
    let (status, nodes) = app
        .post(
            &app.worker,
            &format!("{path}/nodes"),
            json!({"text":"Section","notes":"Original text"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{nodes}");
    let node = nodes["nodes"][0]["id"].as_str().unwrap();
    // Replacing text leaves CRDT tombstones. A state-vector diff can replay
    // that historical delete set even when the next operation changes nothing.
    let (status, patched) = app
        .patch(
            &app.worker,
            &format!("{path}/nodes/{node}"),
            json!({"notes":"Current text"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let reader = app.mint("reader", &["read"], Some(&["tp"]));
    let store = app.open_store();
    let before = store.max_collab_seq(map).unwrap();
    for token in [&reader, &app.worker] {
        let (status, detail) = app.get(token, &path).await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert_eq!(detail["nodes"][0]["notes"], "Current text");
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(
            store.max_collab_seq(map).unwrap(),
            before,
            "GET with {} scope appended a CRDT update",
            if token == &reader { "read" } else { "write" }
        );
    }
}
