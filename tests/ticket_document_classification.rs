mod common;
use common::TestApp;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

struct Fixture {
    app: TestApp,
    ticket: String,
    section: String,
    map: String,
    runner: String,
}
async fn fixture() -> Fixture {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, map) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({"project":"tp","title":"Billing"}),
        )
        .await;
    let map = map["mindmap"]["id"].as_str().unwrap().to_owned();
    let (_, section) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({"text":"Invoice expiration","notes":"Invoices expire after 30 days."}),
        )
        .await;
    let section = section["nodes"][0]["id"].as_str().unwrap().to_owned();
    let (status,ticket) = app.post(&app.worker,"/v1/tickets",json!({"project":"tp","type":"task","title":"Invoice expiration","body":"Implement the specified invoice deadline."})).await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    let ticket = ticket["id"].as_str().unwrap().to_owned();
    let runner = app.mint("agent:classifier", &["agent:run"], Some(&["tp"]));
    Fixture {
        app,
        ticket,
        section,
        map,
        runner,
    }
}
async fn enqueue(f: &Fixture, request: &str) -> Value {
    let (status, result) = f
        .app
        .post(
            &f.app.human,
            &format!("/v1/tickets/{}/document-classification", f.ticket),
            json!({"request_id":request}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert!(result["job_id"].is_string(), "{result}");
    result
}
async fn claim(f: &Fixture) -> Value {
    let (status, result) = f
        .app
        .post(
            &f.runner,
            "/v1/agent-jobs/claim",
            json!({"service_id":"classifier","supported_kinds":["ticket_document_classify"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert!(result["job"].is_object(), "{result}");
    result["job"].clone()
}
fn result(job: &Value, section: &str) -> Value {
    let snapshot: Value = serde_json::from_str(job["snapshot"].as_str().unwrap()).unwrap();
    let source = snapshot["document"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == section)
        .unwrap();
    json!({"service_id":"classifier","attempt_id":job["attempt_id"],"status":"completed","thread_id":format!("thread-{}",job["id"].as_str().unwrap()),"turn_id":"turn","message":"Suggested one source for review.","proposal":{"candidates":[{"section_id":section,"version":source["version"],"quote":"Invoices expire after 30 days.","rationale":"The section defines the requested deadline."}],"ambiguity":null,"no_match_reason":null},"evidence":{"document":{"sources":[{"section_id":section,"version":source["version"]}],"coverage":{"read_section_ids":[section],"total_sections":1,"complete":true}}}})
}
async fn finish(f: &Fixture, job: &Value, body: Value) -> (StatusCode, Value) {
    f.app
        .post(
            &f.runner,
            &format!("/v1/agent-jobs/{}/result", job["id"].as_str().unwrap()),
            body,
        )
        .await
}
async fn links(f: &Fixture) -> Value {
    let (status, value) = f
        .app
        .get(
            &f.app.human,
            &format!("/v1/tickets/{}/document-links", f.ticket),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}
async fn mode(f: &Fixture, value: &str) {
    let response = f
        .app
        .request(
            Method::PUT,
            "/v1/projects/tp/document-classification-config",
        )
        .bearer_auth(&f.app.admin)
        .json(&json!({"mode":value}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn classification_is_capability_filtered_and_captures_immutable_ticket_sources() {
    let f = fixture().await;
    enqueue(&f, "one").await;
    let (status, old) = f
        .app
        .post(
            &f.runner,
            "/v1/agent-jobs/claim",
            json!({"service_id":"old"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(old["job"].is_null(), "{old}");
    let job = claim(&f).await;
    assert_eq!(job["kind"], "ticket_document_classify");
    assert!(job["thread_id"].is_null());
    let snapshot: Value = serde_json::from_str(job["snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(snapshot["ticket"]["id"], f.ticket);
    assert_eq!(snapshot["document"]["mindmap_id"], f.map);
    assert_eq!(
        snapshot["document"]["sections"][0]["notes"],
        "Invoices expire after 30 days."
    );
    assert_eq!(
        snapshot["document"]["sections"][0]["version"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        finish(&f, &job, result(&job, &f.section)).await.0,
        StatusCode::OK
    );
    let current = links(&f).await;
    assert_eq!(current["links"][0]["state"], "suggested");
    assert_eq!(current["links"][0]["provenance"], "automatic");
}

#[tokio::test]
async fn classification_rejects_malformed_candidates_and_unobserved_sources_atomically() {
    let f = fixture().await;
    enqueue(&f, "bad-output").await;
    let job = claim(&f).await;
    for case in [
        "unknown",
        "version",
        "quote",
        "unseen",
        "duplicate",
        "oversized",
        "confidence",
        "empty",
    ] {
        let mut body = result(&job, &f.section);
        match case {
            "unknown" => body["proposal"]["candidates"][0]["section_id"] = json!("invented"),
            "version" => body["proposal"]["candidates"][0]["version"] = json!("0".repeat(64)),
            "quote" => body["proposal"]["candidates"][0]["quote"] = json!("Invented source quote"),
            "unseen" => body["evidence"]["document"]["sources"] = json!([]),
            "duplicate" => {
                let candidate = body["proposal"]["candidates"][0].clone();
                body["proposal"]["candidates"]
                    .as_array_mut()
                    .unwrap()
                    .push(candidate);
            }
            "oversized" => {
                body["proposal"]["candidates"][0]["rationale"] = json!("x".repeat(24_001))
            }
            "confidence" => body["proposal"]["confidence"] = json!(0.99),
            _ => body["proposal"]["candidates"] = json!([]),
        }
        let (status, error) = finish(&f, &job, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{case}: {error}");
        assert!(links(&f).await["links"].as_array().unwrap().is_empty());
    }
    assert_eq!(
        finish(&f, &job, result(&job, &f.section)).await.0,
        StatusCode::OK
    );
    assert_eq!(links(&f).await["links"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn no_match_and_failure_are_distinct_and_explicit_retry_uses_a_new_thread() {
    let f = fixture().await;
    enqueue(&f, "no-match").await;
    let job = claim(&f).await;
    let mut body = result(&job, &f.section);
    body["proposal"] = json!({"candidates":[],"ambiguity":null,"no_match_reason":"No relevant section describes the ticket."});
    body["evidence"]["document"]["sources"] = json!([]);
    body["evidence"]["document"]["coverage"] =
        json!({"read_section_ids":[],"total_sections":1,"complete":false});
    assert_eq!(finish(&f, &job, body).await.0, StatusCode::OK);
    assert_eq!(links(&f).await["classification"]["status"], "no_match");
    enqueue(&f, "explicit-retry").await;
    let retry = claim(&f).await;
    assert_ne!(retry["id"], job["id"]);
    assert_ne!(retry["conversation_id"], job["conversation_id"]);
    assert!(retry["thread_id"].is_null());
    let failure = json!({"service_id":"classifier","attempt_id":retry["attempt_id"],"status":"failed","error":"Provider unavailable"});
    assert_eq!(finish(&f, &retry, failure).await.0, StatusCode::OK);
    let current = links(&f).await;
    assert_eq!(current["classification"]["status"], "failed");
    assert!(current["links"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn document_links_require_scoped_human_writes_and_cannot_forge_direct_origin() {
    let f = fixture().await;
    let path = format!("/v1/tickets/{}/document-links", f.ticket);
    let other = f
        .app
        .mint("human:other", &["read", "write", "human"], Some(&["other"]));
    assert_eq!(f.app.get(&other, &path).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        f.app
            .post(&f.app.worker, &path, json!({"section_id":f.section}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.app
            .post(
                &f.app.worker,
                &format!("/v1/tickets/{}/document-classification", f.ticket),
                json!({"request_id":"denied"})
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for forbidden in ["relation", "provenance", "state"] {
        let mut body = json!({"section_id":f.section});
        body[forbidden] = json!("source");
        assert_eq!(
            f.app.post(&f.app.human, &path, body).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert!(links(&f).await["links"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn stale_ticket_revision_cannot_gain_an_automatic_or_suggested_relationship() {
    let f = fixture().await;
    mode(&f, "auto_apply_clear").await;
    enqueue(&f, "stale").await;
    let job = claim(&f).await;
    assert_eq!(
        f.app
            .patch_with(
                &f.app.worker,
                &format!("/v1/tickets/{}", f.ticket),
                &[("If-Match", "\"1\"")],
                json!({"body":"The requested deadline has changed."})
            )
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        finish(&f, &job, result(&job, &f.section)).await.0,
        StatusCode::OK
    );
    let current = links(&f).await;
    assert_eq!(current["classification"]["status"], "queued");
    let stale: bool = rusqlite::Connection::open(f.app.db_path())
        .unwrap()
        .query_row(
            "SELECT stale FROM ticket_document_jobs WHERE job=?1",
            [job["id"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        stale,
        "The old result must remain marked stale while the latest revision awaits classification"
    );
    assert!(current["links"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn result_uses_current_project_policy_and_preserves_manual_links_on_reclassification() {
    let f = fixture().await;
    mode(&f, "auto_apply_clear").await;
    enqueue(&f, "policy").await;
    let job = claim(&f).await;
    mode(&f, "suggest").await;
    assert_eq!(
        finish(&f, &job, result(&job, &f.section)).await.0,
        StatusCode::OK
    );
    assert_eq!(links(&f).await["links"][0]["state"], "suggested");
    assert_eq!(
        f.app
            .post(
                &f.app.human,
                &format!("/v1/tickets/{}/document-links", f.ticket),
                json!({"section_id":f.section,"primary":true})
            )
            .await
            .0,
        StatusCode::OK
    );
    enqueue(&f, "reclassify-with-manual").await;
    let retry = claim(&f).await;
    assert_eq!(
        finish(&f, &retry, result(&retry, &f.section)).await.0,
        StatusCode::OK
    );
    let current = links(&f).await;
    let accepted: Vec<_> = current["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|link| link["state"] == "accepted")
        .collect();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["provenance"], "manual");
    assert_eq!(accepted[0]["relation"], "related");
}

#[tokio::test]
async fn a_fresh_no_match_retires_prior_suggestions_without_recording_a_human_dismissal() {
    let f = fixture().await;
    enqueue(&f, "initial-match").await;
    let first = claim(&f).await;
    assert_eq!(
        finish(&f, &first, result(&first, &f.section)).await.0,
        StatusCode::OK
    );
    enqueue(&f, "reconsider-match").await;
    let second = claim(&f).await;
    let mut body = result(&second, &f.section);
    body["proposal"] = json!({"candidates":[],"ambiguity":null,"no_match_reason":"The ticket does not require this section after closer review."});
    assert_eq!(finish(&f, &second, body).await.0, StatusCode::OK);
    let current = links(&f).await;
    assert_eq!(current["classification"]["status"], "no_match");
    assert!(current["links"]
        .as_array()
        .unwrap()
        .iter()
        .all(|link| link["state"] != "suggested"));
    let reviewed: i64 = rusqlite::Connection::open(f.app.db_path()).unwrap().query_row(
        "SELECT COUNT(*) FROM ticket_document_links WHERE ticket=?1 AND reviewed_by IS NOT NULL",
        [&f.ticket], |row| row.get(0),
    ).unwrap();
    assert_eq!(reviewed, 0, "automatic retirement is not a human dismissal");
}

#[tokio::test]
async fn automatic_replacement_does_not_suppress_the_same_source_on_later_document_revisions() {
    let f = fixture().await;
    let (_, added) = f
        .app
        .post(
            &f.app.worker,
            &format!("/v1/mindmaps/{}/nodes", f.map),
            json!({"text":"Unrelated section","notes":"First revision."}),
        )
        .await;
    let unrelated = added["nodes"][0]["id"].as_str().unwrap();
    for revision in 0..3 {
        if revision > 0 {
            assert_eq!(
                f.app
                    .patch(
                        &f.app.worker,
                        &format!("/v1/mindmaps/{}/nodes/{unrelated}", f.map),
                        json!({"notes":format!("Unrelated revision {revision}")})
                    )
                    .await
                    .0,
                StatusCode::OK
            );
        }
        // Explicit backfill schedules the normal automatic classifier path:
        // reconsider remains false, while only the document revision changes.
        let (status, _) = f
            .app
            .post(
                &f.app.human,
                "/v1/projects/tp/document-classification",
                json!({"request_id":format!("scan-{revision}")}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 1);
        let job = claim(&f).await;
        let mut body = result(&job, &f.section);
        body["evidence"]["document"]["coverage"] =
            json!({"read_section_ids":[f.section],"total_sections":2,"complete":false});
        assert_eq!(finish(&f, &job, body).await.0, StatusCode::OK);
        let current = links(&f).await;
        let active: Vec<_> = current["links"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|link| link["state"] == "suggested")
            .collect();
        assert_eq!(active.len(), 1, "revision {revision}: {current}");
        assert_eq!(active[0]["section_id"], f.section);
    }
}

#[tokio::test]
async fn moving_a_ticket_during_classification_cannot_publish_or_leak_original_project_evidence() {
    for outcome in ["candidate", "no_match", "failed"] {
        let f = fixture().await;
        let (status, project) = f
            .app
            .post(
                &f.app.admin,
                "/v1/projects",
                json!({"id":"beta","name":"Destination"}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{project}");
        let reader = f.app.mint("human:destination", &["read"], Some(&["beta"]));
        assert_eq!(
            f.app
                .patch(
                    &f.app.worker,
                    &format!("/v1/mindmaps/{}/nodes/{}", f.map, f.section),
                    json!({"text":"PRIVATE_SOURCE_TITLE"})
                )
                .await
                .0,
            StatusCode::OK
        );
        enqueue(&f, "moving-ticket").await;
        let job = claim(&f).await;
        let mut body = result(&job, &f.section);
        body["proposal"]["ambiguity"] = json!("PRIVATE_PROPOSAL_DETAIL");
        if outcome == "no_match" {
            body["proposal"] = json!({"candidates":[],"ambiguity":"PRIVATE_PROPOSAL_DETAIL","no_match_reason":"PRIVATE_NO_MATCH_DETAIL"});
        } else if outcome == "failed" {
            body = json!({"service_id":"classifier","attempt_id":job["attempt_id"],"status":"failed","error":"PRIVATE_PROVIDER_DETAIL"});
        }
        let (status, moved) = f
            .app
            .post(
                &f.app.admin,
                "/v1/tickets/move",
                json!({"tickets":[f.ticket],"to_project":"beta"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{moved}");
        let (status, completed) = finish(&f, &job, body).await;
        assert_eq!(status, StatusCode::OK, "{outcome}: {completed}");
        let (status, visible) = f
            .app
            .get(&reader, &format!("/v1/tickets/{}/document-links", f.ticket))
            .await;
        assert_eq!(status, StatusCode::OK, "{visible}");
        assert!(
            visible["links"].as_array().unwrap().is_empty(),
            "{outcome}: {visible}"
        );
        let text = visible.to_string();
        for private in [
            "PRIVATE_SOURCE_TITLE",
            "PRIVATE_PROPOSAL_DETAIL",
            "PRIVATE_NO_MATCH_DETAIL",
            "PRIVATE_PROVIDER_DETAIL",
            "Invoices expire after 30 days.",
        ] {
            assert!(
                !text.contains(private),
                "{outcome} leaked {private}: {visible}"
            );
        }
        assert_eq!(
            f.app
                .get(
                    &reader,
                    &format!("/v1/agent-jobs/{}", job["id"].as_str().unwrap())
                )
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let count: i64 = rusqlite::Connection::open(f.app.db_path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM ticket_document_links WHERE ticket=?1",
                [&f.ticket],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "old-project results must not create a destination-project relationship"
        );
    }
}

#[tokio::test]
async fn automatic_application_requires_a_unique_exact_section_title() {
    for duplicate in [false, true] {
        let f = fixture().await;
        if duplicate {
            let (status, added) = f.app.post(&f.app.worker, &format!("/v1/mindmaps/{}/nodes", f.map), json!({"text":"Invoice expiration","notes":"A competing section with the same heading."})).await;
            assert_eq!(status, StatusCode::CREATED, "{added}");
        }
        mode(&f, "auto_apply_clear").await;
        enqueue(&f, "auto-apply").await;
        let job = claim(&f).await;
        let mut body = result(&job, &f.section);
        if duplicate {
            body["evidence"]["document"]["coverage"] =
                json!({"read_section_ids":[f.section],"total_sections":2,"complete":false});
        }
        assert_eq!(finish(&f, &job, body).await.0, StatusCode::OK);
        let current = links(&f).await;
        let relevant: Vec<_> = current["links"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|link| link["state"] != "removed")
            .collect();
        assert_eq!(relevant.len(), 1, "{current}");
        assert_eq!(
            relevant[0]["state"],
            if duplicate { "suggested" } else { "accepted" }
        );
        assert_eq!(relevant[0]["primary"], !duplicate);
        assert_eq!(relevant[0]["provenance"], "automatic");
    }
}

#[tokio::test]
async fn deleting_a_ticket_removes_queued_and_running_classification_conversations() {
    for running in [false, true] {
        let f = fixture().await;
        let queued = enqueue(&f, "delete-origin").await;
        let job_id = queued["job_id"].as_str().unwrap();
        if running {
            assert_eq!(claim(&f).await["id"], job_id);
        }
        // There is no individual ticket DELETE endpoint. Keep the project alive
        // and exercise the actual ticket-delete trigger with foreign keys on.
        let connection = rusqlite::Connection::open(f.app.db_path()).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        let conversation: String = connection
            .query_row(
                "SELECT conversation_id FROM agent_jobs WHERE id=?1",
                [job_id],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute("DELETE FROM tickets WHERE id=?1", [&f.ticket])
            .unwrap();
        for (table, id) in [
            ("agent_jobs", job_id),
            ("agent_conversations", conversation.as_str()),
        ] {
            let remaining: i64 = connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE id=?1"),
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                remaining, 0,
                "orphaned {table} after deleting a ticket (running={running})"
            );
        }
        let (status, legacy) = f
            .app
            .post(
                &f.runner,
                "/v1/agent-jobs/claim",
                json!({"service_id":"legacy-after-delete"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{legacy}");
        assert!(
            legacy["job"].is_null(),
            "deleted classification must not fall back to section_chat: {legacy}"
        );
        assert_eq!(
            f.app
                .get(&f.app.human, &format!("/v1/agent-jobs/{job_id}"))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }
}

async fn scheduling(f: &Fixture, value: &str) -> Value {
    let response = f
        .app
        .client
        .put(f.app.url("/v1/projects/tp/document-classification-config"))
        .bearer_auth(&f.app.admin)
        .json(&json!({"mode":"suggest","scheduling":value}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.json().await.unwrap()
}

#[tokio::test]
async fn manual_cancels_automatic_backlog_but_allows_explicit_backfill() {
    let f = fixture().await;
    assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 1);
    let response = scheduling(&f, "manual").await;
    assert_eq!(response["cancelled"], 1);
    let (_, jobs) = f.app.get(&f.app.human, "/v1/agent-jobs?project=tp").await;
    assert_eq!(jobs["counts"]["queued"], 0);
    assert_eq!(jobs["counts"]["cancelled"], 1);
    let (_, filtered) = f
        .app
        .get(&f.app.human, "/v1/agent-jobs?project=tp&status=cancelled")
        .await;
    assert_eq!(filtered["total"], 1);
    let id = filtered["items"][0]["id"].as_str().unwrap();
    let (_, detail) = f
        .app
        .get(&f.app.human, &format!("/v1/agent-jobs/{id}"))
        .await;
    assert_eq!(detail["job"]["status"], "cancelled");
    assert_eq!(links(&f).await["classification"]["status"], "cancelled");
    let (status, _) = f
        .app
        .post(
            &f.app.worker,
            "/v1/tickets",
            json!({"project":"tp","type":"task","title":"Manual new ticket"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 0);
    let (status, body) = f
        .app
        .post(
            &f.app.human,
            "/v1/projects/tp/document-classification",
            json!({"request_id":"explicit-backfill"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["scheduled"], 2);
    assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 2);
    let (_, jobs) = f.app.get(&f.app.human, "/v1/agent-jobs?project=tp").await;
    assert_eq!(jobs["counts"]["queued"], 2);
    assert!(jobs["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|j| j["status"] == "queued")
        .all(|j| j["requested_by"] != "system:document-classifier"));
    assert_eq!(scheduling(&f, "manual").await["cancelled"], 0);
    assert_eq!(scheduling(&f, "off").await["cancelled"], 2);
}

#[tokio::test]
async fn off_blocks_requests_while_running_work_can_finish_and_reenable_does_not_backfill() {
    let f = fixture().await;
    enqueue(&f, "running").await;
    let job = claim(&f).await;
    assert_eq!(scheduling(&f, "off").await["cancelled"], 0);
    for path in [
        format!("/v1/tickets/{}/document-classification", f.ticket),
        "/v1/projects/tp/document-classification".into(),
    ] {
        let (status, body) = f
            .app
            .post(&f.app.human, &path, json!({"request_id":"disabled"}))
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }
    let (status, body) = finish(&f, &job, result(&job, &f.section)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(links(&f).await["links"].as_array().unwrap().len(), 1);
    f.app
        .post(
            &f.app.worker,
            "/v1/tickets",
            json!({"project":"tp","type":"task","title":"Created while off"}),
        )
        .await;
    assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 0);
    scheduling(&f, "automatic").await;
    assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 0);
    f.app
        .post(
            &f.app.worker,
            "/v1/tickets",
            json!({"project":"tp","type":"task","title":"Created while automatic"}),
        )
        .await;
    assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 1);
}

#[tokio::test]
async fn scheduling_requires_admin_and_legacy_policy_updates_preserve_it() {
    let f = fixture().await;
    scheduling(&f, "manual").await;
    mode(&f, "auto_apply_clear").await;
    let (_, cfg) = f
        .app
        .get(
            &f.app.human,
            "/v1/projects/tp/document-classification-config",
        )
        .await;
    assert_eq!(cfg["scheduling"], "manual");
    for (token, value, expected) in [
        (&f.app.human, "off", StatusCode::FORBIDDEN),
        (&f.app.admin, "invalid", StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let response = f
            .app
            .client
            .put(f.app.url("/v1/projects/tp/document-classification-config"))
            .bearer_auth(token)
            .json(&json!({"mode":"suggest","scheduling":value}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn legacy_database_upgrade_preserves_queue_and_defaults_and_reopens_cleanly() {
    let f = fixture().await;
    mode(&f, "auto_apply_clear").await;
    {
        let c = rusqlite::Connection::open(f.app.db_path()).unwrap();
        c.execute_batch("DROP TRIGGER ticket_document_created; DROP TRIGGER ticket_document_edited; ALTER TABLE ticket_document_settings DROP COLUMN scheduling; ALTER TABLE ticket_document_pending DROP COLUMN requested_by; ALTER TABLE ticket_document_jobs DROP COLUMN cancelled;").unwrap();
    }
    let store = f.app.open_store();
    assert_eq!(store.sweep_ticket_classification().unwrap(), 1);
    drop(store);
    drop(f.app.open_store());
    let (_, cfg) = f
        .app
        .get(
            &f.app.human,
            "/v1/projects/tp/document-classification-config",
        )
        .await;
    assert_eq!(cfg["mode"], "auto_apply_clear");
    assert_eq!(cfg["scheduling"], "automatic");
}

#[tokio::test]
async fn scheduling_changes_are_project_scoped_and_do_not_cancel_other_projects() {
    let f = fixture().await;
    enqueue(&f, "keep-other-project").await;
    f.app
        .post(
            &f.app.admin,
            "/v1/projects",
            json!({"id":"beta","name":"Other"}),
        )
        .await;
    let restricted = f.app.mint(
        "human:beta-admin",
        &["read", "write", "human", "admin"],
        Some(&["beta"]),
    );
    let (status, _) = f
        .app
        .put(
            &restricted,
            "/v1/projects/tp/document-classification-config",
            json!({"mode":"suggest","scheduling":"off"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = f
        .app
        .put(
            &restricted,
            "/v1/projects/beta/document-classification-config",
            json!({"mode":"suggest","scheduling":"off"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["cancelled"], 0);
    let (_, jobs) = f.app.get(&f.app.human, "/v1/agent-jobs?project=tp").await;
    assert_eq!(jobs["counts"]["queued"], 1);
    scheduling(&f, "manual").await;
    let (status, _) = f
        .app
        .patch(
            &f.app.worker,
            &format!("/v1/tickets/{}", f.ticket),
            json!({"title":"Edited while manual"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 0);
}

#[tokio::test]
async fn pending_manual_request_does_not_follow_a_ticket_into_another_project() {
    let f = fixture().await;
    scheduling(&f, "manual").await;
    let (status, _) = f
        .app
        .post(
            &f.app.human,
            "/v1/projects/tp/document-classification",
            json!({"request_id":"manual-source"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    f.app
        .post(
            &f.app.admin,
            "/v1/projects",
            json!({"id":"beta","name":"Other"}),
        )
        .await;
    f.app
        .put(
            &f.app.admin,
            "/v1/projects/beta/document-classification-config",
            json!({"mode":"suggest","scheduling":"manual"}),
        )
        .await;
    let (status, body) = f
        .app
        .post(
            &f.app.admin,
            "/v1/tickets/move",
            json!({"tickets":[f.ticket],"to_project":"beta"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let c = rusqlite::Connection::open(f.app.db_path()).unwrap();
    let pending: i64 = c
        .query_row(
            "SELECT count(*) FROM ticket_document_pending WHERE ticket=?1",
            [&f.ticket],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    assert_eq!(f.app.open_store().sweep_ticket_classification().unwrap(), 0);
}
