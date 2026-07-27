//! Integration tests: spawn the real server on an ephemeral port and drive it
//! over HTTP with reqwest.

use futures::future::join_all;
use reqwest::Method;
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use takomo::store::ShareKind;

mod common;
use common::TestApp;

// ---------------------------------------------------------------------------

#[tokio::test]
async fn healthz_open_everything_else_authed() {
    let app = TestApp::spawn().await;
    let resp = app.request(Method::GET, "/healthz").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .request(Method::GET, "/v1/tickets")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "auth.missing");

    let resp = app
        .request(Method::GET, "/v1/tickets")
        .bearer_auth("tk_bogusbogusbogusbogus1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn inbox_and_board_pages_served_unauthenticated() {
    let app = TestApp::spawn().await;
    for (path, marker) in [("/inbox", "takomo · inbox"), ("/board", "takomo")] {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should serve");
        let body = resp.text().await.unwrap();
        assert!(
            body.contains(marker),
            "{path} body should contain '{marker}'"
        );
        // The inbox is wired to the questions API (path built from a base var).
        assert!(
            body.contains("/questions") || path == "/board",
            "inbox talks to the questions API"
        );
        // Both surfaces link the octopus favicon.
        assert!(body.contains("rel=\"icon\""), "{path} links a favicon");
    }
}

/// The ticket filter on `/board` and `/inbox` is client-side, so its contract has
/// two halves that break independently: the API must keep exposing the fields the
/// filter reads, and the pages must keep shipping the control that reads them.
/// A server-side rename of `parent`/`ticket` would silently degrade both filters to
/// "matches nothing" without failing any other test.
#[tokio::test]
async fn ticket_filter_contract_on_board_and_inbox() {
    let app = TestApp::spawn().await;

    // An epic with a child, so the board's subtree filter has a chain to walk:
    // filtering by the epic must keep its subtasks visible, not orphan them.
    let epic = app.create_typed("Billing revamp", "epic", None).await;
    let child = app
        .create_typed("Migrate off billing_v1", "task", Some(&epic))
        .await;
    let other = app.create_typed("Unrelated work", "task", None).await;

    // Advisory: needs no claim/fence and parks nothing, so the ticket stays put.
    app.ask(
        &app.admin,
        json!({
            "ticket": child,
            "mode": "advisory",
            "kind": "confirm",
            "title": "Drop the legacy column?",
        }),
    )
    .await;

    // ---- half 1: the fields the filters read ----
    // The board walks `parent` upward to decide subtree membership.
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&limit=200")
        .await;
    let listed = list["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|t| t["id"] == json!(child))
        .expect("child ticket is listed");
    assert_eq!(
        listed["parent"],
        json!(epic),
        "tickets must expose `parent`; the board's subtree filter walks it"
    );

    // The inbox groups the queue by `ticket` — that field both populates its
    // picker and decides which questions a chosen ticket shows.
    let (_, qs) = app
        .get(&app.admin, "/v1/questions?project=tp&status=open")
        .await;
    assert!(
        qs["items"]
            .as_array()
            .expect("items array")
            .iter()
            .any(|x| x["ticket"] == json!(child)),
        "questions must expose `ticket`; the inbox filter groups on it"
    );

    // The same narrowing over HTTP, so an integrator filtering server-side gets
    // what the page computes client-side.
    let (_, only) = app
        .get(
            &app.admin,
            &format!("/v1/questions?ticket={child}&status=open"),
        )
        .await;
    let items = only["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "?ticket= narrows to that ticket: {only}");
    assert_eq!(items[0]["ticket"], json!(child));
    let (_, none) = app
        .get(
            &app.admin,
            &format!("/v1/questions?ticket={other}&status=open"),
        )
        .await;
    assert!(
        none["items"].as_array().expect("items array").is_empty(),
        "?ticket= on a ticket with no questions returns nothing: {none}"
    );

    // ---- half 2: the control that reads them ----
    for (path, wiring) in [("/board", "inSubtree("), ("/inbox", "visible()")] {
        let body = app
            .request(Method::GET, path)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            body.contains("id=\"ticksel\""),
            "{path} ships the ticket-filter control"
        );
        assert!(
            body.contains(wiring),
            "{path} wires the ticket filter into its render path ('{wiring}')"
        );
        // One STR table per language: a key added to only one renders as
        // `undefined` for whichever locale was forgotten.
        assert_eq!(
            body.matches("allTickets:").count(),
            2,
            "{path} needs `allTickets` in both the DE and EN string tables"
        );
    }
}

/// Keys of one locale's `STR` table, in declaration order.
///
/// The tables are one `key:"value",` per line (takomo-f9y5), so a line scan
/// reads them without pulling in a parser; anything else inside the block is a
/// hard error rather than a silently skipped key.
fn str_keys(file: &str, src: &str, locale: &str) -> Vec<String> {
    let open = format!("{locale}: {{");
    let mut lines = src.lines();
    lines
        .find(|l| l.trim() == "var STR = {")
        .unwrap_or_else(|| panic!("{file}: no `var STR = {{` table to check"));
    lines
        .find(|l| l.trim() == open)
        .unwrap_or_else(|| panic!("{file}: `STR` has no `{locale}` table"));
    let keys: Vec<String> = lines
        .map(str::trim)
        .take_while(|l| *l != "},")
        .map(|l| {
            let (key, _) = l.split_once(':').unwrap_or_else(|| {
                panic!("{file}: `{locale}` line is not `key:\"value\",` — cannot read it: {l}")
            });
            key.to_string()
        })
        .collect();
    assert!(
        !keys.is_empty(),
        "{file}: read 0 keys from `{locale}` — the one-key-per-line table layout this scan \
         relies on is gone, so it is guarding nothing"
    );
    keys
}

/// Every UI string must exist in both locales: a key added to only one renders
/// as `undefined` for whoever gets that language. `include_str!` reads exactly
/// the bytes the binary embeds, so no server is needed here — and a moved file
/// fails the build instead of quietly checking nothing.
#[test]
fn spa_string_tables_agree_on_every_key() {
    for (file, src) in [
        ("src/board.html", include_str!("../src/board.html")),
        ("src/inbox.html", include_str!("../src/inbox.html")),
    ] {
        let de = str_keys(file, src, "de");
        let en = str_keys(file, src, "en");
        for (have, want, missing) in [(&de, &en, "en"), (&en, &de, "de")] {
            for key in have {
                assert!(
                    want.contains(key),
                    "{file}: STR key `{key}` is missing from the `{missing}` table — add it, \
                     or that string renders as `undefined` for {missing} readers"
                );
            }
        }
        assert_eq!(
            de.len(),
            en.len(),
            "{file}: the de and en tables use the same key names but differ in length — \
             one of them repeats a key"
        );
        if let Some(i) = de.iter().zip(&en).position(|(d, e)| d != e) {
            let (d, e) = (&de[i], &en[i]);
            panic!(
                "{file}: de and en list the same keys in a different order (entry {i}: de has \
                 `{d}`, en has `{e}`) — keep the tables line-for-line parallel so a UI diff \
                 stays reviewable"
            );
        }
    }
}

#[tokio::test]
async fn favicon_served_unauthenticated_as_svg() {
    let app = TestApp::spawn().await;
    for path in ["/favicon.svg", "/favicon.ico"] {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should serve");
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(
            ct.contains("image/svg+xml"),
            "{path} should be served as SVG, got '{ct}'"
        );
        let body = resp.text().await.unwrap();
        assert!(body.contains("<svg"), "{path} body should be an SVG");
    }
}

#[tokio::test]
async fn workflow_enforcement_illegal_transition_teaches() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Illegal transition test").await;

    let (status, body) = app.transition(&app.admin, &id, "done").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "transition.illegal");
    assert_eq!(body["current_state"], "brief");
    let allowed: Vec<&str> = body["allowed_transitions"]
        .as_array()
        .expect("allowed_transitions present")
        .iter()
        .map(|t| t["to"].as_str().unwrap())
        .collect();
    assert!(
        allowed.contains(&"spec") && allowed.contains(&"cancelled"),
        "{allowed:?}"
    );
    assert!(body["remedy"].as_str().unwrap().contains("/transition"));
    assert!(body["message"].as_str().unwrap().contains("brief"));

    // Unknown state also teaches.
    let (status, body) = app.transition(&app.admin, &id, "nonexistent").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "transition.unknown_state");
    assert!(body["allowed_transitions"].is_array());
}

#[tokio::test]
async fn scope_gate_403_without_human() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Scope gate test").await;
    let (s, b) = app.transition(&app.worker, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // spec -> ready requires scope:human; the worker lacks it.
    let (status, body) = app.transition(&app.worker, &id, "ready").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "transition.scope");
    assert!(body["message"].as_str().unwrap().contains("human"));
    assert!(body["allowed_transitions"].is_array());

    // With the human scope it passes.
    let (status, body) = app.transition(&app.human, &id, "ready").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "ready");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_ready_claims_are_exactly_once() {
    let app = TestApp::spawn().await;
    const N: usize = 8;
    for i in 0..N {
        let id = app
            .create_ticket(&format!("Concurrent claim target number {i}"))
            .await;
        app.to_ready(&id).await;
    }

    // 16 simultaneous claimers race for 8 tickets: exactly 8 win distinct
    // tickets, the rest get 204.
    let mut futures = Vec::new();
    for i in 0..(N * 2) {
        let token = if i % 2 == 0 {
            app.worker.clone()
        } else {
            app.worker2.clone()
        };
        let client = app.client.clone();
        let base = app.base.clone();
        futures.push(tokio::spawn(async move {
            let resp = client
                .post(format!("{base}/v1/ready/claim"))
                .bearer_auth(token)
                .json(&json!({ "project": "tp" }))
                .send()
                .await
                .expect("claim request");
            let status = resp.status();
            let body = resp.json::<Value>().await.unwrap_or(Value::Null);
            (status, body)
        }));
    }
    let results: Vec<(StatusCode, Value)> = join_all(futures)
        .await
        .into_iter()
        .map(|r| r.expect("join"))
        .collect();

    let mut claimed_ids: Vec<String> = results
        .iter()
        .filter(|(s, _)| *s == StatusCode::OK)
        .map(|(_, b)| b["id"].as_str().expect("claimed id").to_string())
        .collect();
    let misses = results
        .iter()
        .filter(|(s, _)| *s == StatusCode::NO_CONTENT)
        .count();

    assert_eq!(claimed_ids.len(), N, "exactly {N} claims must succeed");
    assert_eq!(misses, N, "the other {N} callers must get 204");
    claimed_ids.sort();
    claimed_ids.dedup();
    assert_eq!(
        claimed_ids.len(),
        N,
        "no ticket may be handed to two claimants"
    );

    // Every winner got a lease with a fence.
    for (s, b) in &results {
        if *s == StatusCode::OK {
            assert!(b["lease"]["fence"].as_i64().unwrap() >= 1);
            assert!(b["claim"]["holder"].is_string());
        }
    }
}

/// Reads must not serialize writers. `GET /v1/export` with no project filter
/// walks the whole tickets table and, per ticket, queries its deps and its
/// comments — the longest read in the API. While it is in flight, claims and
/// heartbeats have to keep completing at their normal speed.
///
/// Before the read connections landed, `with_conn` and `with_tx` shared one
/// `Mutex<Connection>`, so one export stalled every claim, transition and
/// heartbeat in the process for as long as the scan took. Measured on this test
/// at 8k tickets: the worst claim during an export was **104ms** (~80% of the
/// whole export) against a 1.8ms worst case with the store idle. With the read
/// connections: **6-17ms**, and roughly twice as many claims complete during the
/// same export.
///
/// The claim is issued in a loop rather than once, because a single sample can
/// miss the scan window and pass a broken store. The loop samples the entire
/// export: the first iteration claims, the rest are lease renewals — both are
/// `with_tx` writes, which is exactly what used to queue.
///
/// The assertion is a ratio (plus an absolute escape hatch), not a wall-clock
/// budget: a loaded box inflates both numbers together, but only a store that
/// serializes reads against writes makes a claim take a large *fraction of the
/// export*.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_export_does_not_stall_claims_and_heartbeats() {
    let app = TestApp::spawn().await;
    let target = app
        .create_ticket("Claim me while a full export is in flight")
        .await;
    app.to_ready(&target).await;
    // Enough rows that an unfiltered export is a real scan, seeded straight into
    // the DB — 8k tickets over HTTP would dominate the suite's runtime.
    const BULK: usize = 8_000;
    app.seed_bulk_tickets(BULK);

    // Warm the page cache, so the timings below measure the scan and not the
    // first read of a cold file.
    let (warm, _, _) = app.get_raw(&app.admin, "/v1/export").await;
    assert_eq!(warm, StatusCode::OK);

    // What a claim/renewal costs with the store otherwise idle.
    let mut idle: Vec<Duration> = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        let (s, lease) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{target}/claim"),
                json!({}),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "claim failed: {lease}");
        assert!(lease["fence"].as_i64().unwrap() >= 1);
        idle.push(started.elapsed());
    }
    idle.sort();
    let idle_worst = *idle.last().expect("idle samples");

    // What the export costs with nothing to contend with.
    let started = Instant::now();
    let (status, _, text) = app.get_raw(&app.admin, "/v1/export").await;
    let solo_export = started.elapsed();
    assert_eq!(status, StatusCode::OK);
    assert!(
        text.lines().count() > BULK,
        "export must cover the bulk rows, got {} lines",
        text.lines().count()
    );
    assert!(
        solo_export >= Duration::from_millis(50),
        "the export ran in {solo_export:?} — too fast to prove anything; raise BULK"
    );

    // Now the same export, sampled by claims for its whole duration.
    let client = app.client.clone();
    let url = app.url("/v1/export");
    let admin = app.admin.clone();
    let export = tokio::spawn(async move {
        let resp = client
            .get(url)
            .bearer_auth(admin)
            .send()
            .await
            .expect("export request");
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        (status, body.lines().count())
    });

    let mut during: Vec<Duration> = Vec::new();
    while !export.is_finished() {
        let started = Instant::now();
        let (s, lease) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{target}/claim"),
                json!({}),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "claim during export failed: {lease}");
        during.push(started.elapsed());
    }
    let (export_status, export_lines) = export.await.expect("join export task");
    assert_eq!(export_status, StatusCode::OK);
    assert!(
        export_lines > BULK,
        "the concurrent export must be complete"
    );

    during.sort();
    let worst = *during.last().expect("samples during the export");
    eprintln!(
        "export {solo_export:?} | idle claim worst {idle_worst:?} | \
         {} claims during the export, median {:?}, worst {worst:?}",
        during.len(),
        during[during.len() / 2],
    );

    assert!(
        worst * 4 < solo_export || worst < Duration::from_millis(25),
        "the worst claim during the export took {worst:?} out of an export that runs in \
         {solo_export:?} alone (idle worst: {idle_worst:?}) — reads are serializing writers again"
    );
}

#[tokio::test]
async fn fence_goes_stale_after_expiry_and_reclaim() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Fence staleness test").await;
    app.to_ready(&id).await;

    // Worker 1 claims with a 1-second lease.
    let old_fence = app.claim_ttl(&app.worker, &id, Some(1)).await;

    // Let the lease expire (sweep interval is 250ms in tests).
    tokio::time::sleep(Duration::from_millis(1600)).await;

    // The lease_expired event was emitted and the ticket is ready again.
    let (_, events) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=lease_expired"),
        )
        .await;
    assert_eq!(
        events["events"].as_array().unwrap().len(),
        1,
        "lease_expired event expected: {events}"
    );

    // Worker 2 claims it; the fence must be strictly greater.
    let new_fence = app.claim_as(&app.worker2, &id).await;
    assert!(new_fence > old_fence);

    // Zombie worker 1 heartbeats with the stale fence: teaching 409.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({ "fence": old_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "fence.stale");

    // Stale-fence patch by the zombie also bounces (claim held by w2).
    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "hijack attempt", "fence": old_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "claim.held");
}

#[tokio::test]
async fn heartbeat_renewal_emits_no_event() {
    // ts-8zks: lease renewal is silent bookkeeping. Heartbeats must not write
    // to the append-only event log (they flood it at fleet scale), while the
    // lease itself is still renewed.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Heartbeat quiet test").await;
    app.to_ready(&id).await;

    let fence = app.claim_ttl(&app.worker, &id, Some(900)).await;

    // Two heartbeats on the same lease.
    for _ in 0..2 {
        let (s, body) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{id}/heartbeat"),
                json!({ "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "heartbeat should renew: {body}");
        assert!(body["expires_at"].is_string(), "lease renewed: {body}");
    }

    // An idempotent re-claim by the same holder is also a renewal.
    app.claim(&id).await;

    // No heartbeat event ever reached the log.
    let (_, hb) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=heartbeat"),
        )
        .await;
    assert_eq!(
        hb["events"].as_array().unwrap().len(),
        0,
        "no heartbeat events expected in the log: {hb}"
    );

    // The claim itself is still observable (exactly one `claimed`).
    let (_, claimed) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=claimed"),
        )
        .await;
    assert_eq!(
        claimed["events"].as_array().unwrap().len(),
        1,
        "the claim is still logged: {claimed}"
    );
}

