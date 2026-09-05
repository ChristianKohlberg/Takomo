mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn definition(app: &TestApp, policy: &str) -> Value {
    let (status, check) = app.post(&app.admin,"/v1/projects/tp/checks",json!({"title":"Sign in","body":"Enter credentials; expect a session","verification":policy})).await;
    assert_eq!(status, StatusCode::CREATED, "{check}");
    let id = check["id"].as_str().unwrap();
    let (status, body)=app.put(&app.admin,&format!("/v1/checks/{id}/cases"),json!({"cases":[{"key":"valid","label":"Valid credentials","assignment":{"user":"ada"}}]})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    app.get(&app.admin, &format!("/v1/checks/{id}/definition"))
        .await
        .1
}
fn request(d: &Value, key: &str) -> Value {
    json!({"definitions":[{"check":d["id"],"definition_revision":d["definition_revision"],"specification_revision":d["specification_revision"]}],"code_ref":"abc123","idempotency_key":key})
}
async fn create(app: &TestApp, d: &Value, key: &str) -> Value {
    let (status, run) = app
        .post(&app.worker, "/v1/projects/tp/test-runs", request(d, key))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    run
}
async fn transition(app: &TestApp, run: &Value, action: &str) -> Value {
    let (status, body) = app
        .patch(
            &app.worker,
            &format!("/v1/test-runs/{}", run["id"].as_str().unwrap()),
            json!({"action":action}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn outcome(
    app: &TestApp,
    run: &Value,
    kind: &str,
    token: &str,
    key: &str,
) -> (StatusCode, Value) {
    app.post(token,&format!("/v1/test-runs/{}/results",run["id"].as_str().unwrap()),json!({"case":run["cases"][0]["case"],"actor_kind":kind,"verdict":"pass","evidence":["https://ci.example/job/1"],"idempotency_key":key})).await
}
#[tokio::test]
async fn attempts_pin_revisions_and_results_are_immutable() {
    let app = TestApp::spawn().await;
    let d = definition(&app, "agent").await;
    let run = create(&app, &d, "first").await;
    assert_eq!(run["status"], "queued");
    assert_eq!(
        outcome(&app, &run, "agent", &app.worker, "r").await.0,
        StatusCode::CONFLICT
    );
    transition(&app, &run, "start").await;
    assert_eq!(
        outcome(&app, &run, "agent", &app.worker2, "r").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        outcome(&app, &run, "agent", &app.worker, "r").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        outcome(&app, &run, "agent", &app.worker, "r").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        outcome(&app, &run, "agent", &app.worker, "other").await.0,
        StatusCode::CONFLICT
    );
    transition(&app, &run, "complete").await;
    let (_, list) = app
        .get(&app.admin, "/v1/projects/tp/test-definitions")
        .await;
    assert_eq!(list["items"][0]["execution"]["state"], "verified", "{list}");
    let id = d["id"].as_str().unwrap();
    assert_eq!(
        app.patch(
            &app.admin,
            &format!("/v1/checks/{id}"),
            json!({"body":"Changed expectations"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, list) = app
        .get(&app.admin, "/v1/projects/tp/test-definitions")
        .await;
    assert_eq!(list["items"][0]["execution"]["state"], "outdated", "{list}");
    assert_eq!(
        create(&app, &d, "first").await["id"],
        run["id"],
        "replaying creation survives edits"
    );
    assert_eq!(
        app.post(
            &app.worker,
            "/v1/projects/tp/test-runs",
            request(&d, "stale")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let path = format!("/v1/test-runs/{}", run["id"].as_str().unwrap());
    let (_, saved) = app.get(&app.worker, &path).await;
    assert_eq!(
        saved["definitions"][id]["definition"]["body"],
        "Enter credentials; expect a session"
    );
    let (status, retry) = app
        .post(
            &app.worker,
            &format!("{path}/retry"),
            json!({"idempotency_key":"retry"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{retry}");
    assert_eq!(
        retry["cases"][0]["definition_revision"],
        run["cases"][0]["definition_revision"]
    );
    assert_eq!(retry["cases"][0]["results"], json!([]));
    assert_eq!(retry["retry_of"], run["id"]);
    let (_, page) = app
        .get(&app.worker, "/v1/projects/tp/test-runs?limit=1")
        .await;
    assert_eq!(page["total"], 2);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert!(page["next_cursor"].is_string());
}
#[tokio::test]
async fn approval_is_separate_and_bound_to_the_attempt() {
    let app = TestApp::spawn().await;
    let d = definition(&app, "agent_then_human").await;
    let run = create(&app, &d, "first").await;
    transition(&app, &run, "start").await;
    assert_eq!(
        outcome(&app, &run, "human", &app.worker, "human").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        outcome(&app, &run, "human", &app.human, "human").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        outcome(&app, &run, "agent", &app.worker, "agent").await.0,
        StatusCode::CREATED
    );
    transition(&app, &run, "complete").await;
    let (_, list) = app
        .get(&app.admin, "/v1/projects/tp/test-definitions")
        .await;
    assert_eq!(list["items"][0]["execution"]["state"], "needs_approval");
    assert_eq!(
        outcome(&app, &run, "human", &app.human, "human").await.0,
        StatusCode::CREATED
    );
    let (_, list) = app
        .get(&app.admin, "/v1/projects/tp/test-definitions")
        .await;
    assert_eq!(list["items"][0]["execution"]["state"], "verified");
}
#[tokio::test]
async fn legacy_evidence_is_preserved_without_inventing_revisions() {
    let app = TestApp::spawn().await;
    let d = definition(&app, "agent").await;
    let case = d["definition"]["cases"][0]["id"].as_str().unwrap();
    let (status, body) = app
        .post(
            &app.worker,
            &format!("/v1/cases/{case}/verdict"),
            json!({"verdict":"pass"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, list) = app.get(&app.admin, "/v1/projects/tp/test-runs").await;
    assert_eq!(list["total"], 1, "{list}");
    let id = list["items"][0]["id"].as_str().unwrap();
    let (_, run) = app.get(&app.admin, &format!("/v1/test-runs/{id}")).await;
    assert_eq!(run["kind"], "legacy");
    assert_eq!(run["cases"][0]["revision_known"], false);
    assert_eq!(run["cases"][0]["results"][0]["verdict"], "pass");
    // Simulate an old database containing the verdict but no imported ledger yet.
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute("DELETE FROM test_run_results", []).unwrap();
    conn.execute("DELETE FROM test_run_cases", []).unwrap();
    conn.execute("DELETE FROM test_runs", []).unwrap();
    let _reopened = app.open_store();
    assert_eq!(
        app.get(&app.admin, "/v1/projects/tp/test-runs").await.1["total"],
        1
    );
    assert_eq!(
        app.get(&app.admin, "/v1/projects/tp/test-definitions")
            .await
            .1["items"][0]["execution"]["state"],
        "not_executed"
    );
}

#[tokio::test]
async fn environments_cannot_hide_missing_runs_or_mix_code_versions() {
    let app = TestApp::spawn().await;
    let d = definition(&app, "agent").await;
    for slug in ["staging", "production"] {
        let (status, body) = app
            .post(
                &app.admin,
                "/v1/projects/tp/environments",
                json!({"slug":slug,"name":slug}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let id = d["id"].as_str().unwrap();
    let (status, body) = app
        .patch(
            &app.admin,
            &format!("/v1/checks/{id}"),
            json!({"environments":["staging","production"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, d) = app
        .get(&app.admin, &format!("/v1/checks/{id}/definition"))
        .await;
    assert_eq!(
        app.post(
            &app.worker,
            "/v1/projects/tp/test-runs",
            request(&d, "no-env")
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for (env, code, expected) in [
        ("staging", "aaa", "not_executed"),
        ("production", "bbb", "mixed_versions"),
    ] {
        let mut req = request(&d, env);
        req["environment"] = json!(env);
        req["code_ref"] = json!(code);
        let (status, run) = app
            .post(&app.worker, "/v1/projects/tp/test-runs", req)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{run}");
        transition(&app, &run, "start").await;
        assert_eq!(
            outcome(&app, &run, "agent", &app.worker, "pass").await.0,
            StatusCode::CREATED
        );
        transition(&app, &run, "complete").await;
        let (_, list) = app
            .get(&app.admin, "/v1/projects/tp/test-definitions")
            .await;
        assert_eq!(list["items"][0]["execution"]["state"], expected, "{list}");
        assert_eq!(
            list["items"][0]["execution"]["environments"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
#[tokio::test]
async fn claims_are_atomic_and_all_routes_enforce_project_scope() {
    let app = TestApp::spawn().await;
    let d = definition(&app, "agent").await;
    let run = create(&app, &d, "first").await;
    let path = format!("/v1/test-runs/{}", run["id"].as_str().unwrap());
    let (a, b) = tokio::join!(
        app.patch(&app.worker, &path, json!({"action":"start"})),
        app.patch(&app.worker2, &path, json!({"action":"start"}))
    );
    assert!(
        matches!(
            (a.0, b.0),
            (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)
        ),
        "{a:?} {b:?}"
    );
    let stranger = app.mint(
        "agent:stranger",
        &["read", "write", "human"],
        Some(&["other"]),
    );
    for route in [
        "/v1/projects/tp/test-definitions".to_string(),
        "/v1/projects/tp/test-runs".to_string(),
        format!("/v1/checks/{}/definition", d["id"].as_str().unwrap()),
        path.clone(),
    ] {
        assert_eq!(
            app.get(&stranger, &route).await.0,
            StatusCode::FORBIDDEN,
            "{route}"
        );
    }
    assert_eq!(
        app.post(&stranger, "/v1/projects/tp/test-runs", request(&d, "no"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.patch(&stranger, &path, json!({"action":"cancel"}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.post(
            &stranger,
            &format!("{path}/retry"),
            json!({"idempotency_key":"no"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        outcome(&app, &run, "agent", &stranger, "no").await.0,
        StatusCode::FORBIDDEN
    );
    let reader = app.mint("agent:reader", &["read"], Some(&["tp"]));
    assert_eq!(app.get(&reader, &path).await.0, StatusCode::OK);
    assert_eq!(
        app.post(&reader, "/v1/projects/tp/test-runs", request(&d, "reader"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn specification_edits_invalidate_selection_without_rewriting_history() {
    let app = TestApp::spawn().await;
    let d = definition(&app, "agent").await;
    let (status, map) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Specification"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{map}");
    let map = map["mindmap"]["id"].as_str().unwrap();
    let (status, node) = app
        .post(
            &app.admin,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"text":"Sign in requirement"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{node}");
    let node = node["nodes"][0]["id"].as_str().unwrap();
    let id = d["id"].as_str().unwrap();
    assert_eq!(
        app.patch(
            &app.admin,
            &format!("/v1/checks/{id}"),
            json!({"node":node})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, d) = app
        .get(&app.admin, &format!("/v1/checks/{id}/definition"))
        .await;
    let run = create(&app, &d, "pinned").await;
    assert_eq!(
        app.patch(
            &app.admin,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({"text":"Sign in with MFA"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, error) = app
        .post(
            &app.worker,
            "/v1/projects/tp/test-runs",
            request(&d, "changed"),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "conflict.definition_changed");
    let (_, saved) = app
        .get(
            &app.admin,
            &format!("/v1/test-runs/{}", run["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(
        saved["definitions"][id]["specification"]["sections"][0]["title"],
        "Sign in requirement"
    );
    let (status, body) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM test_specification_revisions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "deleting the project also removes captured prose");
}

#[tokio::test]
async fn expiry_archiving_and_project_deletion_preserve_the_right_boundaries() {
    let app = TestApp::spawn().await;
    let d = definition(&app, "agent").await;
    let id = d["id"].as_str().unwrap();
    app.patch(
        &app.admin,
        &format!("/v1/checks/{id}"),
        json!({"expiry_days":1}),
    )
    .await;
    let (_, d) = app
        .get(&app.admin, &format!("/v1/checks/{id}/definition"))
        .await;
    let run = create(&app, &d, "expiry").await;
    transition(&app, &run, "start").await;
    outcome(&app, &run, "agent", &app.worker, "pass").await;
    transition(&app, &run, "complete").await;
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute(
        "UPDATE test_runs SET started_at=started_at-172800000 WHERE id=?1",
        [run["id"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        app.get(&app.admin, "/v1/projects/tp/test-definitions")
            .await
            .1["items"][0]["execution"]["state"],
        "outdated"
    );
    let (status, body) = app.delete(&app.admin, &format!("/v1/checks/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        app.get(
            &app.admin,
            &format!("/v1/test-runs/{}", run["id"].as_str().unwrap())
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, body) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    for table in [
        "test_runs",
        "test_run_cases",
        "test_run_results",
        "test_definition_revisions",
        "test_specification_revisions",
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}
