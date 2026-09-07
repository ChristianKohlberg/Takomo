mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
async fn worker(a: &TestApp) -> String {
    a.open_store()
        .create_token(
            "agent:organizer",
            &["read", "agent:run"].map(str::to_string),
            None,
            10000,
            None,
            None,
        )
        .unwrap()
        .1
}
async fn send(a: &TestApp, request: &str) -> Value {
    let (s, v) = a
        .post(
            &a.human,
            "/v1/projects/tp/lane-organizer/messages",
            json!({"request_id":request,"message":"Group related work, explain uncertainty."}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}
async fn claim(a: &TestApp, token: &str) -> Value {
    let (s, v) = a
        .post(
            token,
            "/v1/agent-jobs/claim",
            json!({"service_id":"organizer-service"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["job"].clone()
}
fn proposal(ticket: &str) -> Value {
    json!({"groups":[{"lane_id":null,"title":"Editing","purpose":"Editing work","context":"Preserve existing document behavior.","readiness":"needs_clarification","reason":"Clarify acceptance conditions.","ticket_ids":[ticket]}],"unassigned":[]})
}
async fn finish(a: &TestApp, token: &str, job: &Value, p: Value) -> (StatusCode, Value) {
    a.post(token,&format!("/v1/agent-jobs/{}/result",job["id"].as_str().unwrap()),json!({"service_id":"organizer-service","attempt_id":job["attempt_id"],"status":"completed","message":"One proposed group.","thread_id":"organizer-thread","turn_id":format!("turn-{}",job["id"]),"proposal":p})).await
}
#[tokio::test]
async fn organizer_drafts_then_human_accepts_atomically_and_idempotently() {
    let a = TestApp::spawn().await;
    let t = a.create_ticket("Preserve focus").await;
    let token = worker(&a).await;
    assert_eq!(
        a.post(
            &a.worker,
            "/v1/projects/tp/lane-organizer/messages",
            json!({"request_id":"denied","message":"Organize"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let v = send(&a, "one").await;
    let jid = v["jobs"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(send(&a, "one").await["jobs"].as_array().unwrap().len(), 1);
    let j = claim(&a, &token).await;
    assert_eq!(j["kind"], "lane_organize");
    assert_eq!(j["id"], jid);
    assert_eq!(j["project"], "tp");
    assert_eq!(
        a.get(&a.worker, "/v1/agent-jobs?project=tp").await.1["items"][0]["kind"],
        "lane_organize"
    );
    let (s, v) = finish(&a, &token, &j, proposal(&t)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(finish(&a, &token, &j, proposal(&t)).await.0, StatusCode::OK);
    assert_eq!(
        a.get(&a.worker, "/v1/projects/tp/lanes").await.1["total"],
        0
    );
    let path = format!("/v1/projects/tp/lane-organizer/jobs/{jid}/accept");
    assert_eq!(
        a.post(&a.worker, &path, json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    let (s, v) = a.post(&a.human, &path, json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["jobs"][0]["accepted_at"].is_string());
    assert_eq!(a.post(&a.human, &path, json!({})).await.0, StatusCode::OK);
    let (_, lanes) = a.get(&a.worker, "/v1/projects/tp/lanes").await;
    assert_eq!(lanes["total"], 1);
    assert_eq!(lanes["items"][0]["tickets"][0]["id"], t);
    assert_eq!(
        lanes["items"][0]["readiness"]["status"],
        "needs_clarification"
    );
    assert_eq!(
        a.get(&a.worker, "/v1/projects/tp/handoffs").await.1["total"],
        0
    );
}
#[tokio::test]
async fn organizer_rejects_stale_scope_and_outside_tickets() {
    let a = TestApp::spawn().await;
    let t = a.create_ticket("Fix focus").await;
    let token = worker(&a).await;
    send(&a, "one").await;
    let j = claim(&a, &token).await;
    assert_eq!(
        finish(&a, &token, &j, proposal("outside")).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(finish(&a, &token, &j, proposal(&t)).await.0, StatusCode::OK);
    a.patch(
        &a.worker,
        &format!("/v1/tickets/{t}"),
        json!({"title":"Changed scope"}),
    )
    .await;
    let path = format!(
        "/v1/projects/tp/lane-organizer/jobs/{}/accept",
        j["id"].as_str().unwrap()
    );
    assert_eq!(
        a.post(&a.human, &path, json!({})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        a.get(&a.worker, "/v1/projects/tp/lanes").await.1["total"],
        0
    );
    let (_, limited) = a
        .open_store()
        .create_token(
            "human:other",
            &["read", "write", "human"].map(str::to_string),
            Some(&["other".into()]),
            10000,
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        a.get(&limited, "/v1/projects/tp/lane-organizer").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        a.post(&limited, &path, json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    send(&a, "two").await;
    let second = claim(&a, &token).await;
    assert_eq!(second["thread_id"], "organizer-thread");
    let mut p = proposal(&t);
    p["unassigned"] = json!([{"ticket_id":t,"reason":"Duplicate entry"}]);
    assert_eq!(
        finish(&a, &token, &second, p).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
#[tokio::test]
async fn organizer_respects_archived_lanes_terminal_work_and_epic_containers() {
    let a = TestApp::spawn().await;
    let t = a.create_ticket("Unassigned work").await;
    let archived = a.create_ticket("Archived lane work").await;
    let epic = a.create_typed("Epic", "epic", None).await;
    let (_, lane) = a
        .post(
            &a.worker,
            "/v1/projects/tp/lanes",
            json!({"title":"Old lane"}),
        )
        .await;
    let lid = lane["id"].as_str().unwrap();
    a.client
        .put(format!("{}/v1/lanes/{lid}/tickets/{archived}", a.base))
        .bearer_auth(&a.worker)
        .send()
        .await
        .unwrap();
    a.patch(
        &a.worker,
        &format!("/v1/lanes/{lid}"),
        json!({"archived":true}),
    )
    .await;
    let terminal = a.create_ticket("Custom terminal work").await;
    let db = rusqlite::Connection::open(a.db_path()).unwrap();
    db.execute("INSERT INTO workflow_states(project,state,category,terminal) VALUES('tp','retired','backlog',1)",[]).unwrap();
    db.execute(
        "UPDATE tickets SET state='retired' WHERE id=?1",
        [&terminal],
    )
    .unwrap();
    let v = send(&a, "one").await;
    let ids = v["jobs"][0]["snapshot"]["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(ids.contains(&t.as_str()));
    assert!(ids.contains(&archived.as_str()));
    assert!(!ids.contains(&epic.as_str()));
    assert!(!ids.contains(&terminal.as_str()));
}

#[tokio::test]
async fn organizer_snapshots_persisted_spec_in_tree_order_and_rejects_changed_prose() {
    let a = TestApp::spawn().await;
    let t = a.create_ticket("Clarify work").await;
    let token = worker(&a).await;
    let (_, map) = a
        .post(
            &a.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Project specification"}),
        )
        .await;
    let mid = map["mindmap"]["id"].as_str().unwrap();
    let mut node_id = String::new();
    for title in ["First section", "Second section", "Third section"] {
        let (s, n) = a
            .post(
                &a.worker,
                &format!("/v1/mindmaps/{mid}/nodes"),
                json!({"text":title,"notes":format!("Requirements for {title}")}),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED);
        node_id = n["nodes"][0]["id"].as_str().unwrap().into();
    }
    let v = send(&a, "one").await;
    let sections = v["jobs"][0]["snapshot"]["specifications"][0]["sections"]
        .as_array()
        .unwrap();
    assert_eq!(sections.len(), 3);
    assert!(sections
        .iter()
        .all(|s| s["body"].as_str().unwrap().contains("Requirements for")));
    let j = claim(&a, &token).await;
    let p = json!({"groups":[],"unassigned":[{"ticket_id":t,"reason":"Need clarification"}]});
    assert_eq!(finish(&a, &token, &j, p.clone()).await.0, StatusCode::OK);
    let path = format!(
        "/v1/projects/tp/lane-organizer/jobs/{}/accept",
        j["id"].as_str().unwrap()
    );
    assert_eq!(a.post(&a.human, &path, json!({})).await.0, StatusCode::OK);
    send(&a, "two").await;
    let j = claim(&a, &token).await;
    assert_eq!(finish(&a, &token, &j, p).await.0, StatusCode::OK);
    let (s, v) = a
        .patch(
            &a.worker,
            &format!("/v1/mindmaps/{mid}/nodes/{node_id}"),
            json!({"notes":"Changed saved requirements"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let path = format!(
        "/v1/projects/tp/lane-organizer/jobs/{}/accept",
        j["id"].as_str().unwrap()
    );
    assert_eq!(
        a.post(&a.human, &path, json!({})).await.0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn organizer_acceptance_rolls_back_all_groups_if_a_later_lane_is_full() {
    let a = TestApp::spawn().await;
    let first = a.create_ticket("First pending").await;
    let second = a.create_ticket("Second pending").await;
    let (_, lane) = a
        .post(
            &a.worker,
            "/v1/projects/tp/lanes",
            json!({"title":"Existing","purpose":"Existing purpose"}),
        )
        .await;
    let lid = lane["id"].as_str().unwrap();
    let db = rusqlite::Connection::open(a.db_path()).unwrap();
    for index in 0..200 {
        let t = a.create_ticket(&format!("Existing {index}")).await;
        db.execute(
            "INSERT INTO work_lane_tickets VALUES(?1,?2)",
            rusqlite::params![lid, t],
        )
        .unwrap();
    }
    send(&a, "one").await;
    let token = worker(&a).await;
    let j = claim(&a, &token).await;
    let mut p = proposal(&first);
    p["groups"].as_array_mut().unwrap().push(json!({"lane_id":lid,"title":"Existing","purpose":"Existing purpose","context":"Enriched context","readiness":"ready","reason":"Related work","ticket_ids":[second]}));
    assert_eq!(finish(&a, &token, &j, p).await.0, StatusCode::OK);
    let path = format!(
        "/v1/projects/tp/lane-organizer/jobs/{}/accept",
        j["id"].as_str().unwrap()
    );
    assert_eq!(
        a.post(&a.human, &path, json!({})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        a.get(&a.worker, "/v1/projects/tp/lanes").await.1["total"],
        1
    );
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM work_lane_tickets WHERE ticket IN (?1,?2)",
            rusqlite::params![first, second],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn organizer_expiry_is_visible_without_mutating_get_and_lane_context_is_fenced() {
    let a = TestApp::spawn_without_sweeper().await;
    let t = a.create_ticket("Pending").await;
    let token = worker(&a).await;
    let (_, lane) = a
        .post(
            &a.worker,
            "/v1/projects/tp/lanes",
            json!({"title":"Existing","purpose":"Purpose","context":"Original context"}),
        )
        .await;
    let lid = lane["id"].as_str().unwrap();
    send(&a, "one").await;
    let job = claim(&a, &token).await;
    let db = rusqlite::Connection::open(a.db_path()).unwrap();
    db.execute(
        "UPDATE agent_jobs SET lease_expires_at=0 WHERE id=?1",
        [job["id"].as_str().unwrap()],
    )
    .unwrap();
    let (s, v) = a.get(&a.worker, "/v1/projects/tp/lane-organizer").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["jobs"][0]["status"], "failed");
    let status: String = db
        .query_row(
            "SELECT status FROM agent_jobs WHERE id=?1",
            [job["id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "running");
    send(&a, "two").await;
    let j = claim(&a, &token).await;
    let mut p = proposal(&t);
    p["groups"][0]["lane_id"] = json!(lid);
    p["groups"][0]["title"] = json!("Existing");
    p["groups"][0]["purpose"] = json!("Purpose");
    assert_eq!(finish(&a, &token, &j, p).await.0, StatusCode::OK);
    a.patch(
        &a.worker,
        &format!("/v1/lanes/{lid}"),
        json!({"context":"Human correction"}),
    )
    .await;
    let path = format!(
        "/v1/projects/tp/lane-organizer/jobs/{}/accept",
        j["id"].as_str().unwrap()
    );
    assert_eq!(
        a.post(&a.human, &path, json!({})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        a.get(&a.worker, &format!("/v1/lanes/{lid}")).await.1["context"],
        "Human correction"
    );
}