#[tokio::test]
async fn body_replacement_requires_cas() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("CAS body test").await;

    // No If-Match: refused with instructions.
    let (s, body) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "body": "new body" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict.if_match_required");
    assert_eq!(body["current_version"], 1);

    // Wrong If-Match: version conflict with current version + body hash.
    let (s, body) = app
        .patch_with(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            &[("If-Match", "\"99\"")],
            json!({ "body": "new body" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict.version");
    assert_eq!(body["current_version"], 1);
    assert!(body["details"]["body_sha256"].is_string());

    // Correct If-Match succeeds and bumps the version.
    let (s, body) = app
        .patch_with(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            &[("If-Match", "\"1\"")],
            json!({ "body": "new body" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["body"], "new body");
    assert_eq!(body["version"], 2);

    // ETag on GET reflects the version.
    let resp = app
        .authed(Method::GET, &app.admin, &format!("/v1/tickets/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.headers().get("ETag").unwrap().to_str().unwrap(),
        "\"2\""
    );
}

#[tokio::test]
async fn idempotent_create_replays() {
    let app = TestApp::spawn().await;
    let req = json!({ "project": "tp", "title": "Idempotency replay test" });

    let (s, first) = app
        .post_with(
            &app.admin,
            "/v1/tickets",
            &[("Idempotency-Key", "create-once")],
            req.clone(),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, second) = app
        .post_with(
            &app.admin,
            "/v1/tickets",
            &[("Idempotency-Key", "create-once")],
            req.clone(),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "replay must be 200, not a twin 201");
    assert_eq!(first["id"], second["id"]);

    // Only one ticket exists with that title.
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&q=Idempotency+replay")
        .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);

    // similar[] hints on a keyword-overlapping create.
    let (status, body) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Idempotency replay follow-up work" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let similar = body["similar"].as_array().unwrap();
    assert!(
        similar.iter().any(|s| s["id"] == first["id"]),
        "similar should mention the twin: {body}"
    );
}

#[tokio::test]
async fn blocked_tickets_never_ready_including_inherited() {
    let app = TestApp::spawn().await;
    let blocker = app.create_ticket("The blocker nobody finished").await;
    let epic = app.create_ticket("Epic parent blocked by dependency").await;
    let child = app
        .create_ticket("Child inherits the ancestor blockage")
        .await;

    // child under epic; epic blocked_by blocker.
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{child}"),
            json!({ "parent": epic }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{epic}/deps"),
            json!({ "blocked_by": blocker }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    app.to_ready(&epic).await;
    app.to_ready(&child).await;

    // Neither epic (directly blocked) nor child (via ancestor) is ready.
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    let ids: Vec<&str> = ready
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        !ids.contains(&epic.as_str()),
        "blocked epic in ready: {ids:?}"
    );
    assert!(
        !ids.contains(&child.as_str()),
        "ancestor-blocked child in ready: {ids:?}"
    );

    // Direct claim also refuses, naming the blocker.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{child}/claim"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "claim.blocked");
    assert!(body["message"].as_str().unwrap().contains(&blocker));

    // Terminal blocker unblocks both (cancelled is terminal).
    let (s, b) = app.transition(&app.admin, &blocker, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    let ids: Vec<&str> = ready
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&epic.as_str()) && ids.contains(&child.as_str()),
        "{ids:?}"
    );

    // Dependency cycles are refused.
    let (s, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{blocker}/deps"),
            json!({ "blocked_by": epic }),
        )
        .await;
    // blocker <- epic already exists, so epic blocked_by blocker + this = cycle
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "validation.dep_cycle");
}

#[tokio::test]
async fn no_open_children_guard_blocks_done() {
    let app = TestApp::spawn().await;
    let parent = app.create_ticket("Parent epic with open child").await;
    let child = app.create_ticket("Open child of the epic").await;
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{child}"),
            json!({ "parent": parent }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // Drive parent to review: ready -> claim -> implementing -> review -> release.
    app.to_ready(&parent).await;
    let fence = app.claim(&parent).await;

    // ready -> implementing requires the claim; without a fence it teaches.
    let (s, body) = app.transition(&app.worker, &parent, "implementing").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.required");

    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{parent}/transition"),
            json!({ "to": "implementing", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{parent}/transition"),
            json!({ "to": "review", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{parent}/release"),
            json!({ "fence": fence, "reason": "PR open" }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // review -> done blocked by the open child, naming it.
    let (s, body) = app.transition(&app.human, &parent, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.guard");
    assert!(body["message"].as_str().unwrap().contains(&child));
    assert_eq!(body["details"]["offending_tickets"][0], child.as_str());

    // Close the child, then done passes.
    let (s, b) = app.transition(&app.admin, &child, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, body) = app.transition(&app.human, &parent, "done").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done");
    assert_eq!(body["state_category"], "done");
}

/// `guard:has_link:commit` makes "done" prove itself: without a commit link the
/// transition 409s naming the missing key (not ticket ids), and passes once the
/// link is set. An empty value must not satisfy it — a blank string would make
/// the proof a formality.
#[tokio::test]
async fn has_link_guard_requires_proof_before_done() {
    let app = TestApp::spawn().await;
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "proof",
                "name": "Proof required",
                "workflow": {
                    "name": "proof-wf",
                    "initial": "ready",
                    "states": [
                        { "id": "ready", "category": "todo", "claimable": true },
                        { "id": "review", "category": "review" },
                        { "id": "done", "category": "done", "terminal": true },
                        { "id": "cancelled", "category": "cancelled", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "ready", "to": "review" },
                        { "from": "ready", "to": "cancelled" },
                        { "from": "review", "to": "done", "requires": ["guard:has_link:commit"] },
                        { "from": "review", "to": "cancelled" }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "proof", "title": "Work that must prove itself" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap().to_string();
    let (s, b) = app.transition(&app.admin, &id, "review").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // No commit link: blocked, and the error teaches which key is missing.
    let (s, body) = app.transition(&app.admin, &id, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.guard");
    assert_eq!(body["details"]["guard"], "has_link:commit");
    // The subject is this ticket, so no offender list is invented.
    assert!(
        body["details"]["offending_tickets"].is_null(),
        "has_link failures must not name ticket ids: {body}"
    );
    let msg = body["message"].as_str().unwrap();
    assert!(msg.contains("commit"), "{msg}");
    let remedy = body["remedy"].as_str().unwrap();
    assert!(remedy.contains("takomo_link"), "{remedy}");

    // A blank value is not proof.
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "commit": "   " } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = app.transition(&app.admin, &id, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.guard");

    // A real sha satisfies it.
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "commit": "5caea2a0f3b91c7d4e28a6b5f0c1d9e8a7b6c5d4" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = app.transition(&app.admin, &id, "done").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn event_cursor_orders_and_longpoll_wakes() {
    let app = TestApp::spawn().await;
    app.create_ticket("Event log seed one").await;
    app.create_ticket("Event log seed two").await;

    // Cursor read: strictly increasing seqs, cursor = last seq.
    let (s, page) = app.get(&app.admin, "/v1/events?since=0").await;
    assert_eq!(s, StatusCode::OK);
    let events = page["events"].as_array().unwrap();
    assert!(events.len() >= 3, "workflow_changed + 2x created expected");
    let seqs: Vec<i64> = events.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
    for pair in seqs.windows(2) {
        assert!(pair[0] < pair[1], "seqs must strictly increase: {seqs:?}");
    }
    let cursor = page["cursor"].as_i64().unwrap();
    assert_eq!(cursor, *seqs.last().unwrap());

    // Resuming from the cursor yields nothing (wait=0).
    let (_, page2) = app
        .get(&app.admin, &format!("/v1/events?since={cursor}"))
        .await;
    assert!(page2["events"].as_array().unwrap().is_empty());
    assert_eq!(page2["cursor"].as_i64().unwrap(), cursor);

    // Long-poll: a waiting reader is woken by the next write.
    let waiter = {
        let client = app.client.clone();
        let base = app.base.clone();
        let token = app.admin.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let resp = client
                .get(format!("{base}/v1/events?since={cursor}&wait=15"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap();
            (start.elapsed(), resp.json::<Value>().await.unwrap())
        })
    };
    tokio::time::sleep(Duration::from_millis(400)).await;
    let id = app.create_ticket("Long poll wake trigger").await;

    let (elapsed, page3) = waiter.await.unwrap();
    assert!(
        elapsed < Duration::from_secs(10),
        "long-poll should wake promptly, took {elapsed:?}"
    );
    let events = page3["events"].as_array().unwrap();
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .any(|e| e["kind"] == "created" && e["ticket"] == id.as_str()));
    assert!(page3["cursor"].as_i64().unwrap() > cursor);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ready_claim_longpoll_wakes_on_new_work() {
    let app = TestApp::spawn().await;
    let waiter = {
        let client = app.client.clone();
        let base = app.base.clone();
        let token = app.worker.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let resp = client
                .post(format!("{base}/v1/ready/claim"))
                .bearer_auth(token)
                .json(&json!({ "project": "tp", "wait_seconds": 15 }))
                .send()
                .await
                .unwrap();
            let status = resp.status();
            (
                start.elapsed(),
                status,
                resp.json::<Value>().await.unwrap_or(Value::Null),
            )
        })
    };
    tokio::time::sleep(Duration::from_millis(400)).await;
    let id = app
        .create_ticket("Work arriving while a worker waits")
        .await;
    // brief -> spec: spec is claimable in factory-default, so this wakes the
    // waiting claimer.
    let (s, b) = app.transition(&app.human, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (elapsed, status, body) = waiter.await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "waiter should get the ticket: {body}"
    );
    assert_eq!(body["id"], id.as_str());
    assert_eq!(body["state"], "spec");
    assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
}

#[tokio::test]
async fn write_rate_limit_returns_429_with_retry_after() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Rate limit target").await;

    // The store the server owns is not reachable from here, so mint through a
    // second connection to the same DB file — the CLI's root of trust.
    let tight = app.mint_limited("agent:chatty", &["read", "write"], None, 3);

    let mut last = None;
    for i in 0..4 {
        let resp = app
            .authed(Method::POST, &tight, &format!("/v1/tickets/{id}/comments"))
            .json(&json!({ "body": format!("comment {i}") }))
            .send()
            .await
            .unwrap();
        last = Some(resp);
    }
    let resp = last.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: i64 = resp
        .headers()
        .get("Retry-After")
        .expect("Retry-After header")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry_after));
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "rate.limited");
    let message = body["message"].as_str().expect("message");
    assert!(
        message.contains("write budget of 3 writes/minute"),
        "429 names the budget the caller actually spent: {message}"
    );
    assert!(
        message.contains("reads are free"),
        "429 tells the caller reads still work: {message}"
    );
    assert!(
        body["remedy"].as_str().is_some_and(|r| r.contains("Wait")),
        "429 carries a remedy: {body}"
    );

    // …and that is true: a GET is not charged, so reads keep working while the
    // write budget is exhausted.
    let (status, _) = app.get(&tight, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "reads stay free while writes are limited"
    );
}

#[tokio::test]
async fn patch_rejects_state_and_unknown_fields() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Patch teaching test").await;

    let (s, body) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "state": "done" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "patch.state_not_patchable");
    assert!(body["remedy"].as_str().unwrap().contains("/transition"));

    // metadata_merge with RFC 7386 delete semantics.
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "metadata_merge": { "test.keep": "yes", "test.drop": "tmp" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "metadata_merge": { "test.drop": null } }),
        )
        .await;
    assert_eq!(body["metadata"]["test.keep"], "yes");
    assert!(body["metadata"].get("test.drop").is_none());
}

#[tokio::test]
async fn non_holder_writes_restricted_while_claimed() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Claimed-ticket write restrictions").await;
    app.to_ready(&id).await;
    app.claim(&id).await;

    // Non-holder title patch: 409 claim.held.
    let (s, body) = app
        .patch(
            &app.worker2,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "stolen" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "claim.held");

    // Non-holder may merge metadata under its own namespace.
    let (s, _) = app
        .patch(
            &app.worker2,
            &format!("/v1/tickets/{id}"),
            json!({ "metadata_merge": { "agent:w2.note": "observed" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // ...but not under someone else's namespace.
    let (s, _) = app
        .patch(
            &app.worker2,
            &format!("/v1/tickets/{id}"),
            json!({ "metadata_merge": { "agent:w1.note": "forged" } }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Comments stay open to everyone.
    let (s, _) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{id}/comments"),
            json!({ "body": "fyi" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // Holder must echo the fence even for its own patches.
    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "renamed by holder" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "fence.required");
}

#[tokio::test]
async fn sse_stream_delivers_events() {
    let app = TestApp::spawn().await;
    app.create_ticket("SSE seed ticket").await;

    let resp = app
        .authed(Method::GET, &app.admin, "/v1/events/stream?since=0")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));

    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("SSE first chunk within 5s")
        .expect("stream item")
        .expect("bytes");
    let text = String::from_utf8_lossy(&chunk);
    assert!(
        text.contains("id:"),
        "SSE frame should carry seq ids: {text}"
    );
    assert!(
        text.contains("created"),
        "SSE frame should carry the created event: {text}"
    );
}

#[tokio::test]
async fn project_scoped_token_is_fenced_in() {
    let app = TestApp::spawn().await;
    app.open_store()
        .create_project("other", "Other Project", None, "test:setup")
        .unwrap();
    let scoped = app.mint("agent:tp-only", &["read", "write"], Some(&["tp"]));

    let (s, body) = app
        .post(
            &scoped,
            "/v1/tickets",
            json!({ "project": "other", "title": "Reach across" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.project");

    let (s, _) = app
        .post(
            &scoped,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Stay inside" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn stale_fence_bounces_even_on_unclaimed_ticket() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Zombie fence on unclaimed ticket").await;
    app.to_ready(&id).await;

    // Claim/release twice: fence advances to 2, ticket ends unclaimed.
    for expected_fence in 1..=2i64 {
        let fence = app.claim(&id).await;
        assert_eq!(fence, expected_fence);
        let (s, _) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{id}/release"),
                json!({ "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::NO_CONTENT);
    }

    // A zombie echoing fence 1 must bounce on PATCH even though the ticket is
    // unclaimed now.
    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "zombie write", "fence": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "fence.stale");

    // Same on transition.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "cancelled", "fence": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.stale");

    // The current fence (2) is accepted on an unclaimed ticket.
    let (s, _) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "current fence ok", "fence": 2 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn deps_respect_claims_and_fences() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Deps under claim").await;
    let other = app.create_ticket("A blocker candidate").await;
    app.to_ready(&id).await;
    let fence = app.claim(&id).await;

    // Non-holder cannot add a dep to a claimed ticket.
    let (s, body) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{id}/deps"),
            json!({ "blocked_by": other }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "claim.held");

    // Holder without the fence is refused too.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/deps"),
            json!({ "blocked_by": other }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.required");

    // Holder with the fence succeeds, and the ticket version bumps.
    let (_, before) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    let v_before = before["version"].as_i64().unwrap();
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/deps"),
            json!({ "blocked_by": other, "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (_, after) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert!(after["version"].as_i64().unwrap() > v_before);
    assert_eq!(after["blocked_by"][0], other.as_str());

    // Removal follows the same rule (fence via query param).
    let (s, _) = app
        .delete(
            &app.worker,
            &format!("/v1/tickets/{id}/deps?blocked_by={other}"),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "holder needs fence on delete too");
    let (s, _) = app
        .delete(
            &app.worker,
            &format!("/v1/tickets/{id}/deps?blocked_by={other}&fence={fence}"),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn autoland_twin_edge_selects_most_actionable_error() {
    let app = TestApp::spawn().await;
    // Second project with a twin review->done edge: human gate OR autoland gate.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "auto",
                "name": "Autoland",
                "workflow": {
                    "name": "auto-wf",
                    "initial": "ready",
                    "states": [
                        { "id": "ready", "category": "todo", "claimable": true },
                        { "id": "review", "category": "review" },
                        { "id": "done", "category": "done", "terminal": true },
                        { "id": "cancelled", "category": "cancelled", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "ready", "to": "review" },
                        { "from": "ready", "to": "cancelled" },
                        { "from": "review", "to": "done", "requires": ["scope:human", "guard:no_open_children"] },
                        { "from": "review", "to": "done", "requires": ["scope:autoland", "guard:no_open_children"] },
                        { "from": "review", "to": "cancelled", "requires": ["scope:human"] }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    let orch = app.mint("orch:main", &["read", "write", "autoland"], None);

    let (s, t) = app
        .post(
            &orch,
            "/v1/tickets",
            json!({ "project": "auto", "title": "Autoland candidate" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let id = t["id"].as_str().unwrap().to_string();
    let (s, child) = app
        .post(
            &orch,
            "/v1/tickets",
            json!({ "project": "auto", "title": "Open child", "parent": id }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let child_id = child["id"].as_str().unwrap().to_string();
    let (s, b) = app.transition(&orch, &id, "review").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // Autoland token + open child: the autoland edge is authorized, so the
    // most actionable failure is the guard 409 — not a 403.
    let (s, body) = app.transition(&orch, &id, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.guard");
    assert_eq!(body["details"]["offending_tickets"][0], child_id.as_str());

    // A token with neither scope gets the 403.
    let (s, body) = app.transition(&app.worker, &id, "done").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "transition.scope");

    // Close the child; autoland lands it without any human scope.
    let (s, b) = app.transition(&orch, &child_id, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, body) = app.transition(&orch, &id, "done").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done");
}

#[tokio::test]
async fn workflow_upload_rejects_typos_and_terminal_exits() {
    let app = TestApp::spawn().await;

    // Misspelled 'requires' must be a 422, not a silently dropped gate.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "typo",
                "name": "Typo",
                "workflow": {
                    "name": "typo-wf",
                    "initial": "open",
                    "states": [
                        { "id": "open", "category": "todo" },
                        { "id": "done", "category": "done", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "open", "to": "done", "require": ["scope:human"] }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.parse");

    // Outgoing transitions from terminal states are refused.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "reopen",
                "name": "Reopen",
                "workflow": {
                    "name": "reopen-wf",
                    "initial": "open",
                    "states": [
                        { "id": "open", "category": "todo" },
                        { "id": "done", "category": "done", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "open", "to": "done" },
                        { "from": "done", "to": "open" }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.invalid");
    assert!(body["message"].as_str().unwrap().contains("terminal"));
}

// Pilot finding A: human approval is authoritative over a held claim. A
// `scope:human` transition performed by a human-scoped caller must succeed even
// while a different actor holds the lease, and must auto-release that lease.
#[tokio::test]
async fn human_transition_overrides_held_claim_and_auto_releases() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Human gate over a worker's lease").await;

    // Move it into `spec` (claimable) and let the worker take the lease.
    let (s, b) = app.transition(&app.worker, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "brief->spec: {b}");
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "worker claim: {lease}");
    assert_eq!(lease["holder"], "agent:w1");

    // The human (a DIFFERENT actor) approves spec->ready — a scope:human edge —
    // while the worker still holds the lease. It must succeed, not 409.
    let (s, body) = app.transition(&app.human, &id, "ready").await;
    assert_eq!(
        s,
        StatusCode::OK,
        "human override should win over the lease: {body}"
    );
    assert_eq!(body["state"], "ready");
    // The lease is gone: the human transition superseded and released it.
    assert!(
        body["claim"].is_null(),
        "claim must be auto-released: {body}"
    );

    // Both a `transitioned` and a `released` event landed, attributed to the
    // human, with the superseding reason on the release.
    let (_, trans) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=transitioned"),
        )
        .await;
    let tev = trans["events"].as_array().unwrap();
    let approve = tev
        .iter()
        .find(|e| e["payload"]["to"] == "ready")
        .expect("spec->ready transitioned event");
    assert_eq!(approve["actor"], "human:reviewer");
    assert_eq!(approve["payload"]["auto_released"], true);

    let (_, rel) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=released"),
        )
        .await;
    let rev = rel["events"].as_array().unwrap();
    let superseded = rev
        .iter()
        .find(|e| e["payload"]["reason"] == "superseded by human transition")
        .expect("a `released` event superseding the worker's lease");
    assert_eq!(superseded["actor"], "human:reviewer");
}

// The holder lock is unchanged for ordinary `claim`-required transitions: a
// non-holder without the human scope is still blocked (finding A is scoped to
// human-required edges only).
#[tokio::test]
async fn holder_lock_still_blocks_non_holder_ordinary_transition() {
    let app = TestApp::spawn().await;
    let id = app
        .create_ticket("Ordinary claim edge keeps holder lock")
        .await;
    app.to_ready(&id).await;

    // Worker 1 holds the lease.
    app.claim(&id).await;

    // Worker 2 (non-holder, no human scope) attempts ready->implementing, an
    // ordinary `claim`-required edge. The holder lock still blocks it.
    let (s, body) = app.transition(&app.worker2, &id, "implementing").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "claim.held");
}

// Pilot finding B: validation order is legality -> scope -> claim/fence, so the
// headline error names the first real blocker rather than a fencing complaint.
#[tokio::test]
async fn error_ordering_legality_and_scope_precede_fence() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Error ordering over a held lease").await;
    let (s, b) = app.transition(&app.worker, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "brief->spec: {b}");
    // Worker holds the lease but echoes NO fence on the attempts below; before
    // the fix both would have surfaced `fence.required`.
    app.claim(&id).await;

    // (scope before fence) The worker attempts spec->ready, a human gate it
    // lacks the scope for: a 403 transition.scope, not fence.required.
    let (s, body) = app.transition(&app.worker, &id, "ready").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "transition.scope");
    assert!(body["message"].as_str().unwrap().contains("human"));

    // (legality before fence) The worker attempts an undefined spec->done edge:
    // transition.illegal, not fence.required.
    let (s, body) = app.transition(&app.worker, &id, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.illegal");
    assert!(body["allowed_transitions"].is_array());
}

// --- token & identity over HTTP ---------------------------------------------

#[tokio::test]
async fn token_mint_list_revoke_and_whoami_over_http() {
    let app = TestApp::spawn().await;

    // Admin mints a project-scoped read,write token over HTTP.
    let (s, minted) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:http", "scopes": ["read", "write"], "projects": ["tp"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{minted}");
    let plaintext = minted["token"]
        .as_str()
        .expect("plaintext token shown once");
    assert!(
        plaintext.starts_with("tk_"),
        "token should be a tk_ plaintext: {minted}"
    );
    let token_id = minted["id"].as_str().expect("token id").to_string();
    assert_eq!(minted["actor"], "agent:http");
    assert_eq!(minted["scopes"], json!(["read", "write"]));
    assert_eq!(minted["projects"], json!(["tp"]));
    // The mint response must never leak the at-rest hash.
    assert!(
        minted.get("hash").is_none(),
        "hash must not be returned: {minted}"
    );

    // The freshly minted token authenticates and can create work in its project.
    let plaintext = plaintext.to_string();
    let (s, t) = app
        .post(
            &plaintext,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Minted-token ticket" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "minted token should work: {t}");

    // whoami reflects the caller's identity (any valid token may call it).
    let (s, who) = app.get(&plaintext, "/v1/whoami").await;
    assert_eq!(s, StatusCode::OK, "{who}");
    assert_eq!(who["actor"], "agent:http");
    assert_eq!(who["scopes"], json!(["read", "write"]));
    assert_eq!(who["projects"], json!(["tp"]));

    // Admin lists tokens: metadata only — never plaintext or hash.
    let (s, rows) = app.get(&app.admin, "/v1/tokens").await;
    assert_eq!(s, StatusCode::OK);
    let rows = rows.as_array().expect("token list array");
    let row = rows
        .iter()
        .find(|r| r["id"] == token_id.as_str())
        .expect("minted token appears in the list");
    assert_eq!(row["actor"], "agent:http");
    assert!(row["revoked_at"].is_null(), "not yet revoked: {row}");
    for r in rows {
        assert!(
            r.get("token").is_none(),
            "list must not leak plaintext: {r}"
        );
        assert!(r.get("hash").is_none(), "list must not leak the hash: {r}");
    }

    // Admin revokes it; the minted token then fails auth.
    let (s, _) = app
        .delete(&app.admin, &format!("/v1/tokens/{token_id}"))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (s, body) = app.get(&plaintext, "/v1/whoami").await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "revoked token must fail: {body}"
    );
    assert_eq!(body["code"], "auth.invalid");

    // Revoking an unknown id is a teaching 404.
    let (s, _) = app.delete(&app.admin, "/v1/tokens/tok_nope").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn token_admin_endpoints_require_admin_scope() {
    let app = TestApp::spawn().await;

    // A non-admin (read,write) token is 403'd on every token-admin endpoint.
    let (s, body) = app
        .post(
            &app.worker,
            "/v1/tokens",
            json!({ "actor": "agent:x", "scopes": ["read"] }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.scope");

    let (s, body) = app.get(&app.worker, "/v1/tokens").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.scope");

    let (s, _) = app.delete(&app.worker, "/v1/tokens/tok_whatever").await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // But whoami is open to any valid token, and admin sees projects "*".
    let (s, who) = app.get(&app.worker, "/v1/whoami").await;
    assert_eq!(s, StatusCode::OK, "{who}");
    assert_eq!(who["actor"], "agent:w1");
    let (s, admin_who) = app.get(&app.admin, "/v1/whoami").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(admin_who["projects"], "*");
    assert!(admin_who["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "admin"));
}

#[tokio::test]
async fn token_create_validates_body() {
    let app = TestApp::spawn().await;

    // Missing scopes.
    let (s, body) = app
        .post(&app.admin, "/v1/tokens", json!({ "actor": "agent:y" }))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "token.scopes");

    // "*" projects means all projects (None internally).
    let (s, minted) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "orch:all", "scopes": ["read", "write", "admin"], "projects": "*",
                    "expires_seconds": 3600 }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{minted}");
    assert_eq!(minted["projects"], "*");
    assert!(minted["expires_at"].is_string(), "expiry echoed: {minted}");

    // Empty projects array is rejected (use "*" for all).
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:z", "scopes": ["read"], "projects": [] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "token.projects");
}

// --- Tier 3 DX polish -------------------------------------------------------

// similar[] scores by title-token overlap (Jaccard) plus a type-match nudge,
// thresholded so a real near-duplicate surfaces with its score and matched
// terms, while an incidental single shared word does not cry wolf.
#[tokio::test]
async fn similar_is_scored_and_thresholded() {
    let app = TestApp::spawn().await;
    let base = app.create_ticket("Optimize the database indexes").await;
    // Incidental single-word overlap ("database") — must NOT surface later.
    app.create_ticket("Database migration tooling script").await;

    let (s, body) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Optimize the database indexes for reads" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let similar = body["similar"].as_array().expect("similar array");

    // Exactly the genuine near-duplicate surfaces.
    assert_eq!(
        similar.len(),
        1,
        "only the real dupe should surface: {body}"
    );
    let hit = &similar[0];
    assert_eq!(hit["id"], base.as_str());
    let score = hit["score"].as_f64().expect("numeric score");
    assert!(score > 0.5, "near-duplicate should score high, got {score}");
    let terms: Vec<&str> = hit["matched_terms"]
        .as_array()
        .expect("matched_terms array")
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert!(terms.contains(&"database"), "matched_terms: {terms:?}");
    assert!(terms.contains(&"optimize"), "matched_terms: {terms:?}");
    assert!(hit["type"].is_string(), "type echoed: {hit}");
}

// A fence greater than the current one was never issued -> fence.invalid (a
// client bug), distinct from a stale (superseded, lower) fence.
#[tokio::test]
async fn fence_greater_than_current_is_invalid_not_stale() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Fence-never-issued test").await;
    app.to_ready(&id).await;

    // Claim: fence becomes 1.
    let fence = app.claim(&id).await;
    assert_eq!(fence, 1);

    // Holder echoes a fence the store never issued (fabricated, too high).
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({ "fence": fence + 5 }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.invalid", "{body}");
    assert!(body["message"].as_str().unwrap().contains("never issued"));
    assert_eq!(body["details"]["current_fence"], fence);

    // The correct current fence still works.
    let (s, ok) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({ "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{ok}");
}

// PATCH links merges per key instead of replacing the whole object; a null
// value deletes just that key.
#[tokio::test]
async fn links_patch_merges_per_key() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Links merge test").await;

    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "branch": "feat/x" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["links"]["branch"], "feat/x");

    // Add a second key — the first must survive (not be clobbered).
    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "pr": "https://example.test/pr/1" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["links"]["branch"], "feat/x",
        "existing key must persist: {b}"
    );
    assert_eq!(b["links"]["pr"], "https://example.test/pr/1");

    // null deletes just that key.
    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "branch": null } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(
        b["links"].get("branch").is_none(),
        "branch should be deleted: {b}"
    );
    assert_eq!(
        b["links"]["pr"], "https://example.test/pr/1",
        "pr should remain: {b}"
    );

    // Non-string, non-null value is rejected.
    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "pr": 5 } }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.links");
}

// GET /tickets?q= is tokenized: every term must match, across title OR body.
#[tokio::test]
async fn search_is_tokenized_all_terms_match() {
    let app = TestApp::spawn().await;
    // Title carries one term, body the other.
    let (s, _t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Refactor the auth layer", "body": "replace the token cache" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    app.create_ticket("Unrelated docs cleanup").await;

    // Both terms present (one in title, one in body) -> match.
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&q=auth+token")
        .await;
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "both terms should match one ticket: {list}");
    assert!(items[0]["title"].as_str().unwrap().contains("auth"));

    // One term matches, the other does not -> no results (AND semantics).
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&q=auth+nonexistentword")
        .await;
    assert!(
        list["items"].as_array().unwrap().is_empty(),
        "unmatched term must exclude the row: {list}"
    );

    // Case-insensitive.
    let (_, list) = app.get(&app.admin, "/v1/tickets?project=tp&q=AUTH").await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1, "{list}");
}

// GET /v1/export streams JSONL of tickets with their comments and deps, and the
// output round-trips (every line is a self-contained JSON ticket).
#[tokio::test]
async fn export_streams_jsonl_with_comments_and_deps() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("Exportable ticket A").await;
    let b = app.create_ticket("Exportable ticket B blocks A").await;
    // A blocked_by B.
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/deps"),
            json!({ "blocked_by": b }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    // A comment on A.
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/comments"),
            json!({ "body": "a note" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (status, ctype, text) = app.get_raw(&app.admin, "/v1/export?project=tp").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        ctype.starts_with("application/x-ndjson"),
        "content-type: {ctype}"
    );

    let lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is valid JSON"))
        .collect();
    assert_eq!(lines.len(), 2, "two tickets exported: {text}");

    let line_a = lines
        .iter()
        .find(|l| l["id"] == a.as_str())
        .expect("A present");
    assert_eq!(
        line_a["blocked_by"][0],
        b.as_str(),
        "deps in export: {line_a}"
    );
    let comments = line_a["comments"].as_array().expect("comments array");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["body"], "a note");

    // A read-only (no write) token can export; project scoping is honored.
    let reader = app.mint("agent:reader", &["read"], Some(&["tp"]));
    let (status, _c, text) = app.get_raw(&reader, "/v1/export?project=tp").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text.lines().filter(|l| !l.trim().is_empty()).count(), 2);
}

// GET /v1/metrics reports ticket counts by state and category per project, open
// claims, and the event total; a scoped token only sees its projects.
#[tokio::test]
async fn metrics_counts_by_state_category_and_claims() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("Metrics ticket one").await;
    app.create_ticket("Metrics ticket two").await;
    // Drive one into a claimed state.
    app.to_ready(&a).await;
    app.claim(&a).await;

    let (s, m) = app.get(&app.admin, "/v1/metrics").await;
    assert_eq!(s, StatusCode::OK, "{m}");
    let tp = &m["projects"]["tp"];
    assert_eq!(tp["total"], 2, "two tickets in tp: {m}");
    assert_eq!(tp["open_claims"], 1, "one open claim: {m}");
    // a was driven to `ready` (todo category) and claimed; the other is `brief`.
    assert_eq!(tp["by_state"]["brief"], 1, "{m}");
    assert_eq!(tp["by_state"]["ready"], 1, "{m}");
    // brief and ready are both `todo`-category in factory-default.
    assert_eq!(tp["by_category"]["todo"], 2, "{m}");
    assert_eq!(m["totals"]["tickets"], 2, "{m}");
    assert!(
        m["totals"]["events"].as_i64().unwrap() > 0,
        "events counted: {m}"
    );

    // A token scoped to a different project sees no tp counts.
    app.open_store()
        .create_project("solo", "Solo", None, "test:setup")
        .unwrap();
    let scoped = app.mint("agent:solo", &["read", "write"], Some(&["solo"]));
    let (s, m) = app.get(&scoped, "/v1/metrics").await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        m["projects"].get("tp").is_none(),
        "scoped token must not see tp: {m}"
    );
    assert_eq!(m["totals"]["tickets"], 0, "solo has no tickets yet: {m}");
}

// --- project delete ---------------------------------------------------------

// DELETE /v1/projects/{id} cascade-removes the project and all of its tickets,
// comments, deps, and events in one shot. 404 for an unknown project.
#[tokio::test]
async fn project_delete_cascades_tickets_and_events() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("Doomed ticket A").await;
    let b = app.create_ticket("Doomed ticket B blocks A").await;
    // A blocked_by B, and a comment on A — all must vanish with the project.
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/deps"),
            json!({ "blocked_by": b }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/comments"),
            json!({ "body": "last words" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // Sanity: events exist for the project before deletion.
    let (_, before) = app.get(&app.admin, "/v1/events?since=0&project=tp").await;
    assert!(!before["events"].as_array().unwrap().is_empty());

    // Unknown project -> 404.
    let (s, body) = app.delete(&app.admin, "/v1/projects/ghost").await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "notfound.project");

    // Delete succeeds with 204 (no active claims).
    let (s, _body) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // The project is gone.
    let (_, projects) = app.get(&app.admin, "/v1/projects").await;
    let ids: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&"tp"), "project should be gone: {ids:?}");

    // Its tickets are gone (404 on GET).
    let (s, _) = app.get(&app.admin, &format!("/v1/tickets/{a}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Its per-project events are gone (the audit event is store-level, project=null).
    let (_, after) = app.get(&app.admin, "/v1/events?since=0&project=tp").await;
    assert!(
        after["events"].as_array().unwrap().is_empty(),
        "project events must be cleared: {after}"
    );

    // Deleting again is a 404 (idempotent-ish: it is truly gone).
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// An active (unexpired) claim blocks delete with a teaching 409; ?force=true
// overrides it.
#[tokio::test]
async fn project_delete_refuses_active_claim_unless_forced() {
    let app = TestApp::spawn().await;
    let id = app
        .create_ticket("Claimed while its project is deleted")
        .await;
    app.to_ready(&id).await;
    // Long lease so it stays active across the test.
    app.claim_ttl(&app.worker, &id, Some(900)).await;

    // Without force: 409 naming the active claim.
    let (s, body) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "project.active_claims");
    assert_eq!(body["details"]["active_claims"], 1);
    assert!(body["message"].as_str().unwrap().contains("force=true"));

    // The project and its ticket still exist (the refusal changed nothing).
    let (s, _) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::OK);

    // With ?force=true: it deletes anyway.
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp?force=true").await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// A released (or expired) claim does not block delete: the guard is about
// *active* leases only.
#[tokio::test]
async fn project_delete_allows_after_claim_released() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Released before project delete").await;
    app.to_ready(&id).await;
    let fence = app.claim(&id).await;
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/release"),
            json!({ "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // No active claim now: plain delete works without force.
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

// Only admin scope may delete a project; read/write is 403'd.
#[tokio::test]
async fn project_delete_requires_admin_scope() {
    let app = TestApp::spawn().await;
    app.create_ticket("Guarded by admin scope").await;

    let (s, body) = app.delete(&app.worker, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.scope");

    // The human scope is not admin either.
    let (s, body) = app.delete(&app.human, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.scope");

    // The project survived the rejected attempts.
    let (_, projects) = app.get(&app.admin, "/v1/projects").await;
    let ids: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"tp"), "project must survive a 403: {ids:?}");
}

// Deleting one project leaves other projects — and cross-project dep edges into
// the deleted project — clean.
#[tokio::test]
async fn project_delete_is_scoped_and_clears_cross_project_deps() {
    let app = TestApp::spawn().await;
    // Second project whose ticket is blocked_by a ticket in `tp`.
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "keep", "name": "Keeper" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "keep", "title": "Survivor ticket" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let keeper = t["id"].as_str().unwrap().to_string();
    let blocker = app.create_ticket("tp ticket blocking a keep ticket").await;
    // keep-ticket blocked_by a tp-ticket (blocked_by is not project-scoped).
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{keeper}/deps"),
            json!({ "blocked_by": blocker }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // Delete tp; keep must be untouched and the dangling dep edge cleared.
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (_, projects) = app.get(&app.admin, "/v1/projects").await;
    let ids: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"keep") && !ids.contains(&"tp"), "{ids:?}");

    // The keep ticket survives with its dangling blocker edge removed.
    let (s, kt) = app.get(&app.admin, &format!("/v1/tickets/{keeper}")).await;
    assert_eq!(s, StatusCode::OK, "{kt}");
    assert!(
        kt["blocked_by"].as_array().unwrap().is_empty(),
        "cross-project dep into a deleted project must be cleared: {kt}"
    );
}

// Regression: deleting a project that has ever carried a question, a tag, an
// answer link or a promotion used to 500. Each of those tables holds a real
// REFERENCES into questions/tickets/projects, so a cascade that skips them hits
// the immediate foreign-key check and aborts the whole transaction. The other
// delete tests never create any of them, which is how this survived.
#[tokio::test]
async fn project_delete_cascades_questions_tags_grants_and_promotions() {
    let app = TestApp::spawn().await;

    // A tagged ticket — creating with `tags` auto-registers the `person:ada`
    // stub in the project tag registry (tags REFERENCES projects(id)) — plus a
    // registry entry no ticket refers to.
    let (s, tagged) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Tagged before its project is deleted", "tags": ["person:ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tagged}");
    let tagged = tagged["id"].as_str().unwrap().to_string();
    let (s, reg) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "component", "handle": "billing" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{reg}");

    // A promotion (REFERENCES tickets(id)).
    let (s, promo) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{tagged}/promote"),
            json!({ "target": "staging" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{promo}");

    // A blocking question (REFERENCES projects(id) AND tickets(id)) with a
    // follow-up thread (question_messages) and a minted answer link
    // (answer_grants), both of which REFERENCE questions(id).
    let asked = app.create_ticket("Parked on a human decision").await;
    let fence = app.to_implementing(&asked).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": asked, "kind": "confirm", "title": "OK to drop the table?", "fence": fence }),
        )
        .await;
    let (s, fu) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/followup"),
            json!({ "message": "How many rows and how long is the lock?" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{fu}");
    let (s, rep) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/reply"),
            json!({ "message": "40k rows, ~2s lock, reversible." }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{rep}");
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");

    // Asking parked the ticket and released the lease, so no force is needed:
    // this must be a clean 204, not a 500 from an aborted transaction.
    let (s, body) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(
        s,
        StatusCode::NO_CONTENT,
        "delete must cascade the question/tag/grant/promotion tables: {body}"
    );

    // Nothing survives in the tables the cascade used to skip. Read them
    // straight from SQLite: some have no list endpoint once the project is gone.
    let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
    for table in [
        "questions",
        "question_messages",
        "answer_grants",
        "tags",
        "promotions",
    ] {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0, "{table} rows survived the project delete");
    }

    // The audit event accounts for every table it cleared.
    let payload: String = conn
        .query_row(
            "SELECT payload FROM events WHERE kind = 'project_deleted'",
            [],
            |r| r.get(0),
        )
        .expect("audit event");
    let payload: Value = serde_json::from_str(&payload).expect("payload json");
    let deleted = &payload["deleted"];
    assert_eq!(deleted["tickets"], 2, "{deleted}");
    assert_eq!(deleted["questions"], 1, "{deleted}");
    assert_eq!(deleted["question_messages"], 2, "{deleted}");
    assert_eq!(deleted["answer_grants"], 1, "{deleted}");
    assert_eq!(deleted["tags"], 2, "{deleted}");
    assert_eq!(deleted["promotions"], 1, "{deleted}");

    // And the project itself is gone.
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn roadmap_rolls_up_epic_subtree() {
    let app = TestApp::spawn().await;

    // epic
    //  |- child_a
    //  |    \- grandchild   (done)
    //  |- child_a           (done, after its only child is done)
    //  |- child_b           (brief)
    //  |- child_c           (brief)
    //  \- child_d           (brief)
    let epic = app.create_typed("Ship the widget", "epic", None).await;
    let child_a = app.create_typed("Backend", "task", Some(&epic)).await;
    let grandchild = app
        .create_typed("Backend subtask", "task", Some(&child_a))
        .await;
    let _child_b = app.create_typed("Frontend", "task", Some(&epic)).await;
    let _child_c = app.create_typed("Docs", "task", Some(&epic)).await;
    let _child_d = app.create_typed("QA", "task", Some(&epic)).await;

    // Finish the grandchild first, then child_a (its subtree is now clear).
    app.drive_to_done(&grandchild).await;
    app.drive_to_done(&child_a).await;

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["project"], "tp");
    let epics = body["epics"].as_array().expect("epics array");
    assert_eq!(epics.len(), 1, "one epic expected: {body}");
    let e = &epics[0];
    assert_eq!(e["id"], epic.as_str());

    // Full descendant subtree = child_a + grandchild + child_b + child_c + child_d = 5.
    assert_eq!(e["total"], 5, "subtree total: {e}");
    // Two are done (child_a + grandchild); percent = round(2/5*100) = 40.
    assert_eq!(e["done"], 2, "done count: {e}");
    assert_eq!(e["percent"], 40, "percent: {e}");
    assert_eq!(e["by_category"]["done"], 2, "by_category done: {e}");
    assert_eq!(e["by_category"]["todo"], 3, "by_category todo: {e}");
    assert_eq!(e["by_state"]["done"], 2, "by_state done: {e}");
    assert_eq!(e["by_state"]["brief"], 3, "by_state brief: {e}");

    // The epic itself is the container, not counted in its own rollup.
    assert!(
        e["by_state"].get("spec").is_none(),
        "epic not self-counted: {e}"
    );

    // Unknown project -> 404.
    let (status, body) = app.get(&app.admin, "/v1/projects/nope/roadmap").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

// The `unparented` bucket catches every way a ticket can end up outside all
// epics: no parent at all, a chain of non-epic ancestors, and a parent id that
// points at a row that is not there. Counts must stay coherent — with flat
// epics, every non-epic ticket lands in exactly one bucket.
#[tokio::test]
async fn roadmap_unparented_bucket_covers_every_orphan_shape() {
    let app = TestApp::spawn().await;

    // Two flat epics, so no ticket is counted by two epic subtrees.
    let epic_a = app.create_typed("Owned work", "epic", None).await;
    let owned_done = app.create_typed("Owned A", "task", Some(&epic_a)).await;
    let _owned_open = app.create_typed("Owned B", "task", Some(&epic_a)).await;
    app.drive_to_done(&owned_done).await;
    let _epic_b = app.create_typed("Planned work", "epic", None).await;

    // 1. No parent at all.
    let loose = app.create_typed("Loose task", "task", None).await;
    app.drive_to_done(&loose).await;
    // 2. A chain of non-epic ancestors: leaf -> mid -> (nothing).
    let mid = app.create_typed("Mid task", "task", None).await;
    let _leaf = app.create_typed("Leaf task", "task", Some(&mid)).await;
    // 3. A dangling parent: the row it points at does not exist.
    let dangling = app.create_typed("Dangling task", "task", None).await;
    app.force_parent(&dangling, "tp-nosuchrow");

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let u = &body["unparented"];

    // loose + mid + leaf + dangling = 4; the two epics themselves never count.
    assert_eq!(u["total"], 4, "unparented total: {body}");
    assert_eq!(u["done"], 1, "only the loose task is done: {body}");
    assert_eq!(u["percent"], 25, "round(1/4*100): {body}");
    assert_eq!(u["by_state"]["done"], 1, "{body}");
    assert_eq!(u["by_state"]["brief"], 3, "{body}");
    assert_eq!(u["by_category"]["done"], 1, "{body}");
    assert_eq!(u["by_category"]["todo"], 3, "{body}");
    assert!(
        u.get("id").is_none() && u.get("title").is_none() && u.get("state").is_none(),
        "the bucket is not a ticket: {u}"
    );

    // Coherence: epics are flat here, so every non-epic ticket is counted
    // exactly once across the epic subtrees plus the unparented bucket.
    let epic_total: i64 = body["epics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["total"].as_i64().unwrap())
        .sum();
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&limit=200")
        .await;
    let non_epics = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["type"] != "epic")
        .count() as i64;
    assert_eq!(
        epic_total + u["total"].as_i64().unwrap(),
        non_epics,
        "epic subtotals + unparented must account for every non-epic ticket: {body}"
    );
}

// A `parent` cycle is not reachable through the API, but a corrupted database
// can hold one. Both recursive walks use UNION, which stops at an already-seen
// id — the endpoint must answer rather than spin.
#[tokio::test]
async fn roadmap_terminates_on_parent_cycles() {
    let app = TestApp::spawn().await;

    // A cycle through an epic: epic -> p -> epic. The subtree walk revisits the
    // epic and must stop there.
    let epic = app.create_typed("Cyclic epic", "epic", None).await;
    let p = app.create_typed("Under epic", "task", Some(&epic)).await;
    app.force_parent(&epic, &p);

    // A cycle with no epic anywhere above it: x <-> y, plus a tail hanging off
    // it whose upward chain runs into the cycle forever.
    let x = app.create_typed("Free one", "task", None).await;
    let y = app.create_typed("Free two", "task", Some(&x)).await;
    app.force_parent(&x, &y);
    let _tail = app
        .create_typed("Tail of the cycle", "task", Some(&x))
        .await;

    let (status, body) = tokio::time::timeout(
        Duration::from_secs(10),
        app.get(&app.admin, "/v1/projects/tp/roadmap"),
    )
    .await
    .expect("roadmap must terminate on a parent cycle, not hang");
    assert_eq!(status, StatusCode::OK, "{body}");

    // The walk reaches p and, around the cycle, the epic itself — each once.
    let e = &body["epics"].as_array().unwrap()[0];
    assert_eq!(e["total"], 2, "cyclic subtree counted once each: {body}");
    // x, y and the tail never reach an epic upward, so all three are unparented
    // (the epic in the cycle is an epic, and epics never join the bucket).
    assert_eq!(body["unparented"]["total"], 3, "{body}");
}

// Each flag is a pure derivation over the epic's own state category and its
// subtree counts.
#[tokio::test]
async fn roadmap_flags_epic_state_contradictions() {
    let app = TestApp::spawn().await;

    // 1. done epic, open children: the child is cancelled (terminal, so the
    //    done guard passes) but not done, leaving done(0) < total(1).
    let e_done_open = app.create_typed("Shipped early", "epic", None).await;
    let stray = app
        .create_typed("Cancelled child", "task", Some(&e_done_open))
        .await;
    let (s, b) = app.transition(&app.human, &stray, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    app.drive_to_done(&e_done_open).await;

    // 2. open epic, all children done.
    let e_open_all_done = app.create_typed("Work finished", "epic", None).await;
    let child = app
        .create_typed("Only child", "task", Some(&e_open_all_done))
        .await;
    app.drive_to_done(&child).await;

    // 3. an epic with no descendants at all.
    let e_empty = app.create_typed("Filed ahead", "epic", None).await;

    // 4. empty *and* done: `empty_epic` fires, `done_with_open_children` must
    //    not — done < total is false when total is 0.
    let e_empty_done = app.create_typed("Empty and done", "epic", None).await;
    app.drive_to_done(&e_empty_done).await;

    // 5. a consistent epic: in progress with a mix of open children.
    let e_ok = app.create_typed("Business as usual", "epic", None).await;
    let ok_done = app.create_typed("Done bit", "task", Some(&e_ok)).await;
    let _ok_open = app.create_typed("Open bit", "task", Some(&e_ok)).await;
    app.drive_to_done(&ok_done).await;

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let flags = |id: &str| -> Vec<String> {
        body["epics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap_or_else(|| panic!("epic {id} missing: {body}"))["flags"]
            .as_array()
            .unwrap_or_else(|| panic!("epic {id} has no flags array: {body}"))
            .iter()
            .map(|f| f.as_str().unwrap().to_string())
            .collect()
    };

    assert_eq!(flags(&e_done_open), ["done_with_open_children"], "{body}");
    assert_eq!(
        flags(&e_open_all_done),
        ["open_with_all_children_done"],
        "{body}"
    );
    assert_eq!(flags(&e_empty), ["empty_epic"], "{body}");
    assert_eq!(
        flags(&e_empty_done),
        ["empty_epic"],
        "an empty done epic is empty only — done < total cannot hold at total 0: {body}"
    );
    assert!(
        flags(&e_ok).is_empty(),
        "a consistent epic carries no flags: {body}"
    );
}

#[tokio::test]
async fn deps_reverse_and_transitive_are_cycle_safe() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("A depends on B").await;
    let b = app.create_ticket("B depends on C").await;
    let c = app.create_ticket("C the root blocker").await;

    // A blocked_by B, B blocked_by C — a two-hop chain.
    for (t, dep) in [(&a, &b), (&b, &c)] {
        let (s, body) = app
            .post(
                &app.admin,
                &format!("/v1/tickets/{t}/deps"),
                json!({ "blocked_by": dep }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
    }

    fn node_ids(v: &Value) -> Vec<String> {
        let mut n: Vec<String> = v["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap().to_string())
            .collect();
        n.sort();
        n
    }
    fn edge_set(v: &Value) -> Vec<(String, String)> {
        let mut e: Vec<(String, String)> = v["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                (
                    x["ticket"].as_str().unwrap().to_string(),
                    x["blocked_by"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        e.sort();
        e
    }
    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }
    fn esorted(mut v: Vec<(String, String)>) -> Vec<(String, String)> {
        v.sort();
        v
    }

    // Direct (non-transitive) blocked_by on A: just A -> B.
    let (s, g) = app.get(&app.admin, &format!("/v1/tickets/{a}/deps")).await;
    assert_eq!(s, StatusCode::OK, "{g}");
    assert_eq!(g["direction"], "blocked_by");
    assert_eq!(g["transitive"], false);
    assert_eq!(node_ids(&g), sorted(vec![a.clone(), b.clone()]));
    assert_eq!(edge_set(&g), vec![(a.clone(), b.clone())]);

    // Transitive blocked_by on A: A -> B -> C.
    let (_, g) = app
        .get(&app.admin, &format!("/v1/tickets/{a}/deps?transitive=true"))
        .await;
    assert_eq!(node_ids(&g), sorted(vec![a.clone(), b.clone(), c.clone()]));
    assert_eq!(
        edge_set(&g),
        esorted(vec![(a.clone(), b.clone()), (b.clone(), c.clone())])
    );

    // Reverse (blocks) from C, direct: C is blocked_by B, i.e. C blocks B.
    let (_, g) = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{c}/deps?direction=blocks"),
        )
        .await;
    assert_eq!(g["direction"], "blocks");
    assert_eq!(node_ids(&g), sorted(vec![c.clone(), b.clone()]));
    assert_eq!(edge_set(&g), vec![(b.clone(), c.clone())]);

    // Reverse transitive from C reaches A through B.
    let (_, g) = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{c}/deps?direction=blocks&transitive=true"),
        )
        .await;
    assert_eq!(node_ids(&g), sorted(vec![a.clone(), b.clone(), c.clone()]));
    assert_eq!(
        edge_set(&g),
        esorted(vec![(a.clone(), b.clone()), (b.clone(), c.clone())])
    );

    // `both` transitive from the middle node B walks both ways and TERMINATES:
    // A blocks-edge points back to B and B blocked_by-edge back to A, so the
    // visited-set cycle guard is exercised. It must reach all three nodes with
    // exactly the two canonical edges and not loop.
    let (_, g) = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{b}/deps?direction=both&transitive=true"),
        )
        .await;
    assert_eq!(g["direction"], "both");
    assert_eq!(node_ids(&g), sorted(vec![a.clone(), b.clone(), c.clone()]));
    assert_eq!(
        edge_set(&g),
        esorted(vec![(a.clone(), b.clone()), (b.clone(), c.clone())])
    );

    // include=deps carries `blocks` (direct reverse edges) alongside blocked_by.
    let (_, ct) = app
        .get(&app.admin, &format!("/v1/tickets/{c}?include=deps"))
        .await;
    let blocks: Vec<&str> = ct["deps"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert_eq!(blocks, vec![b.as_str()], "C blocks B: {ct}");
    assert!(
        ct["deps"]["blocked_by"].as_array().unwrap().is_empty(),
        "C is blocked by nothing: {ct}"
    );

    // Unknown direction -> 400.
    let (s, body) = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{a}/deps?direction=sideways"),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "validation.direction");
}

#[tokio::test]
async fn archive_hides_from_default_views_and_migration_is_additive() {
    // --- Part 1: behaviour over HTTP -------------------------------------
    let app = TestApp::spawn().await;
    let keeper = app.create_ticket("Stays active").await;
    let archived = app.create_ticket("Will be archived").await;
    // Drive the soon-archived ticket to a claimable (ready) state so the
    // ready-queue exclusion is meaningful.
    app.to_ready(&archived).await;

    let ready_has =
        |v: &Value, id: &str| -> bool { v.as_array().unwrap().iter().any(|t| t["id"] == id) };
    let list_has = |v: &Value, id: &str| -> bool {
        v["items"].as_array().unwrap().iter().any(|t| t["id"] == id)
    };

    // Before archiving: present in ready and counted in metrics.
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    assert!(ready_has(&ready, &archived), "ready should list it first");
    let (_, m0) = app.get(&app.admin, "/v1/metrics").await;
    let total0 = m0["projects"]["tp"]["total"].as_i64().unwrap();

    // Archive it (write scope).
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{archived}/archive"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(body["archived_at"].is_string(), "archived_at set: {body}");

    // Default list excludes it; keeper still shows.
    let (_, list) = app.get(&app.admin, "/v1/tickets?project=tp").await;
    assert!(
        !list_has(&list, &archived),
        "archived hidden from default list"
    );
    assert!(list_has(&list, &keeper), "active ticket still listed");

    // archived=only and include_archived=true surface it.
    let (_, only) = app
        .get(&app.admin, "/v1/tickets?project=tp&archived=only")
        .await;
    assert!(list_has(&only, &archived) && !list_has(&only, &keeper));
    let (_, incl) = app
        .get(&app.admin, "/v1/tickets?project=tp&include_archived=true")
        .await;
    assert!(list_has(&incl, &archived) && list_has(&incl, &keeper));

    // Ready queue and metrics exclude it.
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    assert!(
        !ready_has(&ready, &archived),
        "archived excluded from ready"
    );
    let (_, m1) = app.get(&app.admin, "/v1/metrics").await;
    let total1 = m1["projects"]["tp"]["total"].as_i64().unwrap();
    assert_eq!(total1, total0 - 1, "metrics drops the archived ticket");

    // The single-ticket GET still returns it (archived is not deleted).
    let (s, one) = app
        .get(&app.admin, &format!("/v1/tickets/{archived}"))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(one["archived_at"].is_string());

    // Unarchive restores it to the default views.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{archived}/unarchive"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(body["archived_at"].is_null(), "archived_at cleared: {body}");
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    assert!(ready_has(&ready, &archived), "unarchived returns to ready");

    // --- Part 2: additive, non-destructive startup migration -------------
    // Build a database with the PRE-migration schema (no archived_at column)
    // and seed it, then open it with the current binary and prove the column
    // is added without disturbing any existing row.
    use rusqlite::params;
    use takomo::store::{ArchivedFilter, Store, TicketListFilter};

    // The exact pre-archived_at DDL for the tables the code touches.
    const OLD_SCHEMA: &str = r#"
    CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, workflow_json TEXT NOT NULL, created_at INTEGER NOT NULL);
    CREATE TABLE workflow_states (project TEXT NOT NULL, state TEXT NOT NULL, category TEXT NOT NULL, claimable INTEGER NOT NULL DEFAULT 0, terminal INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (project, state));
    CREATE TABLE tickets (
      id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id), type TEXT NOT NULL DEFAULT 'task',
      parent TEXT REFERENCES tickets(id), title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', state TEXT NOT NULL,
      priority TEXT NOT NULL DEFAULT 'normal', labels TEXT NOT NULL DEFAULT '[]', metadata TEXT NOT NULL DEFAULT '{}',
      links TEXT NOT NULL DEFAULT '{}', claim_holder TEXT, claim_expires_at INTEGER, fence_seq INTEGER NOT NULL DEFAULT 0,
      version INTEGER NOT NULL DEFAULT 1, created_by TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
    CREATE TABLE deps (ticket TEXT NOT NULL REFERENCES tickets(id), blocked_by TEXT NOT NULL REFERENCES tickets(id), PRIMARY KEY (ticket, blocked_by));
    CREATE TABLE comments (id TEXT PRIMARY KEY, ticket TEXT NOT NULL REFERENCES tickets(id), author TEXT NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL);
    CREATE TABLE events (seq INTEGER PRIMARY KEY AUTOINCREMENT, ticket TEXT, project TEXT, actor TEXT NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL DEFAULT '{}', at INTEGER NOT NULL);
    CREATE TABLE tokens (id TEXT PRIMARY KEY, hash TEXT NOT NULL UNIQUE, actor TEXT NOT NULL, scopes TEXT NOT NULL, projects TEXT NOT NULL DEFAULT '*', rate_limit INTEGER NOT NULL DEFAULT 120, created_at INTEGER NOT NULL, expires_at INTEGER, revoked_at INTEGER, last_used_at INTEGER);
    CREATE TABLE idempotency (actor TEXT NOT NULL, key TEXT NOT NULL, ticket TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (actor, key));
    "#;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("old.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(OLD_SCHEMA).unwrap();
        // Confirm the seed DB genuinely lacks the new column.
        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(tickets)").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(
            !cols.iter().any(|c| c == "archived_at"),
            "seed DB should predate archived_at"
        );
        conn.execute(
            "INSERT INTO projects (id,name,workflow_json,created_at) VALUES ('op','Old Project','{}',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workflow_states (project,state,category,claimable,terminal) VALUES ('op','brief','todo',0,0),('op','done','done',0,1)",
            [],
        )
        .unwrap();
        for (id, state) in [("op-aaaa", "brief"), ("op-bbbb", "done")] {
            conn.execute(
                "INSERT INTO tickets (id,project,type,parent,title,body,state,priority,labels,metadata,links,fence_seq,version,created_by,created_at,updated_at) \
                 VALUES (?1,'op','task',NULL,?2,'legacy body',?3,'normal','[\"keep\"]','{\"x.k\":1}','{}',0,3,'seed',1,2)",
                params![id, format!("Legacy {id}"), state],
            )
            .unwrap();
        }
    }

    // Open with the current binary — runs the additive migration.
    let store = Store::open(&db_path).unwrap();

    // Every pre-existing ticket survived, unchanged, with archived_at defaulting
    // to null.
    let a = store
        .get_ticket("op-aaaa")
        .unwrap()
        .expect("legacy a survived");
    assert_eq!(a.title, "Legacy op-aaaa");
    assert_eq!(a.state, "brief");
    assert_eq!(a.body, "legacy body");
    assert_eq!(a.labels, vec!["keep".to_string()]);
    assert_eq!(a.version, 3, "existing version untouched");
    assert!(a.archived_at.is_none());
    let b = store
        .get_ticket("op-bbbb")
        .unwrap()
        .expect("legacy b survived");
    assert_eq!(b.state, "done");
    assert!(b.archived_at.is_none());

    // The new column is functional against the migrated DB.
    store.archive_ticket("op-bbbb", "test:mig").unwrap();
    let active = TicketListFilter {
        project: Some("op".into()),
        ..Default::default()
    };
    let (rows, _) = store.list_tickets(&active, None, 50).unwrap();
    let ids: Vec<&str> = rows.iter().map(|t| t.id.as_str()).collect();
    assert!(
        ids.contains(&"op-aaaa") && !ids.contains(&"op-bbbb"),
        "archived hidden after migration: {ids:?}"
    );
    let only = TicketListFilter {
        project: Some("op".into()),
        archived: ArchivedFilter::Only,
        ..Default::default()
    };
    let (arch_rows, _) = store.list_tickets(&only, None, 50).unwrap();
    assert_eq!(arch_rows.len(), 1);
    assert_eq!(arch_rows[0].id, "op-bbbb");

    // Nothing was dropped: both original rows are still present.
    let all = TicketListFilter {
        project: Some("op".into()),
        archived: ArchivedFilter::Include,
        ..Default::default()
    };
    let (all_rows, _) = store.list_tickets(&all, None, 50).unwrap();
    assert_eq!(all_rows.len(), 2, "no data lost in migration");

    // Migration is idempotent: opening the already-migrated DB again is a no-op
    // and the data is still intact.
    drop(store);
    let store2 = Store::open(&db_path).unwrap();
    assert!(store2.get_ticket("op-aaaa").unwrap().is_some());
    assert!(store2
        .get_ticket("op-bbbb")
        .unwrap()
        .unwrap()
        .archived_at
        .is_some());
}

// --- shareable read-only web views (takomo share) -------------------------------

// A project share lists exactly that project's tickets, and nothing from any
// other project. Also covers self-meta (workflow + scope) and per-ticket detail
// scoping (in-scope 200, out-of-scope 404).
#[tokio::test]
async fn share_project_scopes_to_that_project_only() {
    let app = TestApp::spawn().await;
    let t1 = app.create_ticket("tp ticket one").await;
    let _t2 = app.create_ticket("tp ticket two").await;

    // A second project with its own ticket must never leak into a tp share.
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "tp2", "name": "Second" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, other) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp2", "title": "other" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let other_id = other["id"].as_str().unwrap().to_string();

    let (s, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "create share: {share}");
    let token = share["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("tks_"), "share token has its own prefix");
    assert_eq!(share["kind"], "project");
    assert_eq!(share["ref"], "tp");
    assert_eq!(share["path"], format!("/board#s={token}"));

    // self-meta carries the workflow so the board can render columns.
    let (s, meta) = app.get(&token, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::OK, "self: {meta}");
    assert_eq!(meta["project"], "tp");
    assert_eq!(meta["kind"], "project");
    assert!(
        meta["workflow"]["states"].is_array(),
        "workflow present: {meta}"
    );

    let (s, list) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "exactly tp's two tickets: {list}");
    assert!(ids.contains(&t1.as_str()));
    assert!(
        !ids.contains(&other_id.as_str()),
        "other project must not leak into the share"
    );

    // per-ticket detail: in scope 200, out of scope 404.
    let (s, detail) = app
        .get(&token, &format!("/v1/shares/self/tickets/{t1}"))
        .await;
    assert_eq!(s, StatusCode::OK, "in-scope detail: {detail}");
    assert!(detail["comments"].is_array());
    assert!(detail["deps"].is_object());
    let (s, _d) = app
        .get(&token, &format!("/v1/shares/self/tickets/{other_id}"))
        .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "out-of-scope ticket is invisible to the share"
    );
}

// An epic (subtree) share lists exactly the root plus its full recursive
// descendant subtree — not siblings, not the rest of the project.
#[tokio::test]
async fn share_epic_scopes_to_subtree_only() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Epic root", "epic", None).await;
    let c1 = app.create_typed("child one", "task", Some(&epic)).await;
    let c2 = app.create_typed("child two", "task", Some(&epic)).await;
    let g = app.create_typed("grandchild", "task", Some(&c1)).await;
    let sibling = app.create_typed("unrelated sibling", "task", None).await;

    let (s, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "epic", "ref": epic }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{share}");
    let token = share["token"].as_str().unwrap().to_string();
    // 'epic' is the caller-facing spelling; the stored/echoed kind is 'subtree'.
    assert_eq!(share["kind"], "subtree");
    assert_eq!(share["ref"], epic);

    let (s, list) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<String> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&epic), "root included");
    assert!(ids.contains(&c1), "direct child included");
    assert!(ids.contains(&c2), "direct child included");
    assert!(ids.contains(&g), "recursive descendant included");
    assert!(!ids.contains(&sibling), "sibling excluded from subtree");
    assert_eq!(ids.len(), 4, "exactly the subtree: {list}");

    // The sibling is out of scope for the per-ticket detail too.
    let (s, _d) = app
        .get(&token, &format!("/v1/shares/self/tickets/{sibling}"))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// A share token is read-only and reaches ONLY the share endpoints: it is
// rejected on every normal endpoint (read and write). A normal token is likewise
// not accepted as a share token.
#[tokio::test]
async fn share_token_rejected_on_normal_endpoints() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;
    let (_, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    let token = share["token"].as_str().unwrap().to_string();

    // normal read endpoint
    let (s, _) = app.get(&token, "/v1/tickets?project=tp").await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "share token must not read arbitrary endpoints"
    );
    // normal write endpoint
    let (s, _) = app
        .post(
            &token,
            "/v1/tickets",
            json!({ "project": "tp", "title": "x" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "share token must not write");
    // whoami (any-valid-token endpoint) still rejects a share token
    let (s, _) = app.get(&token, "/v1/whoami").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // A normal token is not a share token on the share path.
    let (s, _) = app.get(&app.admin, "/v1/shares/self").await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "normal token is not a share token"
    );
}

// Revocation is immediate: the share token then returns 410 Gone. List never
// discloses the plaintext token or its hash.
#[tokio::test]
async fn share_revocation_returns_410() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;
    let (_, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    let token = share["token"].as_str().unwrap().to_string();
    let id = share["id"].as_str().unwrap().to_string();

    let (s, _) = app.get(&token, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::OK, "share works before revoke");

    // list (admin) shows metadata but never the secret.
    let (s, ls) = app.get(&app.admin, "/v1/shares").await;
    assert_eq!(s, StatusCode::OK);
    let rows = ls.as_array().unwrap();
    assert!(rows.iter().any(|x| x["id"] == id));
    assert!(
        rows.iter()
            .all(|x| x.get("token").is_none() && x.get("token_hash").is_none()),
        "list must not disclose the token or its hash: {ls}"
    );

    let (s, _) = app.delete(&app.admin, &format!("/v1/shares/{id}")).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (s, body) = app.get(&token, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::GONE, "revoked share is gone: {body}");
    assert_eq!(body["code"], "share.expired");
    let (s, _) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::GONE);
}

// An expired share returns 410 Gone; a still-valid one works. Uses a backdated
// mint (a second DB connection) so the test does not sleep on wall-clock time.
#[tokio::test]
async fn share_expiry_returns_410() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;
    let store = app.open_store();

    // expires_at in the far past -> already expired.
    let (_, expired) = store
        .create_share(ShareKind::Project, "tp", "tp", 1, "test:setup")
        .unwrap();
    let (s, body) = app.get(&expired, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::GONE, "expired share is gone: {body}");
    assert_eq!(body["code"], "share.expired");

    // a future expiry still works.
    let future = takomo::ids::now_ms() + 60_000;
    let (_, fresh) = store
        .create_share(ShareKind::Project, "tp", "tp", future, "test:setup")
        .unwrap();
    let (s, _) = app.get(&fresh, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::OK, "unexpired share works");
}

// Archived tickets are excluded from a share by default and included on request.
#[tokio::test]
async fn share_excludes_archived_by_default() {
    let app = TestApp::spawn().await;
    let keep = app.create_ticket("active ticket").await;
    let gone = app.create_ticket("to be archived").await;
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{gone}/archive"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "archive");

    let (_, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    let token = share["token"].as_str().unwrap().to_string();

    let (s, list) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&keep.as_str()));
    assert!(
        !ids.contains(&gone.as_str()),
        "archived excluded by default"
    );

    let (s, list) = app
        .get(&token, "/v1/shares/self/tickets?include_archived=true")
        .await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&gone.as_str()), "archived included on request");
}

// Share creation validates the referent and enforces write scope + project scope.
#[tokio::test]
async fn share_creation_validates_ref_and_authority() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;

    // unknown project / ticket -> 404.
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "nope" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "epic", "ref": "tp-zzzz" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // bad kind / over-cap ttl -> 422.
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "bogus", "ref": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp", "ttl_seconds": 99_999_999 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    // a read-only token cannot mint a share (needs write scope).
    let readonly = app.mint("agent:ro", &["read"], None);
    let (s, _) = app
        .post(
            &readonly,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "share mint needs write scope");

    // a token scoped to a different project cannot share tp.
    let scoped = app.mint("agent:elsewhere", &["read", "write"], Some(&["other"]));
    let (s, _) = app
        .post(
            &scoped,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "cannot share a project outside token scope"
    );
}

// ---------------------------------------------------------------------------
// Ask-a-human board

#[tokio::test]
async fn question_ask_parks_ticket_and_answer_resumes_it() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Delete the legacy billing table?").await;
    let fence = app.to_implementing(&id).await;

    // Agent asks a confirm question, echoing its lease fence.
    let (qid, body) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "confirm",
                "title": "OK to drop table billing_v1?",
                "body": "It has no reads in 90d but I want a human to confirm.",
                "expertise": ["domain:billing"],
                "urgency": "high",
                "fence": fence,
            }),
        )
        .await;
    // Ticket is parked in the blocked state and the lease was released.
    assert_eq!(body["ticket"]["state"], "needs-decision");
    assert!(
        body["ticket"]["claim"].is_null(),
        "lease should be released"
    );

    // The inbox shows it as open, routable by expertise.
    let (s, list) = app
        .get(&app.human, "/v1/questions?project=tp&status=open")
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["items"][0]["id"], qid);

    // A token without the human scope cannot answer.
    let (s, denied) = app.answer(&app.worker, &qid, json!("yes")).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "worker answered without human scope: {denied}"
    );
    assert_eq!(denied["code"], "auth.scope");

    // The human answers yes; the ticket resumes into the claimable ready state.
    let (s, answered) = app
        .answer(
            &app.human,
            &qid,
            json!({ "value": "yes", "note": "confirmed with data team" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "answer failed: {answered}");
    assert_eq!(answered["question"]["status"], "answered");
    assert_eq!(answered["question"]["answer"]["value"], true);
    assert_eq!(answered["question"]["resolved_to"], "ready");
    assert_eq!(answered["ticket"]["state"], "ready");

    // The exchange is recorded as a comment the resuming agent can read.
    let (s, detail) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let comments = detail["comments"].as_array().unwrap();
    assert!(
        comments.iter().any(|c| c["author"] == "human:reviewer"
            && c["body"].as_str().unwrap().contains("Human answered")),
        "answer should leave a comment: {detail}"
    );

    // Answering again is rejected — the question is closed.
    let (s, again) = app.answer(&app.human, &qid, json!("no")).await;
    assert_eq!(s, StatusCode::CONFLICT, "{again}");
    assert_eq!(again["code"], "question.not_open");
}

#[tokio::test]
async fn question_followup_loop_bounces_to_agent_and_back_before_answering() {
    let app = TestApp::spawn().await;
    let id = app
        .create_ticket("Prod schema migration for billing_rollup")
        .await;
    let fence = app.to_implementing(&id).await;

    // Agent asks a blocking approve question; the ticket parks.
    let (qid, body) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "confirm",
                "title": "Run the additive migration on prod now?",
                "expertise": ["domain:billing"],
                "urgency": "high",
                "fence": fence,
            }),
        )
        .await;
    assert_eq!(
        body["question"]["awaiting"], "human",
        "fresh question awaits a human"
    );
    assert_eq!(body["ticket"]["state"], "needs-decision");

    // An agent can't reply out of turn: nothing has been bounced back yet.
    let (s, oot) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/reply"),
            json!({ "message": "unsolicited" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "out-of-turn reply should be refused: {oot}"
    );
    assert_eq!(oot["code"], "question.not_awaiting_reply");

    // Human bounces it back for more research instead of answering.
    let (s, fu) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/followup"),
            json!({ "message": "What's the row count and lock time on prod?" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "followup failed: {fu}");
    assert_eq!(
        fu["status"], "open",
        "question stays open during a follow-up"
    );
    assert_eq!(fu["awaiting"], "agent", "now the agent owes a reply");

    // The ticket stays parked — the human still owns the eventual answer.
    let (_, tk) = app.get(&app.worker, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        tk["state"], "needs-decision",
        "ticket stays parked mid-thread"
    );

    // Thread now carries the human's request.
    let (_, detail) = app.get(&app.worker, &format!("/v1/questions/{qid}")).await;
    let thread = detail["thread"].as_array().unwrap();
    assert_eq!(thread.len(), 1);
    assert_eq!(thread[0]["role"], "human");

    // The asking agent replies with the research; the thread returns to the human.
    let (s, rep) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/reply"),
            json!({ "message": "40k rows, ~2s lock, fully reversible." }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "reply failed: {rep}");
    assert_eq!(rep["awaiting"], "human", "back to the human to decide");

    let (_, detail2) = app.get(&app.human, &format!("/v1/questions/{qid}")).await;
    assert_eq!(detail2["thread"].as_array().unwrap().len(), 2);

    // Now the human answers; the ticket resumes.
    let (s, answered) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "answer failed: {answered}");
    assert_eq!(answered["question"]["status"], "answered");
    assert_eq!(answered["ticket"]["state"], "ready");

    // A follow-up on a closed question is refused.
    let (s, closed) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/followup"),
            json!({ "message": "too late?" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{closed}");
    assert_eq!(closed["code"], "question.not_open");
}

/// Revising a still-open choose question's options: the point is that an agent
/// which learns something mid-thread can fix the choices instead of withdrawing
/// the question and losing the thread. The revision must leave the question
/// coherent (no recommendation pointing at a removed option), must not touch
/// whose turn it is, and must be refused once the question is settled.
#[tokio::test]
async fn question_options_can_be_revised_while_open() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Pick a cache eviction policy").await;
    let fence = app.to_implementing(&id).await;

    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Which eviction policy?",
                "options": ["LRU", "LFU", "FIFO"],
                "recommended": "LFU",
                "fence": fence,
            }),
        )
        .await;

    // Reword/extend while keeping the recommended option: recommendation stands,
    // and whose turn it is must not move.
    let (s, rev) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({
                "options": ["LRU", "LFU", "ARC"],
                "reason": "FIFO thrashes on our access pattern; ARC is the real contender",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "revise failed: {rev}");
    assert_eq!(rev["options"], json!(["LRU", "LFU", "ARC"]));
    assert_eq!(rev["recommended"], "LFU", "still a valid option, so kept");
    assert_eq!(rev["awaiting"], "human", "revising is not a turn change");
    assert_eq!(rev["status"], "open");

    // Dropping the recommended option without naming a new one is refused —
    // a dangling recommendation must not be silently dropped.
    let (s, dangling) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU", "ARC"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{dangling}");
    assert_eq!(dangling["code"], "validation.recommended");
    assert!(
        dangling["message"].as_str().unwrap().contains("LFU"),
        "the error must name the stale recommendation: {dangling}"
    );

    // Same revision, now naming the new recommendation.
    let (s, rev2) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU", "ARC"], "recommended": "ARC", "recommended_note": "adaptive" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{rev2}");
    assert_eq!(rev2["recommended"], "ARC");
    assert_eq!(rev2["recommended_note"], "adaptive");

    // Explicit null clears the recommendation.
    let (s, cleared) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU", "ARC"], "recommended": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{cleared}");
    assert!(cleared["recommended"].is_null(), "{cleared}");

    // Below two options it is no longer a choice.
    let (s, one) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{one}");
    assert_eq!(one["code"], "validation.options");

    // The audit trail: a ticket comment naming the change, and an event.
    let (s, t) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let comments = t["comments"].as_array().unwrap();
    assert!(
        comments.iter().any(|c| {
            let b = c["body"].as_str().unwrap_or("");
            // The comment carries the before/after sets and the stated reason, so
            // a human who read the old options can see exactly what moved.
            b.contains("revised the options")
                && b.contains("LRU, LFU, FIFO")
                && b.contains("FIFO thrashes on our access pattern")
        }),
        "expected a revision comment naming the change: {comments:?}"
    );

    // A human holding the ORIGINAL option list cannot land a stale pick.
    let (s, stale) = app.answer(&app.human, &qid, json!("FIFO")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{stale}");
    assert_eq!(stale["code"], "validation.answer");

    // Answer it for real, then revising is refused: the choices a decision was
    // made on must stay on the record.
    let (s, answered) = app.answer(&app.human, &qid, json!("ARC")).await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    let (s, settled) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU", "LFU"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{settled}");
    assert_eq!(settled["code"], "question.not_open");
}

