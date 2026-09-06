mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::Duration;
use yrs::{updates::decoder::Decode, Doc, Transact, Update};

const CLAIM: &str = "/v1/agent-jobs/claim";

#[tokio::test]
async fn queue_inspection_is_project_scoped_and_does_not_expose_worker_credentials() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, node, path) = fixture(&app).await;
    let queued = send(&app, &path, "inspect", "Grill this section").await;
    let id = queued["jobs"][0]["id"].as_str().unwrap();
    let detail = format!("/v1/agent-jobs/{id}");
    let reader = app.mint("human:observer", &["read"], Some(&["tp"]));
    let outsider = app.mint("human:outsider", &["read"], Some(&["elsewhere"]));
    let runner = runner(&app);
    for endpoint in ["/v1/agent-jobs", detail.as_str()] {
        assert_eq!(app.get(&runner, endpoint).await.0, StatusCode::FORBIDDEN);
    }
    let (status, outside) = app.get(&outsider, "/v1/agent-jobs").await;
    assert_eq!(status, StatusCode::OK, "{outside}");
    assert_eq!(outside["items"], json!([]));
    assert_eq!(
        outside["counts"],
        json!({"queued":0,"running":0,"completed":0,"failed":0,"cancelled":0})
    );
    assert_eq!(
        app.get(&outsider, "/v1/agent-jobs?project=tp").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(app.get(&outsider, &detail).await.0, StatusCode::FORBIDDEN);
    let job = claim(&app, &runner, "inspection-worker").await;
    let (status, inspected) = app.get(&reader, &detail).await;
    assert_eq!(status, StatusCode::OK, "{inspected}");
    assert_eq!(inspected["job"]["service_id"], "inspection-worker");
    assert_eq!(inspected["job"]["attempt_id"], job["attempt_id"]);
    assert_eq!(inspected["job"]["prompt"], "Grill this section");
    assert!(inspected["job"].get("token_id").is_none());
    assert!(inspected["job"].get("result_json").is_none());
    assert!(!inspected.to_string().contains(&runner));
    let (status, listed) = app.get(&reader, "/v1/agent-jobs?project=tp").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    for field in ["token_id", "result_json", "prompt", "snapshot", "response"] {
        assert!(
            listed["items"][0].get(field).is_none(),
            "list leaks {field}"
        );
    }
    // Inspecting overdue work must not itself expire a lease or emit events.
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute("UPDATE agent_jobs SET lease_expires_at=0 WHERE id=?1", [id])
        .unwrap();
    let before = app.open_store().agent_conversation(&map, &node).unwrap();
    assert_eq!(
        app.get(&reader, &detail).await.1["job"]["status"],
        "running"
    );
    assert_eq!(
        app.get(&reader, "/v1/agent-jobs").await.1["items"][0]["lease_expires_at"],
        0
    );
    assert_eq!(read(&app, &path).await, before);
}

