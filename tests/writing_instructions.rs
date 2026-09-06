use reqwest::StatusCode;
use serde_json::json;
mod common;
use common::TestApp;

#[tokio::test]
async fn writing_instructions_persist_validate_and_stay_project_scoped() {
    let app = TestApp::spawn().await;
    let path = "/v1/projects/tp/writing-instructions";
    let empty = json!({"templates": [], "default_id": null});
    assert_eq!(
        app.get(&app.worker, path).await,
        (StatusCode::OK, empty.clone())
    );
    let settings = json!({"templates": [{"id":"concise", "name":"Short headings", "instruction":"Keep headings short. Put explanations in bodies."}], "default_id":"concise"});
    assert_eq!(
        app.put(&app.worker, path, settings.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.put(&app.admin, path, settings.clone()).await,
        (StatusCode::OK, settings.clone())
    );
    assert_eq!(app.get(&app.worker, path).await.1, settings);
    let store = app.open_store();
    assert_eq!(
        store
            .writing_instructions("tp")
            .unwrap()
            .default_id
            .as_deref(),
        Some("concise")
    );
    store
        .create_project("other", "Other", None, "test")
        .unwrap();
    assert_eq!(
        app.get(&app.worker, "/v1/projects/other/writing-instructions")
            .await
            .1,
        empty
    );
    let scoped = app.mint("agent:other", &["read", "admin"], Some(&["other"]));
    assert_eq!(app.get(&scoped, path).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        app.put(&scoped, path, empty.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    for invalid in [
        json!({"templates": [], "default_id":"missing"}),
        json!({"templates": [{"id":"x", "name":" ", "instruction":"text"}], "default_id":null}),
        json!({"templates": [{"id":"x", "name":"x", "instruction":"text"}, {"id":"x", "name":"y", "instruction":"text"}], "default_id":null}),
        json!({"templates": [{"id":"x", "name":"x", "instruction":"x".repeat(4001)}], "default_id":null}),
    ] {
        assert_eq!(
            app.put(&app.admin, path, invalid).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(app.get(&app.worker, path).await.1, settings);
    }
    assert_eq!(
        app.put(&app.admin, path, json!({"templates":[]})).await.0,
        StatusCode::BAD_REQUEST
    );
    store
        .set_style_guide("tp", Some("Existing house style"), "test")
        .unwrap();
    let prompt = store
        .writing_prompt("tp", "Use a long heading here")
        .unwrap();
    assert!(prompt.contains("advisory"));
    assert!(prompt.ends_with("User request:\nUse a long heading here"));
    assert_eq!(
        store
            .get_project("tp")
            .unwrap()
            .unwrap()
            .style_guide
            .as_deref(),
        Some("Existing house style")
    );
    let (status, map) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp", "title":"Plan"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = map["mindmap"]["id"].as_str().unwrap();
    for endpoint in [
        format!("/v1/mindmaps/{id}"),
        format!("/v1/mindmaps/{id}/prose"),
    ] {
        let (status, body) = app.get(&app.worker, &endpoint).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["default_writing_instruction"],
            settings["templates"][0]
        );
    }
    store
        .set_project_archived("tp", true, false, "test")
        .unwrap();
    assert_eq!(
        app.put(&app.admin, path, empty.clone()).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(app.get(&app.worker, path).await.1, settings);
    store
        .set_project_archived("tp", false, false, "test")
        .unwrap();
    assert_eq!(
        app.put(&app.admin, path, empty.clone()).await.0,
        StatusCode::OK
    );
    assert_eq!(store.writing_prompt("tp", "Request").unwrap(), "Request");
    store.delete_project("tp", false, "test").unwrap();
    store
        .create_project("tp", "Recreated", None, "test")
        .unwrap();
    assert_eq!(app.get(&app.worker, path).await.1, empty);
    assert_eq!(
        app.get(&app.worker, "/v1/projects/missing/writing-instructions")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn writing_instructions_reach_editor_model_requests() {
    use axum::{routing::post, Json, Router};
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let observed = captured.clone();
    let mock = Router::new().route("/chat/completions", post(move |Json(body): Json<Value>| {
        let observed = observed.clone();
        async move {
            observed.lock().unwrap().push(body);
            Json(json!({"choices":[{"message":{"content":"{\"summary\":\"No changes\",\"ops\":[]}"}}]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = format!("http://{}", listener.local_addr().unwrap());
    let model_server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let mut app = TestApp::spawn().await;
    let settings = json!({"templates":[{"id":"guide","name":"Guide","instruction":"Keep every heading short."}],"default_id":"guide"});
    assert_eq!(
        app.put(&app.admin, "/v1/projects/tp/writing-instructions", settings)
            .await
            .0,
        StatusCode::OK
    );
    let state = takomo::server::AppState::new_with_agent(
        app.open_store(),
        None,
        Some(takomo::docagent::DocAgentConfig {
            api_key: "test".into(),
            base_url: upstream,
            model: "test".into(),
        }),
        None,
    );
    let router = takomo::server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    app.base = base.clone();
    let (_, doc) = app
        .post(
            &app.worker,
            "/v1/projects/tp/documents",
            json!({"title":"Notes"}),
        )
        .await;
    let doc_id = doc["id"].as_str().unwrap();
    let (_, map) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Plan"}),
        )
        .await;
    let map_id = map["mindmap"]["id"].as_str().unwrap();
    let (_, nodes) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map_id}/nodes"),
            json!({"nodes":[{"text":"Section"}]}),
        )
        .await;
    let node_id = nodes["nodes"][0]["id"].as_str().unwrap();
    for (path, body) in [
        (
            format!("/v1/documents/{doc_id}/run"),
            json!({"instruction":"Use a long heading for this request."}),
        ),
        (
            format!("/v1/mindmaps/{map_id}/run"),
            json!({"node":node_id,"instruction":"Use a long heading for this request."}),
        ),
    ] {
        let response = app
            .client
            .post(format!("{base}{path}"))
            .bearer_auth(&app.worker)
            .json(&body)
            .send()
            .await
            .unwrap();
        // The mock intentionally proposes no edits; reaching operation validation
        // proves the model reply returned through the actual editor route.
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["code"], "validation.document_ops");
    }
    let requests = captured.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        let prompt = request["messages"][1]["content"].as_str().unwrap();
        assert!(prompt.contains("Keep every heading short."));
        assert!(prompt.contains("follow the user's explicit request when it conflicts"));
        assert!(prompt.contains("User request:\nUse a long heading for this request."));
    }
    server.abort();
    model_server.abort();
}