/// Options only exist on a `choose` question, so revising any other kind is a
/// validation error rather than a silent no-op.
#[tokio::test]
async fn question_options_revision_rejects_non_choose_kinds() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Drop the legacy table?").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Drop it?", "fence": fence }),
        )
        .await;
    let (s, err) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["yes", "no"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.kind");
    assert!(
        err["message"].as_str().unwrap().contains("confirm"),
        "{err}"
    );
}

#[tokio::test]
async fn question_multi_select_choose_round_trip_and_answer() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Enable regions").await;
    let fence = app.to_implementing(&id).await;

    let (qid, body) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "multi": true,
                "title": "Which regions should launch?",
                "options": ["US", "EU", "UK", "APAC"],
                "recommended_multi": ["US", "EU"],
                "fence": fence,
            }),
        )
        .await;
    assert_eq!(body["question"]["multi"], true);
    assert_eq!(body["question"]["recommended_multi"], json!(["US", "EU"]));

    // Answer with a subset array.
    let (s, ans) = app.answer(&app.human, &qid, json!(["US", "APAC"])).await;
    assert_eq!(s, StatusCode::OK, "multi answer failed: {ans}");
    assert_eq!(ans["question"]["status"], "answered");
    assert_eq!(ans["question"]["answer"]["value"], json!(["US", "APAC"]));
    assert_eq!(ans["ticket"]["state"], "ready");

    // A non-array answer to a multi question is refused.
    let id2 = app.create_ticket("Regions 2").await;
    let fence2 = app.to_implementing(&id2).await;
    let (_, b2) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id2, "kind": "choose", "multi": true, "title": "?", "options": ["A", "B"], "fence": fence2 }),
        )
        .await;
    let qid2 = b2["question"]["id"].as_str().unwrap().to_string();
    let (s, bad) = app.answer(&app.human, &qid2, json!("A")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");

    // multi on a non-choose kind is refused at ask time.
    let id3 = app.create_ticket("Regions 3").await;
    let fence3 = app.to_implementing(&id3).await;
    let (s, bad3) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id3, "kind": "confirm", "multi": true, "title": "x", "fence": fence3 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad3}");
    assert_eq!(bad3["code"], "validation.multi");
}

