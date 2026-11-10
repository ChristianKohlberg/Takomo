//! `POST /v1/tickets/{id}/reopen` — bringing a ticket back out of a terminal
//! state. Terminal states have no workflow edges by contract, so the reopen is
//! its own operation; these tests pin what it does and every guard it keeps.
mod common;
use common::TestApp;
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn reopen(app: &TestApp, token: &str, id: &str, body: Value) -> (StatusCode, Value) {
    app.post(token, &format!("/v1/tickets/{id}/reopen"), body)
        .await
}

/// Drive a leaf ticket to `done` and return the worker's fence from before the
/// close — the token a zombie worker would still be holding.
async fn done_with_fence(app: &TestApp, id: &str) -> i64 {
    let fence = app.to_implementing(id).await;
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "review", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->review failed: {b}");
    let (s, b) = app.transition(&app.human, id, "done").await;
    assert_eq!(s, StatusCode::OK, "->done failed: {b}");
    fence
}

async fn events(app: &TestApp, id: &str, kind: &str) -> Vec<Value> {
    let (s, body) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind={kind}"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body["events"].as_array().expect("events array").clone()
}

#[tokio::test]
async fn reopen_moves_a_done_ticket_to_the_initial_state_and_records_who_and_why() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("closed by a script").await;
    let stale_fence = done_with_fence(&app, &id).await;

    // A plain transition still cannot leave the terminal state, and the
    // refusal now names the way back.
    let (s, b) = app.transition(&app.human, &id, "brief").await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "transition.illegal");
    assert!(
        b["remedy"].as_str().unwrap().contains("/reopen"),
        "the terminal-state remedy points at reopen: {b}"
    );

    let (s, t) = reopen(
        &app,
        &app.human,
        &id,
        json!({ "reason": "  epic closed by the nightly sweep by mistake  " }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(
        t["state"], "brief",
        "defaults to the workflow's initial state"
    );
    assert!(t["claim"].is_null(), "no lease survives a reopen: {t}");

    let reopened = events(&app, &id, "ticket_reopened").await;
    assert_eq!(reopened.len(), 1, "{reopened:?}");
    let e = &reopened[0];
    assert_eq!(e["actor"], "human:reviewer");
    assert_eq!(e["payload"]["from"], "done");
    assert_eq!(e["payload"]["to"], "brief");
    assert_eq!(
        e["payload"]["reason"], "epic closed by the nightly sweep by mistake",
        "the reason is trimmed"
    );
    assert_eq!(
        e["payload"]["new_fence"].as_i64().unwrap(),
        e["payload"]["fence"].as_i64().unwrap() + 1
    );

    // The state change itself is an ordinary `transitioned` event.
    let moved = events(&app, &id, "transitioned").await;
    let last = moved.last().expect("transitioned events");
    assert_eq!(last["payload"]["from"], "done");
    assert_eq!(last["payload"]["to"], "brief");

    // And the ticket carries a comment saying who reopened it and why.
    let (_, detail) = app
        .get(&app.admin, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let comments = detail["comments"].as_array().expect("comments");
    let note = comments.last().expect("a reopen comment");
    assert_eq!(note["author"], "human:reviewer");
    assert_eq!(note["id"], e["payload"]["comment"]);
    let body = note["body"].as_str().unwrap();
    assert!(
        body.contains("done") && body.contains("brief") && body.contains("nightly sweep"),
        "{body}"
    );

    // The fence was bumped: the worker still holding the pre-close token is a
    // zombie and cannot write to the reopened ticket.
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "spec", "fence": stale_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "fence.stale");

    // The reopened ticket is back in its lifecycle and moves normally.
    let (s, b) = app.transition(&app.worker, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

#[tokio::test]
async fn reopen_into_an_explicit_state_and_validates_the_target() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("cancelled too early").await;
    let (s, b) = app.transition(&app.worker, &id, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (s, b) = reopen(
        &app,
        &app.worker,
        &id,
        json!({ "reason": "r", "to": "done" }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "reopen.target_terminal");

    let (s, b) = reopen(
        &app,
        &app.worker,
        &id,
        json!({ "reason": "r", "to": "nope" }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "reopen.unknown_state");

    let (s, t) = reopen(
        &app,
        &app.worker,
        &id,
        json!({ "reason": "r", "to": "ready" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["state"], "ready");
}

#[tokio::test]
async fn reopen_refuses_a_ticket_that_is_not_terminal_with_a_stable_code() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("still alive").await;

    let (s, b) = reopen(&app, &app.admin, &id, json!({ "reason": "why not" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "reopen.not_terminal");
    assert_eq!(b["current_state"], "brief");
    assert!(b["allowed_transitions"].is_array(), "{b}");

    // A retried reopen that already landed gets the same answer, not a second write.
    let (s, _) = app.transition(&app.worker, &id, "cancelled").await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = reopen(&app, &app.worker, &id, json!({ "reason": "once" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = reopen(&app, &app.worker, &id, json!({ "reason": "once" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "reopen.not_terminal");
    assert_eq!(events(&app, &id, "ticket_reopened").await.len(), 1);
}

#[tokio::test]
async fn reopen_requires_a_reason() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("no reason given").await;
    let (s, _) = app.transition(&app.worker, &id, "cancelled").await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = reopen(&app, &app.worker, &id, json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert_eq!(b["code"], "validation.field_required");

    let (s, b) = reopen(&app, &app.worker, &id, json!({ "reason": "   " })).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.reason");

    let (s, b) = reopen(
        &app,
        &app.worker,
        &id,
        json!({ "reason": "x".repeat(2001) }),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.reason");

    let (s, b) = reopen(
        &app,
        &app.worker,
        &id,
        json!({ "reason": "r", "force": true }),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "unknown fields are refused: {b}"
    );

    // Nothing moved.
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(t["state"], "cancelled");
}

#[tokio::test]
async fn reopen_needs_write_and_the_authority_that_closed_the_ticket() {
    let app = TestApp::spawn().await;

    // Read-only tokens cannot reopen anything.
    let reader = app.mint("agent:reader", &["read"], None);
    let cancelled = app.create_ticket("cancelled").await;
    let (s, _) = app.transition(&app.worker, &cancelled, "cancelled").await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = reopen(&app, &reader, &cancelled, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(b["code"], "auth.scope");

    // A token limited to another project cannot see the ticket at all.
    let outsider = app.mint("agent:outsider", &["read", "write"], Some(&["elsewhere"]));
    let (s, b) = reopen(&app, &outsider, &cancelled, json!({ "reason": "r" })).await;
    assert!(
        s == StatusCode::FORBIDDEN || s == StatusCode::NOT_FOUND,
        "project allowlist is enforced: {s} {b}"
    );

    // `done` is only reachable through a scope:human edge on factory-default, so
    // undoing it needs `human` too; a plain writer is told exactly what is missing.
    let done = app.create_ticket("approved and done").await;
    done_with_fence(&app, &done).await;
    let (s, b) = reopen(&app, &app.worker, &done, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(b["code"], "reopen.scope");
    assert_eq!(b["details"]["missing_scopes"], json!(["human"]));
    let (s, b) = reopen(&app, &app.human, &done, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // `cancelled` has unscoped edges into it, so `write` is enough.
    let (s, b) = reopen(&app, &app.worker, &cancelled, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

#[tokio::test]
async fn reopen_is_refused_while_the_project_is_archived() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("in an archived project").await;
    let (s, _) = app.transition(&app.worker, &id, "cancelled").await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (s, b) = reopen(&app, &app.admin, &id, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "project.archived");
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(t["state"], "cancelled");
}

#[tokio::test]
async fn reopen_puts_the_parent_before_the_child() {
    let app = TestApp::spawn().await;
    let epic = app
        .create_typed("epic closed by a script", "epic", None)
        .await;
    let child = app.create_typed("its child", "task", Some(&epic)).await;
    for id in [&child, &epic] {
        let (s, b) = app.transition(&app.worker, id, "cancelled").await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }

    // A child cannot come back under a parent that is still closed.
    let (s, b) = reopen(&app, &app.worker, &child, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "reopen.parent_terminal");
    assert_eq!(b["details"]["parent"], json!(epic));

    // Reopening the epic leaves its children as they are...
    let (s, b) = reopen(&app, &app.worker, &epic, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, c) = app.get(&app.admin, &format!("/v1/tickets/{child}")).await;
    assert_eq!(c["state"], "cancelled");

    // ...and the child can follow once the parent is open.
    let (s, b) = reopen(&app, &app.worker, &child, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "brief");
}

#[tokio::test]
async fn reopen_respects_an_active_lease_on_a_terminal_ticket() {
    // A workflow whose terminal state is not done/cancelled-category keeps the
    // lease on entry — the one way a terminal ticket can still be claimed.
    let app = TestApp::spawn().await;
    app.create_project_with(
        "lp",
        json!({
            "name": "lease-terminal",
            "initial": "todo",
            "states": [
                { "id": "todo", "category": "todo", "claimable": true },
                { "id": "doing", "category": "in_progress" },
                { "id": "parked", "category": "review", "terminal": true }
            ],
            "transitions": [
                { "from": "todo", "to": "doing", "requires": ["claim"] },
                { "from": "doing", "to": "parked", "requires": ["claim"] }
            ]
        }),
    )
    .await;
    let id = app.create_ticket_in("lp", "leased at the end").await;
    let fence = app.claim(&id).await;
    for to in ["doing", "parked"] {
        let (s, b) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{id}/transition"),
                json!({ "to": to, "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert!(b["claim"].is_object(), "lease kept: {b}");
    }

    // Someone else cannot pull the ticket out from under the holder.
    let (s, b) = reopen(&app, &app.worker2, &id, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "claim.held");

    // The holder must echo its fence.
    let (s, b) = reopen(&app, &app.worker, &id, json!({ "reason": "r" })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "fence.required");
    let (s, b) = reopen(
        &app,
        &app.worker,
        &id,
        json!({ "reason": "r", "fence": fence - 1 }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "fence.stale");

    let (s, t) = reopen(
        &app,
        &app.worker,
        &id,
        json!({ "reason": "r", "fence": fence }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["state"], "todo");
    assert!(t["claim"].is_null(), "the reopen released the lease: {t}");
    assert_eq!(events(&app, &id, "released").await.len(), 1);
}
