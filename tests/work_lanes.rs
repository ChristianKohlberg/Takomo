mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
async fn lane(a: &TestApp) -> String {
    let(s,v)=a.post(&a.worker,"/v1/projects/tp/lanes",json!({"title":"Editor","purpose":"Related editor work","context":"Keep existing decisions"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["id"].as_str().unwrap().into()
}
async fn runner(a: &TestApp, actor: &str) -> String {
    a.open_store()
        .create_token(
            actor,
            &["read", "write", "agent:run"].map(str::to_string),
            None,
            10000,
            None,
            None,
        )
        .unwrap()
        .1
}
async fn attach(a: &TestApp, l: &str, t: &str) -> StatusCode {
    a.client
        .put(format!("{}/v1/lanes/{l}/tickets/{t}", a.base))
        .bearer_auth(&a.worker)
        .send()
        .await
        .unwrap()
        .status()
}
async fn draft(a: &TestApp, l: &str, t: &str, kind: &str, extra: Value) -> Value {
    let mut body = json!({"kind":kind,"provider":"codex","instructions":"Complete this assignment","ticket_ids":[t]});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let (s, v) = a
        .post(&a.worker, &format!("/v1/lanes/{l}/handoffs"), body)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}
#[tokio::test]
async fn lanes_scope_snapshot_human_dispatch_and_review_return() {
    let a = TestApp::spawn().await;
    let l = lane(&a).await;
    let t = a.create_ticket("Preserve selection").await;
    assert_eq!(attach(&a, &l, &t).await, StatusCode::OK);
    let d = draft(&a, &l, &t, "implementation", json!({})).await;
    let id = d["id"].as_str().unwrap();
    let path = format!("/v1/handoffs/{id}");
    a.patch(
        &a.worker,
        &format!("/v1/lanes/{l}"),
        json!({"context":"New context"}),
    )
    .await;
    assert_eq!(
        a.get(&a.worker, &path).await.1["snapshot"]["lane"]["context"],
        "Keep existing decisions"
    );
    assert_eq!(
        a.post(&a.worker, &format!("{path}/dispatch"), json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        a.post(&a.human, &format!("{path}/dispatch"), json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        a.post(&a.worker, &format!("{path}/claim"), json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let token = runner(&a, "agent:runner").await;
    let (s, claim) = a.post(&token, &format!("{path}/claim"), json!({})).await;
    assert_eq!(s, StatusCode::OK, "{claim}");
    let(s,v)=a.post(&token,&format!("{path}/result"),json!({"attempt":claim["attempt"],"status":"completed","result":"Implemented","revision":"abc123"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let review = draft(
        &a,
        &l,
        &t,
        "review",
        json!({"parent_handoff":id,"target_revision":"abc123"}),
    )
    .await;
    let rid = review["id"].as_str().unwrap();
    let rpath = format!("/v1/handoffs/{rid}");
    a.post(&a.human, &format!("{rpath}/dispatch"), json!({}))
        .await;
    let (_, c) = a.post(&token, &format!("{rpath}/claim"), json!({})).await;
    assert_eq!(a.post(&token,&format!("{rpath}/result"),json!({"attempt":c["attempt"],"status":"completed","result":"Finding: keyboard focus is lost"})).await.0,StatusCode::OK);
    let (_, history) = a.get(&a.worker, &format!("/v1/lanes/{l}/handoffs")).await;
    assert_eq!(history["total"], 2);
    assert!(history["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["result"] == "Finding: keyboard focus is lost"
            && h["lane"] == l
            && h["target_revision"] == "abc123"));
    let(s,_)=a.post(&a.worker,&format!("/v1/lanes/{l}/handoffs"),json!({"kind":"review","provider":"codex","instructions":"Review","ticket_ids":[t],"parent_handoff":id,"target_revision":"wrong"})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, restricted) = a
        .open_store()
        .create_token(
            "agent:other",
            &["read", "write"].map(str::to_string),
            Some(&["elsewhere".into()]),
            10000,
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        a.get(&restricted, &format!("/v1/lanes/{l}")).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(a.get(&restricted, &path).await.0, StatusCode::FORBIDDEN);
    a.post(
        &a.admin,
        "/v1/projects",
        json!({"id":"other","name":"Other"}),
    )
    .await;
    let other = a.create_ticket_in("other", "Wrong project").await;
    assert_eq!(attach(&a, &l, &other).await, StatusCode::CONFLICT);
}
#[tokio::test]
async fn handoff_attempts_cancel_and_context_edits_are_fenced() {
    let a = TestApp::spawn().await;
    let l = lane(&a).await;
    let t = a.create_ticket("Fix").await;
    attach(&a, &l, &t).await;
    let d = draft(&a, &l, &t, "preparation", json!({})).await;
    let id = d["id"].as_str().unwrap();
    let p = format!("/v1/handoffs/{id}");
    let token = runner(&a, "agent:runner").await;
    let token2 = runner(&a, "agent:other").await;
    a.post(&a.human, &format!("{p}/dispatch"), json!({})).await;
    let (_, c1) = a.post(&token, &format!("{p}/claim"), json!({})).await;
    assert_eq!(
        a.post(
            &token2,
            &format!("{p}/heartbeat"),
            json!({"attempt":c1["attempt"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let db = rusqlite::Connection::open(a.db_path()).unwrap();
    db.execute("UPDATE work_handoffs SET lease_until=0 WHERE id=?1", [id])
        .unwrap();
    assert_eq!(
        a.get(&token, "/v1/projects/tp/handoffs?status=ready")
            .await
            .1["total"],
        1
    );
    let (_, c2) = a.post(&token, &format!("{p}/claim"), json!({})).await;
    assert_eq!(c2["attempt"], 2);
    assert_eq!(
        a.post(
            &token,
            &format!("{p}/result"),
            json!({"attempt":c1["attempt"],"status":"completed","result":"Stale"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    a.patch(
        &a.worker,
        &format!("/v1/lanes/{l}"),
        json!({"context":"Human correction"}),
    )
    .await;
    let (s, v) = a
        .post(
            &token,
            &format!("{p}/result"),
            json!({"attempt":c2["attempt"],"status":"completed","result":"Proposed context"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["context_applied"], false);
    assert_eq!(
        a.get(&token, &format!("/v1/lanes/{l}")).await.1["context"],
        "Human correction"
    );
    let d = draft(&a, &l, &t, "implementation", json!({})).await;
    let p = format!("/v1/handoffs/{}", d["id"].as_str().unwrap());
    a.post(&a.human, &format!("{p}/dispatch"), json!({})).await;
    let (_, c) = a.post(&token, &format!("{p}/claim"), json!({})).await;
    a.post(&a.human, &format!("{p}/cancel"), json!({})).await;
    assert_eq!(
        a.post(
            &token,
            &format!("{p}/heartbeat"),
            json!({"attempt":c["attempt"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn lane_validation_and_bounded_lists() {
    let a = TestApp::spawn().await;
    let l = lane(&a).await;
    lane(&a).await;
    let (_, v) = a.get(&a.worker, "/v1/projects/tp/lanes?limit=1").await;
    assert_eq!(v["total"], 2);
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert!(v["note"].is_string());
    assert_eq!(
        a.patch(&a.worker, &format!("/v1/lanes/{l}"), json!({"title":null}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        a.post(
            &a.worker,
            &format!("/v1/lanes/{l}/handoffs"),
            json!({"kind":"implementation","provider":"codex","instructions":"x","ticket_ids":[]})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    a.patch(
        &a.worker,
        &format!("/v1/lanes/{l}"),
        json!({"archived":true}),
    )
    .await;
    let t = a.create_ticket("Fix").await;
    assert_eq!(attach(&a, &l, &t).await, StatusCode::CONFLICT);
    a.patch(
        &a.worker,
        &format!("/v1/lanes/{l}"),
        json!({"archived":false}),
    )
    .await;
    assert_eq!(attach(&a, &l, &t).await, StatusCode::OK);
    assert_eq!(
        a.client
            .delete(format!("{}/v1/lanes/{l}/tickets/{t}", a.base))
            .bearer_auth(&a.worker)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        a.get(&a.worker, &format!("/v1/lanes/{l}")).await.1["tickets"],
        json!([])
    );
}

#[tokio::test]
async fn preparation_returns_context_and_snapshots_only_selected_work() {
    let a = TestApp::spawn().await;
    let l = lane(&a).await;
    let t = a.create_ticket("Selected").await;
    let other = a.create_ticket("Outside assignment").await;
    attach(&a, &l, &t).await;
    attach(&a, &l, &other).await;
    let d = draft(&a, &l, &t, "preparation", json!({})).await;
    assert_eq!(d["snapshot"]["tickets"].as_array().unwrap().len(), 1);
    assert!(d["snapshot"]["lane"].get("tickets").is_none());
    let p = format!("/v1/handoffs/{}", d["id"].as_str().unwrap());
    let token = runner(&a, "agent:runner").await;
    a.post(&a.human, &format!("{p}/dispatch"), json!({})).await;
    let (_, c) = a.post(&token, &format!("{p}/claim"), json!({})).await;
    assert_eq!(
        a.post(
            &token,
            &format!("{p}/heartbeat"),
            json!({"attempt":c["attempt"]})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, result) = a.post(&token, &format!("{p}/result"), json!({"attempt":c["attempt"],"status":"completed","result":"Prepared context","conversation_ref":"codex:session-1"})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(result["context_applied"], true);
    let (_, lane) = a.get(&token, &format!("/v1/lanes/{l}")).await;
    assert_eq!(lane["context"], "Prepared context");
    assert_eq!(lane["conversation_ref"], "codex:session-1");
    assert_eq!(lane["tickets"].as_array().unwrap().len(), 2);
}