#[tokio::test]
async fn question_reopen_takes_back_answer_until_the_ticket_is_in_use() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Prod schema migration").await;
    let fence = app.to_implementing(&id).await;

    // Ask + answer: the ticket resumes into 'ready'.
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Run it?", "fence": fence }),
        )
        .await;
    let (s, ans) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "{ans}");
    assert_eq!(ans["question"]["status"], "answered");
    assert_eq!(ans["ticket"]["state"], "ready");

    // Reopen while the ticket is still free: the question returns to open and the
    // ticket is re-parked in a blocked state.
    let (s, re) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/reopen"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "reopen failed: {re}");
    assert_eq!(re["question"]["status"], "open");
    assert_eq!(re["question"]["answer"], serde_json::Value::Null);
    assert_eq!(re["ticket"]["state"], "needs-decision", "ticket re-parked");

    // A read-only/write token without human scope can't reopen.
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/reopen"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "reopen needs the human scope");

    // Answer again, then claim the resumed ticket → reopen is now refused because
    // a worker relies on the answer.
    let (_, ans2) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(ans2["ticket"]["state"], "ready");
    app.claim(&id).await;
    let (s, blocked) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/reopen"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{blocked}");
    assert_eq!(blocked["code"], "question.reopen_claimed");
}

#[tokio::test]
async fn ticket_promote_records_history_and_project_index() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Ship the billing rollup").await;

    // Promote to staging, then production (free-form targets).
    let (s, p1) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/promote"),
            json!({ "target": "staging", "url": "https://ci/deploy/1", "ref": "v1.2.0" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "promote failed: {p1}");
    assert_eq!(p1["target"], "staging");
    assert_eq!(p1["ref"], "v1.2.0");

    let (s, _p2) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/promote"),
            json!({ "target": "production", "note": "canary then full" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // History is newest-first.
    let (s, hist) = app
        .get(&app.worker, &format!("/v1/tickets/{id}/promotions"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let items = hist["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["target"], "production", "newest first");
    assert_eq!(items[1]["target"], "staging");

    // The project index returns the latest per ticket (production).
    let (s, idx) = app.get(&app.worker, "/v1/promotions?project=tp").await;
    assert_eq!(s, StatusCode::OK);
    let idx_items = idx["items"].as_array().unwrap();
    assert_eq!(idx_items.len(), 1, "one row per ticket");
    assert_eq!(idx_items[0]["ticket"], id);
    assert_eq!(idx_items[0]["target"], "production");

    // include=promotions attaches the history to ticket detail.
    let (_, detail) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=promotions"))
        .await;
    assert_eq!(detail["promotions"].as_array().unwrap().len(), 2);

    // An empty target is rejected.
    let (s, bad) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/promote"),
            json!({ "target": "  " }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.target");
}

#[tokio::test]
async fn question_rich_fields_round_trip_and_quality_hints() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Prod schema migration").await;
    let fence = app.to_implementing(&id).await;

    // A well-formed choose: options with per-option descriptions, a recommended
    // option, a rationale, confidence, and a summary.
    let (_, body) = app
        .ask(
            &app.worker,
            json!({
            "ticket": id,
            "kind": "choose",
            "title": "How should the migration run?",
            "body": "Long enough body to matter for the summary hint. ".repeat(6),
            "options": [
                { "value": "Run it now", "desc": "Additive & reversible; unblocks today." },
                { "value": "Wait for the window", "desc": "Safer timing, but parks the work ~2 days." }
            ],
            "recommended": "Run it now",
            "recommended_note": "additive and reversible",
            "confidence": 3,
            "summary": "Additive migration — run now or wait for the window.",
            "fence": fence,
        }),
        )
        .await;
    let q = &body["question"];
    assert_eq!(q["options"], json!(["Run it now", "Wait for the window"]));
    assert_eq!(
        q["option_notes"],
        json!([
            "Additive & reversible; unblocks today.",
            "Safer timing, but parks the work ~2 days."
        ])
    );
    assert_eq!(q["confidence"], 3);
    assert_eq!(q["recommended_note"], "additive and reversible");
    assert_eq!(
        q["summary"],
        "Additive migration — run now or wait for the window."
    );
    // Fully specified → no quality hints.
    assert_eq!(
        body["hints"].as_array().map(|a| a.len()),
        Some(0),
        "a complete question should produce no hints: {body}"
    );

    // A bare confirm (no recommendation, long body, no summary) → gets hints.
    let id2 = app.create_ticket("Bump toolchain").await;
    let fence2 = app.to_implementing(&id2).await;
    let (_, body2) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id2,
                "kind": "confirm",
                "title": "OK to bump the toolchain?",
                "body": "A fairly long body that should trigger the summary hint. ".repeat(5),
                "fence": fence2,
            }),
        )
        .await;
    let hints = body2["hints"].as_array().unwrap();
    assert!(
        !hints.is_empty(),
        "a bare question should suggest improvements: {body2}"
    );

    // confidence out of range is refused with a teaching error.
    let id3 = app.create_ticket("Another").await;
    let fence3 = app.to_implementing(&id3).await;
    let (s, bad) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id3, "kind": "confirm", "title": "x", "confidence": 5, "fence": fence3 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.confidence");
}

#[tokio::test]
async fn question_choose_validates_options_and_mine_filters_by_expertise() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Which migration strategy?").await;
    let fence = app.to_implementing(&id).await;

    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Pick a migration strategy",
                "options": ["big-bang", "dual-write", "backfill"],
                "expertise": ["domain:data"],
                "fence": fence,
            }),
        )
        .await;

    // An answer outside the offered options is rejected with a teaching error.
    let (s, bad) = app
        .answer(&app.human, &qid, json!("rewrite-everything"))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.answer");

    // Mint an expert token and confirm ?mine=true routes by expert:<tag> scope.
    let (s, tok) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:data", "scopes": ["read", "write", "human", "expert:domain:data"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tok}");
    let expert = tok["token"].as_str().unwrap().to_string();

    let (s, mine) = app.get(&expert, "/v1/questions?project=tp&mine=true").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(mine["items"].as_array().unwrap().len(), 1, "{mine}");
    assert_eq!(mine["items"][0]["id"], qid);

    // A billing expert sees nothing under ?mine=true (different tag).
    let (s, tok2) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:bill", "scopes": ["read", "human", "expert:domain:billing"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tok2}");
    let billing = tok2["token"].as_str().unwrap().to_string();
    let (s, none) = app
        .get(&billing, "/v1/questions?project=tp&mine=true")
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(none["items"].as_array().unwrap().len(), 0, "{none}");

    // The expert answers and resumes the ticket.
    let (s, answered) = app.answer(&expert, &qid, json!("dual-write")).await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["question"]["answer"]["value"], "dual-write");
    assert_eq!(answered["ticket"]["state"], "ready");
}

