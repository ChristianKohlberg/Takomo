mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};
async fn fixture(app: &TestApp) -> (String, String, String, String) {
    let map = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Requirements"}),
        )
        .await
        .1["mindmap"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let nodes=app.post(&app.human,&format!("/v1/mindmaps/{map}/nodes"),json!({"nodes":[{"text":"Payment retry handling","notes":"A retried payment must create exactly one charge."},{"text":"Receipt delivery","notes":"Every completed purchase must send one receipt."}]})).await.1;
    let first = nodes["nodes"][0]["id"].as_str().unwrap().to_string();
    let second = nodes["nodes"][1]["id"].as_str().unwrap().to_string();
    let (status,t)=app.post(&app.human,"/v1/tickets",json!({"project":"tp","title":"Payment retry handling","body":"A retried payment must create exactly one charge."})).await;
    assert_eq!(status, StatusCode::CREATED, "{t}");
    (
        map,
        first,
        second,
        t["ticket"]["id"]
            .as_str()
            .unwrap_or_else(|| t["id"].as_str().unwrap())
            .to_string(),
    )
}
#[tokio::test]
async fn manual_references_are_reviewed_durable_filterable_and_not_dependencies() {
    let app = TestApp::spawn_without_sweeper().await;
    let (map, first, second, ticket) = fixture(&app).await;
    let path = format!("/v1/tickets/{ticket}/document-links");
    let reader = app.mint("reader", &["read"], Some(&["tp"]));
    let foreign = app.mint("foreign", &["read", "write", "human"], Some(&["other"]));
    for token in [&reader, &app.worker, &foreign] {
        assert_eq!(
            app.post(token, &path, json!({"section_id":first})).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        app.post(
            &app.human,
            &path,
            json!({"section_id":first,"relation":"source"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, linked) = app
        .post(
            &app.human,
            &path,
            json!({"section_id":first,"primary":true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    let id = linked["links"][0]["id"].as_str().unwrap();
    assert_eq!(linked["links"][0]["provenance"], "manual");
    assert_eq!(linked["links"][0]["state"], "accepted");
    let fetched = app.get(&reader, &format!("/v1/tickets/{ticket}")).await.1;
    let t = fetched.get("ticket").unwrap_or(&fetched);
    assert_eq!(t["document_refs"][0]["section_id"], first);
    assert_eq!(t["blocked_by"], json!([]));
    assert!(t["parent"].is_null());
    let list = app
        .get(
            &reader,
            &format!("/v1/tickets?project=tp&document_section={first}"),
        )
        .await
        .1;
    assert_eq!(list["items"].as_array().unwrap().len(), 1, "{list}");
    let reverse = app
        .get(
            &reader,
            &format!("/v1/projects/tp/document-links?section_id={first}&limit=1&offset=0"),
        )
        .await
        .1;
    assert_eq!(reverse["total"], 1);
    assert_eq!(reverse["items"][0]["ticket"], ticket);
    app.post(
        &app.human,
        &path,
        json!({"section_id":second,"primary":true}),
    )
    .await;
    let rows = app.get(&reader, &path).await.1;
    assert_eq!(
        rows["links"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["primary"] == true)
            .count(),
        1
    );
    app.patch(
        &app.human,
        &format!("/v1/mindmaps/{map}/nodes/{first}"),
        json!({"text":"Renamed requirement"}),
    )
    .await;
    let rows = app.get(&reader, &path).await.1;
    let row = rows["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap();
    assert_eq!(row["title"], "Renamed requirement");
    assert_eq!(row["captured_title"], "Payment retry handling");
    app.delete(&app.human, &format!("/v1/mindmaps/{map}/nodes/{first}"))
        .await;
    let rows = app.get(&reader, &path).await.1;
    assert!(rows["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap()["missing"]
        .as_bool()
        .unwrap());
    let list = app
        .get(
            &reader,
            &format!("/v1/tickets?project=tp&document_section={first}"),
        )
        .await
        .1;
    assert!(list["items"].as_array().unwrap().is_empty());
    let removed = app.delete(&app.human, &format!("{path}/{id}")).await;
    assert_eq!(removed.0, StatusCode::OK, "{}", removed.1);
    let row = removed.1["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap();
    assert_eq!(row["state"], "removed");
    assert!(!row["reviewed_by"].is_null());
}
#[tokio::test]
async fn automatic_outbox_coalesces_material_edits_and_does_not_block_missing_documents() {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, _, _, ticket) = fixture(&app).await;
    let runner = app.mint("classifier", &["agent:run"], Some(&["tp"]));
    assert_eq!(app.open_store().sweep_ticket_classification().unwrap(), 1);
    assert_eq!(app.open_store().sweep_ticket_classification().unwrap(), 0);
    assert!(app
        .post(
            &runner,
            "/v1/agent-jobs/claim",
            json!({"service_id":"old","supported_kinds":["document_workspace"]})
        )
        .await
        .1["job"]
        .is_null());
    app.patch(
        &app.human,
        &format!("/v1/tickets/{ticket}"),
        json!({"title":"Changed payment details"}),
    )
    .await;
    assert_eq!(app.open_store().sweep_ticket_classification().unwrap(), 1);
    let job = app
        .post(
            &runner,
            "/v1/agent-jobs/claim",
            json!({"service_id":"classifier","supported_kinds":["ticket_document_classify"]}),
        )
        .await
        .1["job"]
        .clone();
    let snapshot: Value = serde_json::from_str(job["snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(snapshot["ticket"]["title"], "Changed payment details");
    assert_eq!(job["ticket_id"], ticket);
    app.patch(
        &app.human,
        &format!("/v1/tickets/{ticket}"),
        json!({"title":"Newer payment details"}),
    )
    .await;
    assert_eq!(
        app.open_store().sweep_ticket_classification().unwrap(),
        0,
        "must wait for active attempt"
    );
    let result=app.post(&runner,&format!("/v1/agent-jobs/{}/result",job["id"].as_str().unwrap()),json!({"service_id":"classifier","attempt_id":job["attempt_id"],"status":"failed","error":"Fixture failure"})).await;
    assert_eq!(result.0, StatusCode::OK, "{}", result.1);
    assert_eq!(app.open_store().sweep_ticket_classification().unwrap(), 1);
    app.post(
        &app.admin,
        "/v1/projects",
        json!({"id":"empty","name":"No document"}),
    )
    .await;
    let (status, t) = app
        .post(
            &app.human,
            "/v1/tickets",
            json!({"project":"empty","title":"Work without a specification"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{t}");
    app.open_store().sweep_ticket_classification().unwrap();
    let id = t["id"]
        .as_str()
        .or_else(|| t["ticket"]["id"].as_str())
        .unwrap();
    let view = app
        .get(&app.human, &format!("/v1/tickets/{id}/document-links"))
        .await
        .1;
    assert_eq!(view["classification"]["status"], "unavailable");
}
#[tokio::test]
async fn project_policy_is_opt_in_and_references_never_add_a_workflow_guard() {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, _, _, ticket) = fixture(&app).await;
    let cfg = "/v1/projects/tp/document-classification-config";
    assert_eq!(app.get(&app.human, cfg).await.1["mode"], "suggest");
    assert_eq!(
        app.client
            .put(format!("{}{cfg}", app.base))
            .bearer_auth(&app.human)
            .json(&json!({"mode":"auto_apply_clear"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let changed = app
        .client
        .put(format!("{}{cfg}", app.base))
        .bearer_auth(&app.admin)
        .json(&json!({"mode":"auto_apply_clear"}))
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    assert_eq!(app.get(&app.human, cfg).await.1["mode"], "auto_apply_clear");
    let (status, backfill) = app
        .post(
            &app.human,
            "/v1/projects/tp/document-classification",
            json!({"request_id":"backfill"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{backfill}");
    let wf = app
        .get(&app.human, "/v1/projects/tp/workflow")
        .await
        .1
        .to_string();
    assert!(!wf.contains("has_document_reference"));
    assert!(app
        .get(&app.human, &format!("/v1/tickets/{ticket}"))
        .await
        .1
        .to_string()
        .contains(&ticket));
}
