//! Verification: behaviors, linked test keys, reported runs, and the status
//! computed from them. See docs/verification.md.

mod common;

use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn behavior(app: &TestApp, body: Value) -> Value {
    let (s, b) = app
        .post(&app.worker, "/v1/projects/tp/behaviors", body)
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b
}

async fn report(app: &TestApp, body: Value) -> Value {
    let (s, b) = app.post(&app.worker, "/v1/projects/tp/runs", body).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b
}

async fn status_of(app: &TestApp, id: &str) -> String {
    let (s, b) = app.get(&app.worker, &format!("/v1/behaviors/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["status"].as_str().unwrap().to_string()
}

/// A plan section to link behaviors to.
async fn section(app: &TestApp) -> String {
    let (s, made) = app
        .post(
            &app.admin,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Plan" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (s, node) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "Saving" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{node}");
    node["nodes"][0]["id"].as_str().unwrap().to_string()
}

/// The whole loop: describe, link, report, read the status back.
#[tokio::test]
async fn status_follows_the_latest_result_of_each_linked_test() {
    let app = TestApp::spawn().await;
    let b = behavior(
        &app,
        json!({
            "title": "A failed save keeps edits",
            "statement": "Make saving fail; the edits stay and retry works.",
            "tests": ["ui:save retry", "api:save conflict", "ui:save retry"],
        }),
    )
    .await;
    let id = b["id"].as_str().unwrap();
    assert!(id.starts_with("bhv-"), "{b}");
    assert_eq!(b["status"], "untested");
    assert_eq!(b["last_result"], Value::Null);
    // Deduplicated and sorted.
    assert_eq!(b["tests"], json!(["api:save conflict", "ui:save retry"]));

    // One fresh pass is enough: which variants to run is the reporter's call.
    let out = report(
        &app,
        json!({
            "commit": "a1b2c3",
            "note": "Only the UI path changed.",
            "results": [
                { "test": "ui:save retry", "outcome": "pass" },
                { "test": "unrelated:thing", "outcome": "pass" },
            ],
        }),
    )
    .await;
    assert_eq!(out["run"]["passed"], 2);
    assert_eq!(out["run"]["failed"], 0);
    assert_eq!(out["run"]["commit"], "a1b2c3");
    assert_eq!(out["behaviors_affected"], 1);
    assert_eq!(out["unlinked"], json!(["unrelated:thing"]));
    assert_eq!(status_of(&app, id).await, "verified");

    // A failure on any linked test wins over the pass.
    report(
        &app,
        json!({
            "commit": "d4e5f6",
            "results": [{ "test": "api:save conflict", "outcome": "fail", "detail": "409 not retryable" }],
        }),
    )
    .await;
    let (_, detail) = app.get(&app.worker, &format!("/v1/behaviors/{id}")).await;
    assert_eq!(detail["status"], "failing", "{detail}");
    assert_eq!(detail["last_result"]["outcome"], "fail");
    assert_eq!(detail["last_result"]["commit"], "d4e5f6");
    let latest: Vec<&Value> = detail["test_results"].as_array().unwrap().iter().collect();
    assert_eq!(latest.len(), 2);
    let api = latest
        .iter()
        .find(|t| t["test"] == "api:save conflict")
        .unwrap();
    assert_eq!(api["latest"]["detail"], "409 not retryable");
    assert_eq!(api["latest"]["actor"], "agent:w1");
    let history = detail["history"].as_array().unwrap();
    assert_eq!(history.len(), 2, "{detail}");
    assert_eq!(history[0]["outcome"], "fail", "newest first: {detail}");
    assert_eq!(history[1]["note"], "Only the UI path changed.");

    // Fixed: the latest result is what counts, not the worst ever seen.
    report(
        &app,
        json!({ "results": [{ "test": "api:save conflict", "outcome": "pass" }] }),
    )
    .await;
    assert_eq!(status_of(&app, id).await, "verified");
}

/// A pass older than the freshness window is stale, not verified — and a
/// behavior that never had a result stays untested rather than stale.
#[tokio::test]
async fn old_passes_read_stale_and_never_run_reads_untested() {
    let app = TestApp::spawn().await;
    let b = behavior(&app, json!({ "title": "Old", "tests": ["t:old"] })).await;
    let untested = behavior(&app, json!({ "title": "Never", "tests": ["t:never"] })).await;
    let out = report(
        &app,
        json!({ "results": [{ "test": "t:old", "outcome": "pass" }] }),
    )
    .await;
    app.backdate_run(out["run"]["id"].as_str().unwrap(), 15 * 86_400_000);
    assert_eq!(status_of(&app, b["id"].as_str().unwrap()).await, "stale");
    assert_eq!(
        status_of(&app, untested["id"].as_str().unwrap()).await,
        "untested"
    );

    let (s, sum) = app.get(&app.worker, "/v1/projects/tp/verification").await;
    assert_eq!(s, StatusCode::OK, "{sum}");
    assert_eq!(sum["fresh_days"], 14);
    assert_eq!(
        sum["summary"],
        json!({ "total": 2, "verified": 0, "failing": 0, "stale": 1, "untested": 1 })
    );
}

#[tokio::test]
async fn behaviors_link_to_a_plan_section_and_the_summary_counts_per_section() {
    let app = TestApp::spawn().await;
    let node = section(&app).await;
    let linked = behavior(
        &app,
        json!({ "title": "Linked", "section": node, "tests": ["t:a"] }),
    )
    .await;
    behavior(&app, json!({ "title": "Loose" })).await;
    report(
        &app,
        json!({ "results": [
        { "test": "t:a", "outcome": "fail" },
        { "test": "t:orphan", "outcome": "pass" },
    ] }),
    )
    .await;

    let (_, sum) = app.get(&app.worker, "/v1/projects/tp/verification").await;
    assert_eq!(sum["sections"][&node]["total"], 1, "{sum}");
    assert_eq!(sum["sections"][&node]["failing"], 1, "{sum}");
    assert_eq!(sum["unsectioned"], 1);
    assert_eq!(sum["unlinked_tests"]["total"], 1);
    assert_eq!(sum["unlinked_tests"]["items"][0]["test"], "t:orphan");
    assert_eq!(sum["latest_run"]["failed"], 1);

    let (_, list) = app
        .get(
            &app.worker,
            &format!("/v1/projects/tp/behaviors?section={node}"),
        )
        .await;
    assert_eq!(list["total"], 1, "{list}");
    assert_eq!(list["items"][0]["id"], linked["id"]);
    let (_, list) = app
        .get(&app.worker, "/v1/projects/tp/behaviors?section=none")
        .await;
    assert_eq!(list["items"][0]["title"], "Loose", "{list}");

    // A section that is not in this project's plan is refused with a remedy.
    let (s, bad) = app
        .post(
            &app.worker,
            "/v1/projects/tp/behaviors",
            json!({ "title": "x", "section": "mn-nosuchnode" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.behavior_section");
    assert!(bad["remedy"].as_str().unwrap().contains("node id"), "{bad}");

    // null unlinks.
    let id = linked["id"].as_str().unwrap();
    let (s, b) = app
        .patch(
            &app.worker,
            &format!("/v1/behaviors/{id}"),
            json!({ "section": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["section"], Value::Null);
}

#[tokio::test]
async fn patch_replaces_tests_and_delete_keeps_the_results() {
    let app = TestApp::spawn().await;
    let b = behavior(&app, json!({ "title": "Edit me", "tests": ["t:one"] })).await;
    let id = b["id"].as_str().unwrap();
    report(
        &app,
        json!({ "results": [{ "test": "t:two", "outcome": "pass" }] }),
    )
    .await;

    let (s, b) = app
        .patch(
            &app.worker,
            &format!("/v1/behaviors/{id}"),
            json!({ "title": "Edited", "statement": "Now with words", "tests": ["t:two"] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["title"], "Edited");
    assert_eq!(b["statement"], "Now with words");
    assert_eq!(b["tests"], json!(["t:two"]));
    // Linking a test that already reported picks its result up at once.
    assert_eq!(b["status"], "verified");

    let (s, _) = app
        .delete(&app.worker, &format!("/v1/behaviors/{id}"))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, gone) = app.get(&app.worker, &format!("/v1/behaviors/{id}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{gone}");
    // The result is evidence about the test, so it stays — now unlinked.
    let (_, sum) = app.get(&app.worker, "/v1/projects/tp/verification").await;
    assert_eq!(sum["unlinked_tests"]["items"][0]["test"], "t:two", "{sum}");
}

#[tokio::test]
async fn input_is_validated_with_teaching_errors() {
    let app = TestApp::spawn().await;
    for (path, body, code) in [
        (
            "/v1/projects/tp/behaviors",
            json!({ "title": "  " }),
            "validation.behavior_title",
        ),
        (
            "/v1/projects/tp/behaviors",
            json!({ "title": "x", "tests": ["bad\nkey"] }),
            "validation.test_key",
        ),
        (
            "/v1/projects/tp/runs",
            json!({ "results": [] }),
            "validation.run_results",
        ),
        (
            "/v1/projects/tp/runs",
            json!({ "results": [{ "test": "t", "outcome": "skipped" }] }),
            "validation.run_outcome",
        ),
        (
            "/v1/projects/tp/runs",
            json!({ "results": [{ "test": "t", "outcome": "pass" }, { "test": "t", "outcome": "fail" }] }),
            "validation.run_duplicate_test",
        ),
    ] {
        let (s, b) = app.post(&app.worker, path, body).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{path}: {b}");
        assert_eq!(b["code"], code, "{b}");
        assert!(b["remedy"].as_str().is_some_and(|r| !r.is_empty()), "{b}");
    }
    let (s, b) = app
        .get(&app.worker, "/v1/projects/tp/behaviors?status=green")
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.behavior_status");

    let tests: Vec<String> = (0..101).map(|i| format!("t:{i}")).collect();
    let (s, b) = app
        .post(
            &app.worker,
            "/v1/projects/tp/behaviors",
            json!({ "title": "Too broad", "tests": tests }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.behavior_tests");
}

/// A retried report with the same key records once; the same key with a
/// different body is a conflict, not a silent second run.
#[tokio::test]
async fn a_retried_report_records_once() {
    let app = TestApp::spawn().await;
    let body = json!({ "commit": "c1", "results": [{ "test": "t", "outcome": "pass" }] });
    let (s, first) = app
        .post_with(
            &app.worker,
            "/v1/projects/tp/runs",
            &[("Idempotency-Key", "ci-42")],
            body.clone(),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{first}");
    let (s, again) = app
        .post_with(
            &app.worker,
            "/v1/projects/tp/runs",
            &[("Idempotency-Key", "ci-42")],
            body,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{again}");
    assert_eq!(again["run"]["id"], first["run"]["id"]);
    assert_eq!(first["replayed"], false);
    assert_eq!(again["replayed"], true);
    let (_, runs) = app.get(&app.worker, "/v1/projects/tp/runs").await;
    assert_eq!(runs["total"], 1, "{runs}");

    let (s, b) = app
        .post_with(
            &app.worker,
            "/v1/projects/tp/runs",
            &[("Idempotency-Key", "ci-42")],
            json!({ "results": [{ "test": "t", "outcome": "fail" }] }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "conflict.idempotency_key");
}

#[tokio::test]
async fn lists_are_bounded_and_say_so() {
    let app = TestApp::spawn().await;
    for i in 0..3 {
        behavior(&app, json!({ "title": format!("B{i}") })).await;
        report(
            &app,
            json!({ "results": [{ "test": format!("t{i}"), "outcome": "pass" }] }),
        )
        .await;
    }
    let (_, page) = app
        .get(&app.worker, "/v1/projects/tp/behaviors?limit=2")
        .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert_eq!(page["total"], 3);
    assert_eq!(page["limit"], 2);
    assert!(page["note"].as_str().unwrap().contains("offset"), "{page}");
    let (_, rest) = app
        .get(&app.worker, "/v1/projects/tp/behaviors?limit=2&offset=2")
        .await;
    assert_eq!(rest["items"][0]["title"], "B2", "{rest}");

    let (_, runs) = app.get(&app.worker, "/v1/projects/tp/runs?limit=1").await;
    assert_eq!(runs["total"], 3);
    assert!(runs["note"].is_string(), "{runs}");
}

#[tokio::test]
async fn scope_project_and_archive_guards_hold() {
    let app = TestApp::spawn().await;
    let b = behavior(&app, json!({ "title": "Guarded" })).await;
    let id = b["id"].as_str().unwrap();

    let reader = app.mint("reader", &["read"], None);
    let (s, _) = app
        .post(
            &reader,
            "/v1/projects/tp/behaviors",
            json!({ "title": "x" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = app
        .post(
            &reader,
            "/v1/projects/tp/runs",
            json!({ "results": [{ "test": "t", "outcome": "pass" }] }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A token scoped to another project cannot reach this one's behavior by id.
    app.create_project_with("other", common::simple_workflow())
        .await;
    let outsider = app.mint("outsider", &["read", "write"], Some(&["other"]));
    let (s, _) = app.get(&outsider, &format!("/v1/behaviors/{id}")).await;
    assert!(
        s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND,
        "{s}"
    );
    let (s, _) = app.get(&outsider, "/v1/projects/tp/verification").await;
    assert!(
        s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND,
        "{s}"
    );

    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = app
        .post(
            &app.worker,
            "/v1/projects/tp/runs",
            json!({ "results": [{ "test": "t", "outcome": "pass" }] }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    let (s, b) = app
        .patch(
            &app.worker,
            &format!("/v1/behaviors/{id}"),
            json!({ "title": "no" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
}