#[tokio::test]
async fn question_withdraw_closes_it_without_answering() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Never mind, found it").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "clarify", "title": "What does archived mean here?", "fence": fence }),
        )
        .await;

    let (s, w) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/withdraw"),
            json!({ "reason": "figured it out from the docs" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(w["status"], "withdrawn");

    // It leaves the open inbox.
    let (_, list) = app
        .get(&app.human, "/v1/questions?project=tp&status=open")
        .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 0, "{list}");

    // A withdrawn question can no longer be answered.
    let (s, _) = app.answer(&app.human, &qid, json!("some text")).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn answer_link_lets_an_outsider_answer_once() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Outside review").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship it?", "fence": fence }),
        )
        .await;

    // A write-only worker cannot mint a link (delegating needs the human scope).
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A human mints the link.
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("tka_"), "grant token: {token}");
    assert!(link["path"].as_str().unwrap().contains("#a="));

    // The outsider (holding ONLY the grant token) sees the one question...
    let (s, self_view) = app.get(&token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::OK, "{self_view}");
    assert_eq!(self_view["question"]["id"], qid);

    // ...and can answer it, which resumes the ticket.
    let (s, answered) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["ticket"]["state"], "ready");
    assert_eq!(answered["question"]["answered_by"], "human:contractor");

    // The link is single-use: reuse is gone.
    let (s, _) = app.get(&token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::GONE);
    let (s, _) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "no" }))
        .await;
    assert_eq!(s, StatusCode::GONE);

    // A normal (non-grant) token cannot reach the answer endpoints at all.
    let (s, denied) = app.get(&app.worker, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{denied}");
}