#[tokio::test]
async fn queue_inspection_filters_jobs_and_preserves_historical_input_and_results() {
    let app = TestApp::spawn_without_sweeper().await;
    let token = runner(&app);
    let (map, node, path) = fixture(&app).await;
    send(&app, &path, "first-inspection", "Grill this section").await;
    let job = claim(&app, &token, "inspection-worker").await;
    assert_eq!(
        app.post(
            &token,
            &endpoint(&job, "result"),
            completed(&job, "inspection-worker", "turn-1")
        )
        .await
        .0,
        StatusCode::OK
    );
    let queued = send(&app, &path, "second-inspection", "Please explain").await;
    let (status, listed) = app
        .get(&app.human, "/v1/agent-jobs?project=tp&limit=1")
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    assert_eq!(listed["items"][0]["status"], "queued");
    assert_eq!(
        listed["items"][0]["conversation_service_id"],
        "inspection-worker"
    );
    assert_eq!(listed["total"], 2);
    assert_eq!(
        listed["counts"],
        json!({"queued":1,"running":0,"completed":1,"failed":0,"cancelled":0})
    );
    let (status, filtered) = app
        .get(&app.human, "/v1/agent-jobs?project=tp&status=completed")
        .await;
    assert_eq!(status, StatusCode::OK, "{filtered}");
    assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["items"][0]["id"], job["id"]);
    assert_eq!(filtered["counts"], listed["counts"]);
    assert_eq!(filtered["total"], 1);
    assert_eq!(
        app.patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({"text":"Changed title","notes":"Changed requirements"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let inspected = read(
        &app,
        &format!("/v1/agent-jobs/{}", job["id"].as_str().unwrap()),
    )
    .await;
    assert_eq!(inspected["job"]["section_title"], "Overdue invoices");
    assert_eq!(
        inspected["job"]["snapshot"],
        "# Overdue invoices\n\nNotify the customer promptly."
    );
    assert_eq!(
        inspected["job"]["response"],
        "How many hours count as promptly?"
    );
    assert_eq!(inspected["messages"].as_array().unwrap().len(), 3);
    assert_eq!(read(&app, &path).await["jobs"], queued["jobs"]);
    for (query, expected) in [
        ("status=bogus", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=101", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=nope", StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(
            app.get(&app.human, &format!("/v1/agent-jobs?{query}"))
                .await
                .0,
            expected,
            "{query}"
        );
    }
}

async fn fixture(app: &TestApp) -> (String, String, String) {
    let (s, map) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Payments"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{map}");
    let map = map["mindmap"]["id"].as_str().unwrap().to_owned();
    let (s, node) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"text":"Overdue invoices","notes":"Notify the customer promptly."}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{node}");
    let node = node["nodes"][0]["id"].as_str().unwrap().to_owned();
    let path = format!("/v1/mindmaps/{map}/nodes/{node}/conversation");
    (map, node, path)
}
fn runner(app: &TestApp) -> String {
    app.mint("agent:runner", &["agent:run"], Some(&["tp"]))
}
async fn send(app: &TestApp, path: &str, request: &str, message: &str) -> Value {
    let (s, v) = app
        .post(
            &app.human,
            &format!("{path}/messages"),
            json!({"request_id":request,"message":message}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}
async fn claim(app: &TestApp, token: &str, service: &str) -> Value {
    let (s, v) = app.post(token, CLAIM, json!({"service_id":service})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["job"].clone()
}
fn endpoint(job: &Value, action: &str) -> String {
    format!("/v1/agent-jobs/{}/{action}", job["id"].as_str().unwrap())
}
fn completed(job: &Value, service: &str, turn: &str) -> Value {
    json!({"service_id":service,"attempt_id":job["attempt_id"],"status":"completed","message":"How many hours count as promptly?","thread_id":"thread-1","turn_id":turn})
}
async fn read(app: &TestApp, path: &str) -> Value {
    let (s, v) = app.get(&app.human, path).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

#[tokio::test]
async fn conversation_roundtrip_is_durable_idempotent_and_read_only() {
    let app = TestApp::spawn_without_sweeper().await;
    let token = runner(&app);
    let (map, node, path) = fixture(&app).await;
    assert!(read(&app, &path).await["conversation"].is_null());
    let queued = send(&app, &path, "first", "Grill this section").await;
    assert_eq!(queued["jobs"][0]["status"], "queued");
    assert_eq!(queued["messages"].as_array().unwrap().len(), 1);
    assert_eq!(
        send(&app, &path, "first", "Grill this section").await,
        queued
    );
    let (s, _) = app
        .post(
            &app.human,
            &format!("{path}/messages"),
            json!({"request_id":"first","message":"Different request"}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Edit after submission: the claimed input must retain the submitted snapshot.
    let (s, v) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({"notes":"Notify within 24 hours."}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // REST patch awaits the persistence barrier. Verify its durable CRDT state,
    // then observe with read-only credentials: write-capable GETs can migrate
    // legacy prose and would contaminate this check of the queue's behavior.
    let reader = app.mint("human:observer", &["read"], Some(&["tp"]));
    let updates_before = app.open_store().load_collab_updates(&map).unwrap();
    let durable = Doc::new();
    for update in &updates_before {
        durable
            .transact_mut()
            .apply_update(Update::decode_v1(update).unwrap())
            .unwrap();
    }
    let (_, _, durable_nodes) = takomo::store::mindmapdoc::snapshot(&durable, &map);
    assert_eq!(
        durable_nodes.iter().find(|n| n.id == node).unwrap().notes,
        "Notify within 24 hours."
    );
    let (s, before) = app.get(&reader, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{before}");
    let (s, history_before) = app
        .get(&reader, &format!("/v1/mindmaps/{map}/versions"))
        .await;
    assert_eq!(s, StatusCode::OK, "{history_before}");
    let job = claim(&app, &token, "local").await;
    assert_eq!(job["id"], queued["jobs"][0]["id"]);
    assert_eq!(
        job["snapshot"],
        "# Overdue invoices\n\nNotify the customer promptly."
    );
    assert_eq!(job["source_revision"], queued["jobs"][0]["source_revision"]);
    assert!(job["thread_id"].is_null());
    let (s, hb) = app.post(&token, &endpoint(&job, "heartbeat"), json!({"service_id":"local","attempt_id":job["attempt_id"],"thread_id":"thread-1","turn_id":"turn-1"})).await;
    assert_eq!(s, StatusCode::OK, "{hb}");
    assert!(hb["lease_expires_at"].as_i64().unwrap() > chrono::Utc::now().timestamp_millis());
    let result = completed(&job, "local", "turn-1");
    for _ in 0..2 {
        let (s, v) = app
            .post(&token, &endpoint(&job, "result"), result.clone())
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let mut different = result;
    different["message"] = json!("Another answer");
    assert_eq!(
        app.post(&token, &endpoint(&job, "result"), different)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let saved = read(&app, &path).await;
    assert_eq!(saved["messages"].as_array().unwrap().len(), 2);
    assert_eq!(saved["messages"][1]["role"], "assistant");
    assert_eq!(
        saved["messages"][1]["body"],
        "How many hours count as promptly?"
    );
    assert_eq!(saved["jobs"][0]["status"], "completed");
    assert_eq!(
        app.open_store().agent_conversation(&map, &node).unwrap(),
        saved
    );
    let (s, after) = app.get(&reader, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{after}");
    // Map.updated_at is cache/flush metadata; authoritative content, standing,
    // version history and the actual persisted CRDT updates must be untouched.
    for field in ["nodes", "relationships", "standing", "total"] {
        assert_eq!(after[field], before[field], "agent changed {field}");
    }
    assert_eq!(
        app.get(&reader, &format!("/v1/mindmaps/{map}/versions"))
            .await
            .1,
        history_before
    );
    assert_eq!(
        app.open_store().load_collab_updates(&map).unwrap(),
        updates_before
    );

    send(&app, &path, "second", "Within 24 hours.").await;
    assert!(claim(&app, &token, "another-machine").await.is_null());
    let followup = claim(&app, &token, "local").await;
    assert_eq!(followup["thread_id"], "thread-1");
    assert_eq!(followup["conversation_id"], job["conversation_id"]);
    assert_eq!(followup["prompt"], "Within 24 hours.");
    assert_eq!(
        followup["snapshot"],
        "# Overdue invoices\n\nNotify within 24 hours."
    );
    assert_ne!(followup["source_revision"], job["source_revision"]);
    assert_eq!(
        app.post(
            &token,
            &endpoint(&followup, "result"),
            completed(&followup, "local", "turn-2")
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        read(&app, &path).await["messages"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[tokio::test]
async fn queue_authorization_and_attempt_ownership_are_separate_from_document_access() {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, _, path) = fixture(&app).await;
    let token = runner(&app);
    let other_project = app.mint(
        "agent:elsewhere",
        &["agent:run", "read", "write", "human"],
        Some(&["other"]),
    );
    let readonly = app.mint("human:reader", &["read"], Some(&["tp"]));
    for denied in [&token, &app.worker, &readonly, &other_project] {
        assert_eq!(
            app.post(
                denied,
                &format!("{path}/messages"),
                json!({"request_id":"denied","message":"Grill"})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(app.get(&readonly, &path).await.0, StatusCode::OK);
    for denied in [&token, &other_project] {
        assert_eq!(app.get(denied, &path).await.0, StatusCode::FORBIDDEN);
    }
    for denied in [&app.human, &app.admin, &app.worker] {
        assert_eq!(
            app.post(denied, CLAIM, json!({"service_id":"local"}))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    send(&app, &path, "first", "Grill").await;
    assert!(claim(&app, &other_project, "local").await.is_null());
    let job = claim(&app, &token, "local").await;
    let body = completed(&job, "local", "turn-1");
    assert_eq!(
        app.post(&other_project, &endpoint(&job, "result"), body.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let impostor = runner(&app); // Same actor and scopes, different credential.
    assert_eq!(
        app.post(&impostor, &endpoint(&job, "result"), body.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut wrong = body.clone();
    wrong["attempt_id"] = json!("stale-attempt");
    assert_eq!(
        app.post(&token, &endpoint(&job, "result"), wrong).await.0,
        StatusCode::CONFLICT
    );
    let mut wrong = body.clone();
    wrong["service_id"] = json!("another-service");
    assert_eq!(
        app.post(&token, &endpoint(&job, "result"), wrong).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        read(&app, &path).await["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        app.post(&token, &endpoint(&job, "result"), body).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn concurrent_claims_have_one_winner_and_active_conversations_reject_new_turns() {
    let app = TestApp::spawn_without_sweeper().await;
    let token = runner(&app);
    let (map, _, path) = fixture(&app).await;
    send(&app, &path, "first", "Grill").await;
    let next = json!({"request_id":"second","message":"Follow up"});
    assert_eq!(
        app.post(&app.human, &format!("{path}/messages"), next.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (a, b) = tokio::join!(claim(&app, &token, "local"), claim(&app, &token, "remote"));
    assert_ne!(
        a.is_null(),
        b.is_null(),
        "exactly one claim must win: {a} / {b}"
    );
    assert_eq!(
        app.post(&app.human, &format!("{path}/messages"), next)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(read(&app, &path).await["jobs"].as_array().unwrap().len(), 1);
    let service = if a.is_null() { "remote" } else { "local" };
    let (s, section) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"text":"Second section"}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{section}");
    let node = section["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["title"] == "Second section")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let second_path = format!("/v1/mindmaps/{map}/nodes/{node}/conversation");
    send(&app, &second_path, "first", "Another section").await;
    assert!(
        claim(&app, &token, service).await.is_null(),
        "a service executes only one turn at a time"
    );
}

#[tokio::test]
async fn expired_attempts_cannot_publish_or_renew_and_failure_survives_reopen() {
    let app = TestApp::spawn_without_sweeper().await;
    let token = runner(&app);
    let (map, node, path) = fixture(&app).await;
    send(&app, &path, "first", "Grill").await;
    let job = claim(&app, &token, "local").await;
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    conn.execute(
        "UPDATE agent_jobs SET lease_expires_at=0 WHERE id=?1",
        [job["id"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        app.post(
            &token,
            &endpoint(&job, "heartbeat"),
            json!({"service_id":"local","attempt_id":job["attempt_id"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.post(
            &token,
            &endpoint(&job, "result"),
            completed(&job, "local", "turn-1")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(app.open_store().sweep_expired_agent_jobs().unwrap(), 1);
    let saved = app.open_store().agent_conversation(&map, &node).unwrap();
    assert_eq!(saved["jobs"][0]["status"], "failed");
    assert!(saved["jobs"][0]["error"]
        .as_str()
        .unwrap()
        .contains("interrupted"));
    assert_eq!(saved["messages"].as_array().unwrap().len(), 1);
    assert!(
        claim(&app, &token, "local").await.is_null(),
        "an interrupted job must not retry automatically"
    );
    send(&app, &path, "second", "Try again").await;
    let retry = claim(&app, &token, "local").await;
    let failed = json!({"service_id":"local","attempt_id":retry["attempt_id"],"status":"failed","error":"Codex authentication is required."});
    for _ in 0..2 {
        assert_eq!(
            app.post(&token, &endpoint(&retry, "result"), failed.clone())
                .await
                .0,
            StatusCode::OK
        );
    }
    let saved = read(&app, &path).await;
    assert_eq!(saved["jobs"][1]["status"], "failed");
    assert_eq!(saved["messages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn waiting_service_wakes_promptly_when_a_message_is_submitted() {
    let app = TestApp::spawn_without_sweeper().await;
    let token = runner(&app);
    let (_, _, path) = fixture(&app).await;
    let wait = app.post(
        &token,
        CLAIM,
        json!({"service_id":"local","wait_seconds":25}),
    );
    let submit = async {
        // Give the HTTP request time to reach the empty queue before enqueueing.
        tokio::time::sleep(Duration::from_millis(100)).await;
        send(&app, &path, "first", "Grill").await
    };
    let ((s, result), queued) =
        tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(wait, submit) })
            .await
            .expect("job submission should wake a pending claim without waiting 25 seconds");
    assert_eq!(s, StatusCode::OK, "{result}");
    assert_eq!(result["job"]["id"], queued["jobs"][0]["id"]);
}

#[tokio::test]
async fn archive_freezes_agent_writes_and_map_deletion_removes_conversation() {
    let app = TestApp::spawn_without_sweeper().await;
    let token = runner(&app);
    let (map, _, path) = fixture(&app).await;
    send(&app, &path, "first", "Grill").await;
    let job = claim(&app, &token, "local").await;
    let (s, v) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = app
        .post(
            &token,
            &endpoint(&job, "result"),
            completed(&job, "local", "turn-1"),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["code"], "project.archived");
    assert_eq!(
        app.post(
            &app.human,
            &format!("{path}/messages"),
            json!({"request_id":"second","message":"Continue"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(claim(&app, &token, "local").await.is_null());
    assert_eq!(read(&app, &path).await["jobs"][0]["status"], "failed");
    assert_eq!(
        app.post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
            .await
            .0,
        StatusCode::OK
    );
    let (s, v) = app.delete(&app.admin, &format!("/v1/mindmaps/{map}")).await;
    assert!(s.is_success(), "{s}: {v}");
    assert_eq!(app.get(&app.human, &path).await.0, StatusCode::NOT_FOUND);
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    for table in ["agent_conversations", "agent_jobs", "agent_messages"] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table} should cascade with its document");
    }
}
