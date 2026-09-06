use reqwest::StatusCode;
use rusqlite::{params, Connection};
use serde_json::json;
mod common;
use common::TestApp;

#[tokio::test]
async fn epic_activity_includes_the_container_and_unclaimed_descendants() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Epic", "epic", None).await;
    let child = app.create_typed("Child", "task", Some(&epic)).await;
    let empty = app.create_typed("No tasks", "epic", None).await;
    let db = Connection::open(app.db_path()).unwrap();
    for (id, at) in [
        (&epic, 1_700_000_000_000_i64),
        (&child, 1_700_000_020_000),
        (&empty, 1_700_000_010_000),
    ] {
        db.execute(
            "UPDATE tickets SET updated_at = ?2 WHERE id = ?1",
            params![id, at],
        )
        .unwrap();
    }
    let (status, body) = app.get(&app.worker, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK);
    let epics = body["epics"].as_array().unwrap();
    let row = epics.iter().find(|e| e["id"] == epic).unwrap();
    assert_eq!(row["claim"], json!(null));
    assert_eq!(row["last_activity_at"], takomo::ids::iso(1_700_000_020_000));
    let row = epics.iter().find(|e| e["id"] == empty).unwrap();
    assert_eq!(row["last_activity_at"], takomo::ids::iso(1_700_000_010_000));
    db.execute(
        "UPDATE tickets SET updated_at = ?2 WHERE id = ?1",
        params![epic, 1_700_000_030_000_i64],
    )
    .unwrap();
    let (_, body) = app.get(&app.worker, "/v1/projects/tp/roadmap").await;
    let row = body["epics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == epic)
        .unwrap();
    assert_eq!(row["last_activity_at"], takomo::ids::iso(1_700_000_030_000));
}

#[tokio::test]
async fn epic_own_questions_are_separate_from_descendant_tasks() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Question on epic", "epic", None).await;
    for title in ["Which direction?", "Which material?"] {
        let (status, body) = app
            .post(
                &app.admin,
                "/v1/questions",
                json!({
                    "ticket": epic, "mode": "advisory", "kind": "clarify", "title": title
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let (status, body) = app.get(&app.worker, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK);
    let row = &body["epics"][0];
    assert_eq!(row["own_open_questions"], 2);
    assert_eq!(row["awaiting_answer"], 0);
    assert_eq!(row["total"], 0);
}