#[tokio::test]
async fn answer_link_delegates_approve_only_with_expertise() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Legal sign-off").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "OK legally?", "expertise": ["domain:legal"], "fence": fence }),
        )
        .await;

    // A plain human (no expert:domain:legal) cannot mint a link for an approve
    // question — you can't delegate authority you don't hold.
    let (s, denied) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["code"], "question.approve_expertise");

    // A legal expert mints it; the outsider's link then satisfies the approve
    // gate for this one question.
    let (s, tok) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:counsel", "scopes": ["read", "write", "human", "expert:domain:legal"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tok}");
    let counsel = tok["token"].as_str().unwrap().to_string();
    let (s, link) = app
        .post(
            &counsel,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    let (s, answered) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["ticket"]["state"], "ready");
}

#[tokio::test]
async fn answer_link_revoke_kills_it() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Revoke test").await;
    let fence = app.to_implementing(&id).await;
    let (_, b) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "clarify", "title": "Detail?", "fence": fence }),
        )
        .await;
    let qid = b["question"]["id"].as_str().unwrap().to_string();
    let (_, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    let token = link["token"].as_str().unwrap().to_string();
    let gid = link["id"].as_str().unwrap().to_string();

    // Revoke it, then the token is gone.
    let (s, _) = app
        .delete(&app.human, &format!("/v1/answer-links/{gid}"))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app.get(&token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::GONE);
}

