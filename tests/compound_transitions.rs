//! REST compound verbs use the hosted MCP transactions, including rollback.
mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::json;

#[tokio::test]
async fn compound_start_rolls_back_and_returns_a_reusable_lease() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("atomic start").await;
    app.to_ready(&id).await;
    let path = format!("/v1/tickets/{id}/start");
    let (status, error) = app.post(&app.worker, &path, json!({"to":"done"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(error["allowed_transitions"].is_array());
    let (_, claim) = app
        .get(&app.worker, &format!("/v1/tickets/{id}/claim"))
        .await;
    assert!(claim["holder"].is_null(), "{claim}");
    let (status, ticket) = app
        .post(
            &app.worker,
            &path,
            json!({"to":"implementing", "ttl_seconds":60}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{ticket}");
    assert_eq!(ticket["state"], "implementing");
    let fence = ticket["lease"]["fence"].as_i64().expect("returned fence");
    let (status, lease) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({"fence":fence}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{lease}");
}

#[tokio::test]
async fn compound_block_rolls_back_comments_on_transition_and_fence_refusals() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("atomic block").await;
    let path = format!("/v1/tickets/{id}/block");
    let (status, error) = app
        .post(
            &app.worker,
            &path,
            json!({"to":"needs-decision", "comment":"must roll back"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    app.to_ready(&id).await;
    let fence = app.claim(&id).await;
    let (status, ticket) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/start"),
            json!({"to":"implementing", "fence":fence}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{ticket}");
    let (status, error) = app
        .post(
            &app.worker,
            &path,
            json!({"to":"needs-decision", "fence":fence+1, "comment":"stale must roll back"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    let (_, ticket) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert_eq!(ticket["comments"].as_array().unwrap().len(), 0, "{ticket}");
    let (status, ticket) = app
        .post(
            &app.worker,
            &path,
            json!({"to":"needs-decision", "comment":"persist exactly once", "fence":fence}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{ticket}");
    let (_, ticket) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert_eq!(ticket["comments"].as_array().unwrap().len(), 1, "{ticket}");
}

#[tokio::test]
async fn compound_transitions_enforce_auth_and_request_validation() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("compound guards").await;
    for verb in ["start", "block"] {
        let path = format!("/v1/tickets/{id}/{verb}");
        let (status, _) = app.post("invalid", &path, json!({"to":"spec"})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        for body in [
            json!({}),
            json!({"to":"spec", "extra":true}),
            json!({"to":"spec", "fence":"wrong"}),
        ] {
            let (status, error) = app.post(&app.worker, &path, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
        }
    }
}

#[tokio::test]
async fn compound_transitions_preserve_scope_project_and_archive_guards() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("compound auth").await;
    app.to_ready(&id).await;
    let store = app.open_store();
    let (_, reader) = store
        .create_token("reader", &["read".into()], None, 100, None, None)
        .unwrap();
    let (_, outsider) = store
        .create_token(
            "outsider",
            &["read".into(), "write".into()],
            Some(&["other".into()]),
            100,
            None,
            None,
        )
        .unwrap();
    for verb in ["start", "block"] {
        let path = format!("/v1/tickets/{id}/{verb}");
        for token in [&reader, &outsider] {
            let (status, error) = app.post(token, &path, json!({"to":"implementing"})).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
        }
    }
    let (status, _) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    for verb in ["start", "block"] {
        let (status, error) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{id}/{verb}"),
                json!({"to":"implementing"}),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["code"], "project.archived", "{error}");
    }
    let (_, ticket) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert_eq!(ticket["state"], "ready");
    assert!(ticket["claim"].is_null());
    assert_eq!(ticket["comments"].as_array().unwrap().len(), 0);
}
