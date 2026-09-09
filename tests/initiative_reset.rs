mod common;

use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn seed(app: &TestApp, project: &str) -> Value {
    let (status, initiative) = app.post(&app.admin, "/v1/initiatives", json!({
        "project": project, "title": "Keep this identity", "summary": "Clear this",
        "status": "parked", "labels": ["important"], "metadata": {"path": ["folder"], "custom": "keep"}
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{initiative}");
    let id = initiative["id"].as_str().unwrap();
    for kind in ["document", "thread", "view", "proposal"] {
        let (status, entry) = app
            .post(
                &app.admin,
                &format!("/v1/initiatives/{id}/entries"),
                json!({
                    "kind": kind, "text": "Stale content", "source": "test",
                    "content_base64": "b2xk", "mime": "text/plain", "filename": "old.txt"
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{entry}");
    }
    app.get(&app.admin, &format!("/v1/initiatives/{id}"))
        .await
        .1
}

#[tokio::test]
async fn initiative_reset_clears_only_selected_document_and_preserves_work() {
    let app = TestApp::spawn().await;
    app.open_store()
        .create_project("op", "Other", None, "test")
        .unwrap();
    let before = seed(&app, "tp").await;
    let other = seed(&app, "op").await;
    let id = before["id"].as_str().unwrap();
    let (_, project_before) = app.get(&app.admin, "/v1/projects/tp").await;
    let (status, ticket) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({
                "project": "tp", "title": "Keep work", "tags": [format!("initiative:{id}")]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    let ticket = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{}", ticket["id"].as_str().unwrap()),
        )
        .await
        .1;
    let (status, check) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({"title": "Keep check", "initiative": id}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{check}");
    let check = app
        .get(
            &app.admin,
            &format!("/v1/checks/{}", check["id"].as_str().unwrap()),
        )
        .await
        .1;
    let (status, after) = app
        .post(
            &app.admin,
            &format!("/v1/initiatives/{id}/reset"),
            json!({"confirm_id": id}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    for key in [
        "id",
        "project",
        "title",
        "status",
        "tags",
        "labels",
        "created_at",
        "created_by",
        "metadata",
    ] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert_eq!(
        app.get(
            &app.admin,
            &format!("/v1/checks/{}", check["id"].as_str().unwrap())
        )
        .await
        .1,
        check
    );
    assert_eq!(after["summary"], "");
    assert_eq!(
        after["version"].as_i64(),
        Some(before["version"].as_i64().unwrap() + 1)
    );
    let (_, entries) = app
        .get(&app.admin, &format!("/v1/initiatives/{id}/entries"))
        .await;
    assert_eq!(entries["items"], json!([]));
    assert_eq!(
        app.get(
            &app.admin,
            &format!("/v1/initiatives/{}", other["id"].as_str().unwrap())
        )
        .await
        .1,
        other
    );
    assert_eq!(
        app.get(&app.admin, "/v1/projects/tp").await.1,
        project_before
    );
    assert_eq!(
        app.get(
            &app.admin,
            &format!("/v1/tickets/{}", ticket["id"].as_str().unwrap())
        )
        .await
        .1,
        ticket
    );
}

#[tokio::test]
async fn initiative_reset_requires_admin_project_confirmation_and_writable_project() {
    let app = TestApp::spawn().await;
    let before = seed(&app, "tp").await;
    let id = before["id"].as_str().unwrap();
    let path = format!("/v1/initiatives/{id}/reset");
    let elsewhere = app.mint(
        "human:elsewhere",
        &["admin", "read", "write"],
        Some(&["op"]),
    );
    for token in [&app.worker, &app.human, &elsewhere] {
        assert_eq!(
            app.post(token, &path, json!({"confirm_id": id})).await.0,
            StatusCode::FORBIDDEN
        );
    }
    for body in [
        json!({}),
        json!({"confirm_id": "wrong"}),
        json!({"confirm_id": null}),
    ] {
        assert!(app.post(&app.admin, &path, body).await.0.is_client_error());
    }
    assert_eq!(
        app.get(&app.admin, &format!("/v1/initiatives/{id}"))
            .await
            .1,
        before
    );
    assert_eq!(
        app.post(&app.admin, "/v1/projects/tp/archive", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    let (status, error) = app.post(&app.admin, &path, json!({"confirm_id": id})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "project.archived");
    assert_eq!(
        app.get(&app.admin, &format!("/v1/initiatives/{id}"))
            .await
            .1,
        before
    );
}