#[tokio::test]
async fn question_recommended_timeout_requires_a_real_window() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("No instant self-approve").await;
    let fence = app.to_implementing(&id).await;

    // on_timeout=recommended with a 1s window is refused: it would let a
    // write-only agent satisfy the human gate almost instantly.
    let (s, body) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": id, "kind": "confirm", "title": "Proceed?",
                "recommended": "yes", "expires_in_seconds": 1, "on_timeout": "recommended",
                "fence": fence,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "validation.on_timeout");
}

#[tokio::test]
async fn question_expiry_applies_recommendation() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Auto-resolve on timeout").await;
    let fence = app.to_implementing(&id).await;

    // A valid (>= minimum) recommended-timeout window.
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "confirm",
                "title": "Proceed if nobody objects?",
                "recommended": "yes",
                "expires_in_seconds": 3600,
                "on_timeout": "recommended",
                "fence": fence,
            }),
        )
        .await;

    // Backdate the deadline directly in the DB (as an aged question would be),
    // so the sweeper picks it up without waiting an hour.
    {
        let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let past = takomo::ids::now_ms() - 1000;
        let n = conn
            .execute(
                "UPDATE questions SET expires_at = ?2 WHERE id = ?1",
                rusqlite::params![qid, past],
            )
            .expect("backdate");
        assert_eq!(n, 1);
    }

    // The sweeper runs every 250ms in tests.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, q) = app.get(&app.admin, &format!("/v1/questions/{qid}")).await;
        if q["status"] == "answered" {
            assert_eq!(q["answered_by"], "system");
            assert_eq!(q["answer"]["value"], true);
            let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
            assert_eq!(t["state"], "ready", "ticket should resume on timeout");
            break;
        }
        assert!(Instant::now() < deadline, "question was not swept: {q}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

#[tokio::test]
async fn project_question_language_surfaces_to_agents() {
    let app = TestApp::spawn().await;

    // Admin sets the project's human-facing question language.
    let (s, body) = app
        .put(
            &app.admin,
            "/v1/projects/tp/language",
            json!({ "language": "German" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["question_language"], "German");

    // It shows up in the project list…
    let tp = app.project(&app.worker, "tp").await;
    assert_eq!(tp["question_language"], "German");

    // …and the ask response nudges the agent toward that language.
    let id = app.create_ticket("Sprachtest").await;
    let fence = app.to_implementing(&id).await;
    let (_, asked) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Weitermachen?", "fence": fence }),
        )
        .await;
    assert!(
        asked["note"].as_str().unwrap().contains("German"),
        "ask note should nudge the language: {asked}"
    );

    // Clearing it removes the nudge.
    let (s, cleared) = app
        .put(
            &app.admin,
            "/v1/projects/tp/language",
            json!({ "language": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(cleared["question_language"].is_null());

    // Non-admins can't set it.
    let (s, _) = app
        .put(
            &app.worker,
            "/v1/projects/tp/language",
            json!({ "language": "French" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn project_style_guide_surfaces_to_agents() {
    let app = TestApp::spawn().await;
    let guide = "Two sentences max. Plain language, no marketing voice.";

    // Admin sets the project's house style for agent-written text.
    let (s, body) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": guide }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["style_guide"], guide);

    // It shows up in the project list…
    let tp = app.project(&app.worker, "tp").await;
    assert_eq!(tp["style_guide"], guide);

    // …and the ask response echoes it back, so an agent that asked before
    // reading the conventions can still fix the question it just wrote.
    let id = app.create_ticket("Style check").await;
    let fence = app.to_implementing(&id).await;
    let (_, asked) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Proceed?", "fence": fence }),
        )
        .await;
    assert!(
        asked["note"].as_str().unwrap().contains(guide),
        "ask note should carry the style guide: {asked}"
    );

    // A guide over the cap is refused with a teaching 422 — and the previous
    // guide survives, so a bad update never silently wipes the setting.
    let too_long = "x".repeat(2001);
    let (s, err) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": too_long }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "project.style_guide_too_long");
    assert_eq!(err["details"]["max_chars"], 2000);
    assert_eq!(
        app.project(&app.worker, "tp").await["style_guide"],
        guide,
        "a rejected update must not clear the existing guide"
    );

    // A blank string is a clear, so callers never have to distinguish "" from null.
    let (s, cleared) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": "   " }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(cleared["style_guide"].is_null(), "{cleared}");

    // Omitting the field is an error, not a silent clear.
    let (s, _) = app
        .put(&app.admin, "/v1/projects/tp/style", json!({}))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Non-admins can't set it.
    let (s, _) = app
        .put(
            &app.worker,
            "/v1/projects/tp/style",
            json!({ "style_guide": "anything" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn project_create_rejects_oversized_style_guide_before_creating() {
    let app = TestApp::spawn().await;
    let (s, err) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "styl", "name": "Style", "style_guide": "x".repeat(2001) }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "project.style_guide_too_long");

    // The project must not exist: the guide is validated before the insert, so a
    // rejected create leaves nothing half-configured behind.
    let (_, list) = app.get(&app.admin, "/v1/projects").await;
    assert!(
        !list.as_array().unwrap().iter().any(|p| p["id"] == "styl"),
        "rejected create must not leave a project behind: {list}"
    );

    // A valid one sets the guide at creation time.
    let (s, made) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "styl", "name": "Style", "style_guide": "Terse. Imperative mood." }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["style_guide"], "Terse. Imperative mood.");
}

#[tokio::test]
async fn question_barrier_resumes_only_when_all_answered() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Two decisions").await;
    let fence = app.to_implementing(&id).await;

    // Two distinct questions on the same parked ticket.
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "OK to drop the table?", "fence": fence }),
        )
        .await;
    // Second ask: ticket is already parked + unclaimed, so no fence needed.
    let (q2, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "choose", "title": "Which migration?", "options": ["a", "b"] }),
        )
        .await;
    assert_ne!(q1, q2);

    // Answering the first does NOT resume — the barrier is not cleared.
    let (s, a1) = app.answer(&app.human, &q1, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "{a1}");
    assert!(
        a1["question"]["resolved_to"].is_null(),
        "first answer must not resume: {a1}"
    );
    assert_eq!(a1["ticket"]["state"], "needs-decision");

    // Answering the last one resumes the ticket.
    let (s, a2) = app.answer(&app.human, &q2, json!("a")).await;
    assert_eq!(s, StatusCode::OK, "{a2}");
    assert_eq!(a2["question"]["resolved_to"], "ready");
    assert_eq!(a2["ticket"]["state"], "ready");
}

#[tokio::test]
async fn question_advisory_on_epic_does_not_park() {
    let app = TestApp::spawn().await;
    // An epic sits in `brief` — which has no self-service park edge, so a
    // blocking question would fail. Advisory works and changes no state.
    let epic = app
        .create_typed("Ship the billing revamp", "epic", None)
        .await;

    let (s, blocked) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": epic, "kind": "confirm", "title": "Do this epic at all?" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "blocking on a brief epic can't park: {blocked}"
    );
    assert_eq!(blocked["code"], "question.no_park");

    let (_, body) = app
        .ask(
            &app.worker,
            json!({ "ticket": epic, "mode": "advisory", "kind": "choose",
                "title": "Which direction for the epic?", "options": ["rewrite", "incremental"],
                "expertise": ["domain:product"] }),
        )
        .await;
    assert_eq!(body["question"]["mode"], "advisory");
    // The epic did not move and holds no claim.
    assert_eq!(body["ticket"]["state"], "brief");
    let qid = body["question"]["id"].as_str().unwrap().to_string();

    // Answering records the decision but changes no ticket state.
    let (s, ans) = app.answer(&app.human, &qid, json!("incremental")).await;
    assert_eq!(s, StatusCode::OK, "{ans}");
    assert_eq!(ans["question"]["status"], "answered");
    assert!(ans["question"]["resolved_to"].is_null());
    assert_eq!(
        ans["ticket"]["state"], "brief",
        "advisory must not move the ticket"
    );
}

#[tokio::test]
async fn question_advisory_does_not_gate_the_barrier() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Blocking + advisory").await;
    let fence = app.to_implementing(&id).await;

    // A blocking question parks the ticket.
    let (blocking, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "OK to proceed?", "fence": fence }),
        )
        .await;
    // An advisory question on the same (now parked, unclaimed) ticket.
    let (advisory, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "mode": "advisory", "kind": "clarify", "title": "FYI: any concerns?" }),
        )
        .await;

    // Answering the advisory one does NOT resume — and, being advisory, never would.
    let (s, _) = app.answer(&app.human, &advisory, json!("none")).await;
    assert_eq!(s, StatusCode::OK);
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        t["state"], "needs-decision",
        "advisory answer must not resume"
    );

    // Answering the blocking one resumes, even though... the advisory was the
    // only other open question and advisory never counts toward the barrier.
    let (s, done) = app.answer(&app.human, &blocking, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["ticket"]["state"], "ready");
}

#[tokio::test]
async fn question_ask_is_idempotent_on_retry() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Retry safe").await;
    let fence = app.to_implementing(&id).await;
    let ask = json!({ "ticket": id, "kind": "confirm", "title": "Same question?", "fence": fence });
    let (s, first) = app.post(&app.worker, "/v1/questions", ask.clone()).await;
    assert_eq!(s, StatusCode::CREATED, "{first}");
    // A retry with identical (asker, kind, title) returns the same question,
    // not a duplicate.
    let (_, again) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Same question?" }),
        )
        .await;
    assert_eq!(first["question"]["id"], again["question"]["id"]);
    let (_, list) = app
        .get(&app.human, "/v1/questions?project=tp&status=open")
        .await;
    assert_eq!(
        list["items"].as_array().unwrap().len(),
        1,
        "no duplicate: {list}"
    );
}

#[tokio::test]
async fn question_approve_requires_a_matching_domain_expert() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Approve gate").await;
    let fence = app.to_implementing(&id).await;

    // approve must name an expertise domain.
    let (s, bad) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "approve", "title": "Sign off?", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.expertise");

    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "Sign off?", "expertise": ["domain:legal"], "fence": fence }),
        )
        .await;

    // A plain human (no matching expert scope) is refused — approve has teeth.
    let (s, denied) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["code"], "question.approve_expertise");

    // The domain expert can.
    let (s, tok) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:lawyer", "scopes": ["read", "write", "human", "expert:domain:legal"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tok}");
    let expert = tok["token"].as_str().unwrap().to_string();
    let (s, ok) = app.answer(&expert, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "{ok}");
    assert_eq!(ok["ticket"]["state"], "ready");
}

// ---------------------------------------------------------------------------
// Security-review hardening (unknown-field rejection, question pagination,
// security headers, malformed-JSON teaching errors).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_create_rejects_unknown_field() {
    // A typo'd `expires_seconds` must be a loud 400 — not silently ignored,
    // which would mint a non-expiring token.
    let app = TestApp::spawn().await;
    let (status, body) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:x", "scopes": ["read"], "expires_second": 3600 }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "typo'd field must 400: {body}"
    );
    assert_eq!(body["code"], "validation.unknown_field");
}

#[tokio::test]
async fn transition_rejects_unknown_field_so_fence_typo_cannot_slip_through() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("fence typo").await;
    let (status, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "todo", "fenc": 7 }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "typo'd fence must 400: {body}"
    );
    assert_eq!(body["code"], "validation.unknown_field");
}

#[tokio::test]
async fn questions_list_is_paginated_with_limit_and_cursor() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("q host").await;
    for i in 0..3 {
        app.ask(
            &app.admin,
            json!({
                "ticket": id, "kind": "clarify", "mode": "advisory",
                "title": format!("q{i}"),
            }),
        )
        .await;
    }
    // First page of one, with a cursor for more.
    let (st, page1) = app
        .get(&app.admin, "/v1/questions?status=open&limit=1")
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        page1["items"].as_array().unwrap().len(),
        1,
        "page size honored"
    );
    let cursor = page1["next_cursor"].as_i64().expect("next_cursor present");
    // Following the cursor returns the rest.
    let (_, page2) = app
        .get(
            &app.admin,
            &format!("/v1/questions?status=open&limit=1&cursor={cursor}"),
        )
        .await;
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert_ne!(
        page1["items"][0]["id"], page2["items"][0]["id"],
        "distinct pages"
    );
}

#[tokio::test]
async fn html_apps_send_security_headers() {
    let app = TestApp::spawn().await;
    for path in ["/board", "/inbox"] {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        let h = resp.headers();
        assert!(
            h.get("content-security-policy")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("default-src 'self'"))
                .unwrap_or(false),
            "{path} must send a CSP"
        );
        assert_eq!(
            h.get("x-frame-options").and_then(|v| v.to_str().ok()),
            Some("DENY"),
            "{path}"
        );
        assert_eq!(
            h.get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn malformed_json_body_returns_a_teaching_error() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("bad body").await;
    let resp = app
        .authed(
            Method::POST,
            &app.admin,
            &format!("/v1/tickets/{id}/comments"),
        )
        .header("content-type", "application/json")
        .body("{ this is not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = resp.json::<Value>().await.unwrap_or(Value::Null);
    assert_eq!(
        body["code"], "validation.json",
        "structured teaching error, got {body}"
    );
}

#[tokio::test]
async fn timeout_recommendation_holds_ticket_while_another_blocking_question_is_open() {
    // The HIGH barrier fix: a recommended-timeout must NOT resume a parked ticket
    // while another open blocking question remains on it.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("two blockers").await;
    let fence = app.to_implementing(&id).await;

    // Q1: recommended-on-timeout.
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id, "kind": "confirm", "title": "Q1 auto?",
                "recommended": "yes", "expires_in_seconds": 3600,
                "on_timeout": "recommended", "fence": fence,
            }),
        )
        .await;

    // Q2: a second blocking question on the same (now parked) ticket.
    let (s2, b2) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "clarify", "title": "Q2 needs a human" }),
        )
        .await;
    assert_eq!(
        s2,
        StatusCode::CREATED,
        "second blocking ask should be allowed: {b2}"
    );

    // Backdate Q1's deadline so the sweeper fires it.
    {
        let conn = rusqlite::Connection::open(app.db_path()).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        conn.execute(
            "UPDATE questions SET expires_at = ?2 WHERE id = ?1",
            rusqlite::params![q1, takomo::ids::now_ms() - 1000],
        )
        .unwrap();
    }

    // Wait for Q1 to be swept to answered.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, q) = app.get(&app.admin, &format!("/v1/questions/{q1}")).await;
        if q["status"] == "answered" {
            break;
        }
        assert!(Instant::now() < deadline, "Q1 not swept: {q}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // The ticket must still be parked (blocked) because Q2 is open — NOT resumed.
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        t["state_category"], "blocked",
        "ticket must stay parked while Q2 is open, got {t}"
    );
}

// ---------------------------------------------------------------------------
// Round-2 review fixes: grant revocation, advisory non-resume, dep scoping.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn answer_link_is_revoked_when_the_question_is_answered_elsewhere() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("grant revoke").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship?", "fence": fence }),
        )
        .await;
    // Human mints an answer link for an outsider.
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    // An internal human answers the question directly, before the outsider acts.
    let (s, _) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK);
    // The outstanding link is now dead — revoked on resolution, not just single-use.
    let (s, _) = app.get(&token, "/v1/answer/self").await;
    assert_ne!(s, StatusCode::OK, "stale link must not resolve");
    let (s2, _) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "no" }))
        .await;
    assert_ne!(s2, StatusCode::OK, "revoked link must not answer");
}

// ---------------------------------------------------------------------------
// Answer-link single-use is a property of ONE transaction (takomo-o4uw): the
// grant is spent in the same tx that records the answer, not by a follow-up
// write. These two tests pin both halves of that — the race, and the row it
// leaves behind.
// ---------------------------------------------------------------------------

/// Eight holders of ONE answer link answer simultaneously. The grant row is the
/// serialization point, so exactly one wins and the other seven are rejected
/// with a message an outside expert can act on — and, crucially, the question
/// carries exactly one answer and one mirrored comment afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_answers_on_one_link_spend_it_exactly_once() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Outside expert races itself").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Which vendor?",
                "options": ["alpha", "beta"],
                "fence": fence,
            }),
        )
        .await;
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    let gid = link["id"].as_str().unwrap().to_string();

    // Every attempt sends a DIFFERENT answer, so a second write that landed
    // would be visible in the recorded value, not merely in a status code.
    const N: usize = 8;
    let mut futures = Vec::new();
    for i in 0..N {
        let token = token.clone();
        let client = app.client.clone();
        let base = app.base.clone();
        futures.push(tokio::spawn(async move {
            let choice = if i % 2 == 0 { "alpha" } else { "beta" };
            let resp = client
                .post(format!("{base}/v1/answer/self"))
                .bearer_auth(token)
                .json(&json!({ "answer": choice }))
                .send()
                .await
                .expect("answer request");
            let status = resp.status();
            let body = resp.json::<Value>().await.unwrap_or(Value::Null);
            (status, body)
        }));
    }
    let results: Vec<(StatusCode, Value)> = join_all(futures)
        .await
        .into_iter()
        .map(|r| r.expect("join"))
        .collect();

    let winners: Vec<&(StatusCode, Value)> = results
        .iter()
        .filter(|(s, _)| *s == StatusCode::OK)
        .collect();
    assert_eq!(
        winners.len(),
        1,
        "exactly one of {N} concurrent answers may win: {results:?}"
    );

    // EVERY loser is a coherent, teaching 410 — never a 500, never a 409 left
    // over from some other module's bookkeeping, never a silent no-op. A loser
    // is turned away either by the answer-auth path (its link was already spent
    // when the request arrived: `answer.expired`) or, if it got past auth in the
    // window before the winner committed, by the spend inside the answering
    // transaction itself (`answer_link.spent`). Both tell the holder the link
    // was single-use and is gone.
    for (status, body) in results.iter().filter(|(s, _)| *s != StatusCode::OK) {
        assert_eq!(*status, StatusCode::GONE, "loser response: {body}");
        let code = body["code"].as_str().expect("loser code");
        assert!(
            matches!(code, "answer_link.spent" | "answer.expired"),
            "loser needs a stable spent-link code, got: {body}"
        );
        let message = body["message"].as_str().expect("loser message");
        assert!(
            message.contains("single-use"),
            "loser must be told the link was single-use: {message}"
        );
        if code == "answer_link.spent" {
            // The in-transaction rejection also says what became of the question
            // and that nothing of theirs was recorded.
            assert!(
                message.contains("another answer landed first")
                    && message.contains("Nothing was recorded"),
                "in-tx loser must be told their submission did not land: {message}"
            );
            assert!(
                body["remedy"].as_str().is_some_and(|r| !r.is_empty()),
                "in-tx loser needs a remedy: {body}"
            );
        }
    }

    // The state the winner left behind: one answer, one answered event, one
    // mirrored comment. Seven rolled-back transactions wrote nothing.
    let winning_answer = winners[0].1["question"]["answer"]["value"].clone();
    let (s, q) = app.get(&app.human, &format!("/v1/questions/{qid}")).await;
    assert_eq!(s, StatusCode::OK, "{q}");
    assert_eq!(q["status"], "answered");
    assert_eq!(q["answer"]["value"], winning_answer);
    assert_eq!(q["answered_by"], "human:contractor");

    let (_, events) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=question_answered"),
        )
        .await;
    assert_eq!(
        events["events"].as_array().unwrap().len(),
        1,
        "exactly one question_answered event: {events}"
    );
    let (_, ticket) = app
        .get(&app.human, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let mirrored = ticket["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .filter(|c| {
            c["body"]
                .as_str()
                .is_some_and(|b| b.starts_with("Human answered"))
        })
        .count();
    assert_eq!(mirrored, 1, "exactly one mirrored answer comment: {ticket}");

    // The grant is spent, not revoked: the winning transaction marked it used,
    // so the resolution sweep (which only touches still-unused links) left it be.
    let grant = app.open_store().get_answer_grant(&gid).unwrap().unwrap();
    assert!(grant.used_at.is_some(), "the winner must spend the link");
    assert!(
        grant.revoked_at.is_none(),
        "a link spent by its own answer is used, not revoked"
    );
}

/// A link the holder spent themselves must report itself USED, not revoked —
/// the distinction the outside expert reads on the 410. It only holds if the
/// spend happens inside the answering transaction, ahead of the sweep that
/// revokes every still-unused link for a resolved question.
#[tokio::test]
async fn a_link_spent_by_its_own_answer_reports_used_not_revoked() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Spent link reports used").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship it?", "fence": fence }),
        )
        .await;
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    let gid = link["id"].as_str().unwrap().to_string();

    let (s, answered) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");

    let grant = app.open_store().get_answer_grant(&gid).unwrap().unwrap();
    assert!(grant.used_at.is_some(), "used_at must be set by the answer");
    assert!(
        grant.revoked_at.is_none(),
        "the answering link is spent, not revoked: {:?}",
        grant.revoked_at
    );

    // So the holder who reloads their link is told what actually happened.
    for (s, body) in [
        app.get(&token, "/v1/answer/self").await,
        app.post(&token, "/v1/answer/self", json!({ "answer": "no" }))
            .await,
    ] {
        assert_eq!(s, StatusCode::GONE, "{body}");
        assert_eq!(body["code"], "answer.expired", "{body}");
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|m| m.contains("already been used")),
            "a self-spent link must read as used, not revoked: {body}"
        );
    }
}

#[tokio::test]
async fn advisory_question_never_resumes_ticket_on_recommended_timeout() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("advisory no resume").await;
    let fence = app.to_implementing(&id).await;
    // Park the ticket via a blocking question, then withdraw it — the ticket
    // stays blocked with NO open blocking questions.
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "park", "fence": fence }),
        )
        .await;
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(t["state_category"], "blocked", "ticket parked");
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/questions/{q1}/withdraw"),
            json!({ "reason": "n/a" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    // An advisory question with on_timeout=recommended on the still-blocked ticket.
    let (q2, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "mode": "advisory", "title": "adv",
                "recommended": "yes", "expires_in_seconds": 3600, "on_timeout": "recommended" }),
        )
        .await;
    {
        let conn = rusqlite::Connection::open(app.db_path()).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        conn.execute(
            "UPDATE questions SET expires_at = ?2 WHERE id = ?1",
            rusqlite::params![q2, takomo::ids::now_ms() - 1000],
        )
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, q) = app.get(&app.admin, &format!("/v1/questions/{q2}")).await;
        if q["status"] == "answered" {
            break;
        }
        assert!(Instant::now() < deadline, "advisory q not swept: {q}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // Advisory must NEVER touch ticket state — it stays blocked.
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        t["state_category"], "blocked",
        "advisory timeout must not resume, got {t}"
    );
}

#[tokio::test]
async fn cross_project_dep_detail_is_hidden_from_a_scoped_token() {
    let app = TestApp::spawn().await;
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "tp2", "name": "Second" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, b) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp2", "type": "task", "title": "SECRET infra name" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let secret = b["id"].as_str().unwrap().to_string();
    let a = app.create_ticket("A depends on secret").await;
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/deps"),
            json!({ "blocked_by": secret }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    // A token scoped ONLY to tp.
    let (s, tk) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:scoped", "scopes": ["read", "write"], "projects": ["tp"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tk}");
    let scoped = tk["token"].as_str().unwrap().to_string();
    let (s, detail) = app
        .get(&scoped, &format!("/v1/tickets/{a}?include=deps"))
        .await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    let dep = &detail["deps"]["blocked_by"][0];
    assert_eq!(dep["id"], secret);
    assert_eq!(
        dep["out_of_scope"], true,
        "cross-project dep must be redacted: {dep}"
    );
    assert!(dep.get("title").is_none(), "title must not leak: {dep}");
}

#[tokio::test]
async fn human_can_answer_a_single_choose_with_a_custom_free_text_answer() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("custom answer").await;
    let (qid, _) = app
        .ask(
            &app.admin,
            json!({ "ticket": id, "kind": "choose", "mode": "advisory", "title": "Which path?",
                "options": ["big-bang", "canary"] }),
        )
        .await;
    // A free-text answer that isn't one of the options is rejected WITHOUT the flag.
    let (s, _) = app
        .answer(&app.human, &qid, json!("phased rollout instead"))
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a non-option must be rejected normally"
    );
    // With custom:true the human's own instruction is accepted and recorded verbatim.
    let (s, ans) = app
        .answer(
            &app.human,
            &qid,
            json!({ "value": "phased rollout instead", "custom": true }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{ans}");
    let (_, q) = app.get(&app.admin, &format!("/v1/questions/{qid}")).await;
    assert_eq!(q["answer"]["value"], "phased rollout instead");
    assert_eq!(q["answer"]["custom"], true);
}

// ---------------------------------------------------------------------------
// Tag registry + ticket tagging

#[tokio::test]
async fn tag_registry_crud_and_conflict() {
    let app = TestApp::spawn().await;

    // Create a person entity.
    let (s, t) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "ada", "label": "Ada Lovelace", "meta": { "email": "ada@x", "role": "eng" } }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    assert_eq!(t["ref"], "person:ada");
    assert_eq!(t["label"], "Ada Lovelace");
    assert_eq!(t["meta"]["email"], "ada@x");

    // Duplicate (project, kind, handle) is a 409.
    let (s, e) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    assert_eq!(e["code"], "tag.exists");

    // A different kind with the same handle is allowed (identity is kind+handle).
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "component", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // List (ordered by kind, handle) and kind filter.
    let (s, list) = app.get(&app.admin, "/v1/projects/tp/tags").await;
    assert_eq!(s, StatusCode::OK);
    let refs: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["ref"].as_str().unwrap())
        .collect();
    assert_eq!(
        refs,
        vec!["component:ada", "person:ada"],
        "ordered by kind,handle"
    );
    let (_, people) = app
        .get(&app.admin, "/v1/projects/tp/tags?kind=person")
        .await;
    assert_eq!(people["items"].as_array().unwrap().len(), 1);

    // Substring search on handle/label.
    let (_, q) = app.get(&app.admin, "/v1/projects/tp/tags?q=love").await;
    assert_eq!(q["items"].as_array().unwrap().len(), 1);
    assert_eq!(q["items"][0]["ref"], "person:ada");

    // Get one.
    let (s, one) = app.get(&app.admin, "/v1/projects/tp/tags/person/ada").await;
    assert_eq!(s, StatusCode::OK, "{one}");
    assert_eq!(one["label"], "Ada Lovelace");

    // Patch label + merge meta (null deletes a key).
    let (s, patched) = app
        .patch(
            &app.admin,
            "/v1/projects/tp/tags/person/ada",
            json!({ "label": "Ada L.", "meta_merge": { "role": null, "team": "core" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    assert_eq!(patched["label"], "Ada L.");
    assert_eq!(patched["meta"]["email"], "ada@x");
    assert_eq!(patched["meta"]["team"], "core");
    assert!(
        patched["meta"].get("role").is_none(),
        "role deleted via null"
    );

    // Delete; ticket refs (none yet) => still_referenced 0.
    let (s, del) = app
        .delete(&app.admin, "/v1/projects/tp/tags/person/ada")
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(del["still_referenced"], 0);

    // Get after delete is 404.
    let (s, _) = app.get(&app.admin, "/v1/projects/tp/tags/person/ada").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ticket_tags_lazy_create_patch_and_filter() {
    let app = TestApp::spawn().await;

    // Create a ticket tagged with an unregistered handle: it is lazily registered.
    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Invoice rounding", "tags": ["person:ada", "component:billing", "person:ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap().to_string();
    // Deduped, order preserved.
    assert_eq!(t["tags"], json!(["person:ada", "component:billing"]));

    // The lazy-created registry stub exists (label defaults to the handle).
    let (s, stub) = app.get(&app.admin, "/v1/projects/tp/tags/person/ada").await;
    assert_eq!(s, StatusCode::OK, "{stub}");
    assert_eq!(stub["label"], "ada");

    // tags_add / tags_remove are commutative set ops.
    let (s, patched) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "tags_add": ["team:core"], "tags_remove": ["component:billing"] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    assert_eq!(patched["tags"], json!(["person:ada", "team:core"]));

    // Whole-set replace.
    let (_, replaced) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "tags": ["person:ada"] }),
        )
        .await;
    assert_eq!(replaced["tags"], json!(["person:ada"]));

    // A second ticket with a different person.
    let (_, t2) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Payroll", "tags": ["person:grace"] }),
        )
        .await;
    let id2 = t2["id"].as_str().unwrap().to_string();

    // Filter by exact ref.
    let (_, byref) = app
        .get(&app.admin, "/v1/tickets?project=tp&tag=person:ada")
        .await;
    let ids: Vec<&str> = byref["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&id.as_str()) && !ids.contains(&id2.as_str()),
        "tag=person:ada -> {ids:?}"
    );

    // Filter by kind (both tickets carry a person tag).
    let (_, bykind) = app
        .get(&app.admin, "/v1/tickets?project=tp&tag_kind=person")
        .await;
    assert_eq!(
        bykind["items"].as_array().unwrap().len(),
        2,
        "tag_kind=person matches both"
    );

    // Deleting the registry entry reports the tickets still referencing it.
    let (_, del) = app
        .delete(&app.admin, "/v1/projects/tp/tags/person/ada")
        .await;
    assert_eq!(
        del["still_referenced"], 1,
        "one ticket still tags person:ada"
    );
    // The ticket reference survives the registry delete.
    let (_, still) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(still["tags"], json!(["person:ada"]));
}

#[tokio::test]
async fn tag_validation_and_scope() {
    let app = TestApp::spawn().await;

    // Malformed ref on a ticket (no colon) is a teaching 422.
    let (s, e) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "x", "tags": ["ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.tag_ref");

    // Bad kind (uppercase) and bad handle are rejected on registry create.
    let (s, e) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "Person", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.tag_kind");

    let (s, e) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "Ada Lovelace" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.tag_handle");

    // Mint a read-only token: it cannot create a tag or tag a ticket.
    let (_, minted) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:ro", "scopes": ["read"] }),
        )
        .await;
    let ro = minted["token"].as_str().unwrap().to_string();

    let (s, e) = app
        .post(
            &ro,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert_eq!(e["code"], "auth.scope");

    let id = app.create_ticket("taggable").await;
    let (s, e) = app
        .patch(
            &ro,
            &format!("/v1/tickets/{id}"),
            json!({ "tags_add": ["person:ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert_eq!(e["code"], "auth.scope");
}
