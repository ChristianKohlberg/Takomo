//! Integration tests for the hosted MCP endpoint (`/mcp`).
//!
//! Spawns the real server on an ephemeral port and drives the streamable-HTTP
//! MCP transport over raw JSON-RPC with reqwest (stateless/json-response mode,
//! so each POST is a self-contained request/response). Covers the handshake,
//! tool discovery, a full ticket work loop through the internal store, and the
//! bearer-auth boundary (missing / invalid / share-token requests are rejected).

use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use takomo::store::ShareKind;

mod common;
use common::TestApp;

const PROTO: &str = "2025-06-18";

impl TestApp {
    /// Raw JSON-RPC POST to /mcp. `token` None omits the Authorization header.
    async fn rpc(&self, token: Option<&str>, method: &str, params: Value) -> reqwest::Response {
        let mut req = self
            .request(Method::POST, "/mcp")
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTO)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params,
            }));
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        req.send().await.expect("request sent")
    }

    /// A JSON-RPC call that expects a 200 and a `result`.
    async fn ok_call(&self, token: &str, method: &str, params: Value) -> Value {
        let resp = self.rpc(Some(token), method, params).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "method {method} should be 200"
        );
        let body: Value = resp.json().await.expect("json body");
        assert!(
            body.get("error").is_none(),
            "method {method} returned a JSON-RPC error: {body}"
        );
        body["result"].clone()
    }

    /// Invoke an MCP tool and return the parsed tool payload plus its isError flag.
    async fn tool(&self, token: &str, name: &str, arguments: Value) -> (Value, bool) {
        let result = self
            .ok_call(
                token,
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        let is_error = result["isError"].as_bool().unwrap_or(false);
        let text = result["content"][0]["text"]
            .as_str()
            .expect("tool result has text content");
        let payload: Value = serde_json::from_str(text).expect("tool text is JSON");
        (payload, is_error)
    }

    /// Invoke a tool that is expected to succeed.
    async fn tool_ok(&self, token: &str, name: &str, arguments: Value) -> Value {
        let (payload, is_error) = self.tool(token, name, arguments).await;
        assert!(!is_error, "tool {name} unexpectedly errored: {payload}");
        payload
    }
}

fn init_params() -> Value {
    json!({
        "protocolVersion": PROTO,
        "capabilities": {},
        "clientInfo": { "name": "takomo-test", "version": "0" },
    })
}

#[tokio::test]
async fn hosted_mcp_handshake_and_tool_discovery() {
    let app = TestApp::spawn().await;

    let init = app.ok_call(&app.worker, "initialize", init_params()).await;
    assert_eq!(init["protocolVersion"].as_str().unwrap(), PROTO);
    assert!(
        init["capabilities"]["tools"].is_object(),
        "server advertises tools capability: {init}"
    );
    assert!(init["serverInfo"]["name"].is_string());

    let list = app.ok_call(&app.worker, "tools/list", json!({})).await;
    let names: Vec<&str> = list["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "takomo_new",
        "takomo_ready",
        "takomo_claim",
        "takomo_next",
        "takomo_start",
        "takomo_transition",
        "takomo_done",
        "takomo_block",
        "takomo_cancel",
        "takomo_release",
        "takomo_whoami",
    ] {
        assert!(
            names.contains(&expected),
            "tools/list missing {expected}: {names:?}"
        );
    }
}

#[tokio::test]
async fn hosted_mcp_drives_full_work_loop() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;

    // whoami reflects the bearer identity.
    let who = app.tool_ok(&app.admin, "takomo_whoami", json!({})).await;
    assert_eq!(who["whoami"]["actor"].as_str().unwrap(), "human:admin");

    // new — created in the workflow's initial state ("brief").
    let created = app
        .tool_ok(
            &app.admin,
            "takomo_new",
            json!({ "project": "tp", "title": "hosted mcp loop", "type": "task" }),
        )
        .await;
    let id = created["ticket"]["id"]
        .as_str()
        .expect("ticket id")
        .to_string();
    assert_eq!(created["ticket"]["state"].as_str().unwrap(), "brief");

    // Advance brief -> spec -> ready (spec->ready needs scope:human).
    let to_spec = app
        .tool_ok(
            &app.admin,
            "takomo_transition",
            json!({ "id": id, "to": "spec" }),
        )
        .await;
    assert_eq!(to_spec["ticket"]["state"].as_str().unwrap(), "spec");
    let to_ready = app
        .tool_ok(
            &app.admin,
            "takomo_transition",
            json!({ "id": id, "to": "ready" }),
        )
        .await;
    assert_eq!(to_ready["ticket"]["state"].as_str().unwrap(), "ready");

    // ready — the ticket now shows up in the ready queue.
    let ready = app
        .tool_ok(&app.admin, "takomo_ready", json!({ "project": "tp" }))
        .await;
    let ready_ids: Vec<&str> = ready["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ready_ids.contains(&id.as_str()),
        "ready queue should list {id}"
    );

    // claim — takes the lease and returns a fencing token.
    let claim = app
        .tool_ok(&app.admin, "takomo_claim", json!({ "id": id }))
        .await;
    assert!(
        claim["lease"]["fence"].is_number(),
        "claim returns a fence: {claim}"
    );

    // start — moves ready -> implementing (auto-resolves the held fence).
    let started = app
        .tool_ok(&app.admin, "takomo_start", json!({ "id": id }))
        .await;
    assert_eq!(started["ticket"]["state"].as_str().unwrap(), "implementing");

    // implementing -> review (needs the claim; fence resolved automatically).
    let to_review = app
        .tool_ok(
            &app.admin,
            "takomo_transition",
            json!({ "id": id, "to": "review" }),
        )
        .await;
    assert_eq!(to_review["ticket"]["state"].as_str().unwrap(), "review");

    // done — review -> done (needs scope:human + no open children).
    let done = app
        .tool_ok(&app.admin, "takomo_done", json!({ "id": id }))
        .await;
    assert_eq!(done["transitioned_to"].as_str().unwrap(), "done");
    assert_eq!(done["ticket"]["state"].as_str().unwrap(), "done");
}

#[tokio::test]
async fn hosted_mcp_relays_store_errors_for_self_correction() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;

    let created = app
        .tool_ok(
            &app.admin,
            "takomo_new",
            json!({ "project": "tp", "title": "illegal move" }),
        )
        .await;
    let id = created["ticket"]["id"].as_str().unwrap().to_string();

    // brief -> done is not a legal edge; the store's teaching error must come
    // back as a tool-level error with allowed_transitions, not a 500.
    let (payload, is_error) = app
        .tool(
            &app.admin,
            "takomo_transition",
            json!({ "id": id, "to": "done" }),
        )
        .await;
    assert!(
        is_error,
        "illegal transition should be a tool error: {payload}"
    );
    assert_eq!(payload["ok"], json!(false));
    assert!(
        payload["allowed_transitions"].is_array(),
        "error relays allowed_transitions: {payload}"
    );
}

#[tokio::test]
async fn hosted_mcp_rejects_unauthorized_requests() {
    let app = TestApp::spawn().await;

    // No Authorization header.
    let missing = app.rpc(None, "initialize", init_params()).await;
    assert_eq!(
        missing.status(),
        StatusCode::UNAUTHORIZED,
        "missing token must 401"
    );

    // A bogus bearer token.
    let bad = app
        .rpc(Some("tk_not_a_real_token"), "initialize", init_params())
        .await;
    assert_eq!(
        bad.status(),
        StatusCode::UNAUTHORIZED,
        "invalid token must 401"
    );

    // A valid *share* token (tks_...) must not work on /mcp — it lives in the
    // shares table and never resolves against the normal bearer path.
    let (_, share) = app
        .open_store()
        .create_share(
            ShareKind::Project,
            "tp",
            "tp",
            takomo::ids::now_ms() + 3_600_000,
            "human:admin",
        )
        .unwrap();
    let shared = app.rpc(Some(&share), "initialize", init_params()).await;
    assert_eq!(
        shared.status(),
        StatusCode::UNAUTHORIZED,
        "share token must be rejected at /mcp"
    );

    // Sanity: the share string really is a share token, and it does authorize
    // the share-scoped read endpoint — proving the /mcp rejection is about the
    // endpoint boundary, not a malformed token.
    let (share_status, _) = app.get(&share, "/v1/shares/self").await;
    assert_eq!(
        share_status,
        StatusCode::OK,
        "share token works on its own endpoint"
    );
}

#[tokio::test]
async fn hosted_mcp_ask_and_answer_round_trip() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;

    // Create and drive a ticket to implementing (worker holds the lease).
    let created = app
        .tool_ok(
            &app.worker,
            "takomo_new",
            json!({ "project": "tp", "title": "mcp ask", "type": "task" }),
        )
        .await;
    let id = created["ticket"]["id"].as_str().unwrap().to_string();
    // brief -> spec -> ready needs the human scope.
    app.tool_ok(
        &app.admin,
        "takomo_transition",
        json!({ "id": id, "to": "spec" }),
    )
    .await;
    app.tool_ok(
        &app.admin,
        "takomo_transition",
        json!({ "id": id, "to": "ready" }),
    )
    .await;
    app.tool_ok(&app.worker, "takomo_claim", json!({ "id": id }))
        .await;
    app.tool_ok(&app.worker, "takomo_start", json!({ "id": id }))
        .await;

    // The agent asks a human; the ticket parks and the lease releases.
    let asked = app
        .tool_ok(
            &app.worker,
            "takomo_ask",
            json!({
                "id": id,
                "kind": "confirm",
                "title": "Ship it?",
                "expertise": ["domain:release"],
            }),
        )
        .await;
    let qid = asked["question"]["id"].as_str().unwrap().to_string();
    assert_eq!(asked["ticket"]["state"].as_str().unwrap(), "needs-decision");

    // takomo_show surfaces the open question so a resuming agent sees it.
    let shown = app
        .tool_ok(&app.worker, "takomo_show", json!({ "id": id }))
        .await;
    assert_eq!(shown["open_questions"][0]["id"].as_str().unwrap(), qid);

    // A worker without the human scope is refused.
    let (denied, is_err) = app
        .tool(
            &app.worker,
            "takomo_answer",
            json!({ "id": qid, "answer": "yes" }),
        )
        .await;
    assert!(is_err, "worker must not answer: {denied}");

    // The human answers; the ticket resumes into a claimable state.
    let answered = app
        .tool_ok(
            &app.admin,
            "takomo_answer",
            json!({ "id": qid, "answer": "yes" }),
        )
        .await;
    assert_eq!(answered["question"]["status"].as_str().unwrap(), "answered");
    assert_eq!(answered["ticket"]["state"].as_str().unwrap(), "ready");
}

#[tokio::test]
async fn hosted_mcp_surfaces_project_language() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;

    // Admin sets the project's expected human-facing question language.
    let (s, _) = app
        .put(
            &app.admin,
            "/v1/projects/tp/language",
            json!({ "language": "German" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // takomo_workflow carries it.
    let wf = app
        .tool_ok(&app.admin, "takomo_workflow", json!({ "project": "tp" }))
        .await;
    assert_eq!(wf["question_language"], "German");

    // Drive a ticket to implementing; the work-loop responses carry a hint.
    let created = app
        .tool_ok(
            &app.worker,
            "takomo_new",
            json!({ "project": "tp", "title": "sprach", "type": "task" }),
        )
        .await;
    let id = created["ticket"]["id"].as_str().unwrap().to_string();
    app.tool_ok(
        &app.admin,
        "takomo_transition",
        json!({ "id": id, "to": "spec" }),
    )
    .await;
    app.tool_ok(
        &app.admin,
        "takomo_transition",
        json!({ "id": id, "to": "ready" }),
    )
    .await;
    let started = app
        .tool_ok(&app.worker, "takomo_start", json!({ "id": id }))
        .await;
    assert_eq!(started["language_hint"]["question_language"], "German");
    let shown = app
        .tool_ok(&app.worker, "takomo_show", json!({ "id": id }))
        .await;
    assert_eq!(shown["language_hint"]["question_language"], "German");

    // …and the ask response nudges toward it.
    let asked = app
        .tool_ok(
            &app.worker,
            "takomo_ask",
            json!({ "id": id, "kind": "confirm", "title": "Weiter?" }),
        )
        .await;
    assert!(
        asked["note"].as_str().unwrap().contains("German"),
        "ask note: {asked}"
    );
}

#[tokio::test]
async fn hosted_mcp_tag_tool_tags_and_filters() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;

    // Create a ticket with a tag inline (lazy-registers person:ada).
    let created = app
        .tool_ok(
            &app.admin,
            "takomo_new",
            json!({ "project": "tp", "title": "tagged via mcp", "tags": ["person:ada"] }),
        )
        .await;
    let id = created["ticket"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["ticket"]["tags"], json!(["person:ada"]));

    // takomo_tag adds and removes refs.
    let tagged = app
        .tool_ok(
            &app.admin,
            "takomo_tag",
            json!({ "id": id, "add": ["component:billing"], "remove": ["person:ada"] }),
        )
        .await;
    assert_eq!(tagged["tags"], json!(["component:billing"]));

    // takomo_list filters by tag kind.
    let listed = app
        .tool_ok(
            &app.admin,
            "takomo_list",
            json!({ "project": "tp", "tag_kind": "component" }),
        )
        .await;
    let ids: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&id.as_str()),
        "tag_kind=component should list {id}: {ids:?}"
    );
}

#[tokio::test]
async fn hosted_mcp_surfaces_project_style_guide() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;
    let guide = "Two sentences max. Plain language, no marketing voice.";

    // With no style guide set, the work loop stays free of the extra key.
    let bare = app
        .tool_ok(
            &app.worker,
            "takomo_new",
            json!({ "project": "tp", "title": "before", "type": "task" }),
        )
        .await;
    assert!(
        bare.get("style_hint").is_none(),
        "no guide set → no style_hint: {bare}"
    );

    // Admin sets the project's house style for agent-written text.
    let (s, _) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": guide }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // takomo_workflow carries it.
    let wf = app
        .tool_ok(&app.admin, "takomo_workflow", json!({ "project": "tp" }))
        .await;
    assert_eq!(wf["style_guide"], guide);

    // takomo_new echoes it at the moment the ticket text was written.
    let created = app
        .tool_ok(
            &app.worker,
            "takomo_new",
            json!({ "project": "tp", "title": "stil", "type": "task" }),
        )
        .await;
    assert_eq!(created["style_hint"]["style_guide"], guide);
    let id = created["ticket"]["id"].as_str().unwrap().to_string();

    // …as do the claim/start/show work-loop responses.
    app.tool_ok(
        &app.admin,
        "takomo_transition",
        json!({ "id": id, "to": "spec" }),
    )
    .await;
    app.tool_ok(
        &app.admin,
        "takomo_transition",
        json!({ "id": id, "to": "ready" }),
    )
    .await;
    let started = app
        .tool_ok(&app.worker, "takomo_start", json!({ "id": id }))
        .await;
    assert_eq!(started["style_hint"]["style_guide"], guide);
    let shown = app
        .tool_ok(&app.worker, "takomo_show", json!({ "id": id }))
        .await;
    assert_eq!(shown["style_hint"]["style_guide"], guide);

    // …and the ask response carries it too.
    let asked = app
        .tool_ok(
            &app.worker,
            "takomo_ask",
            json!({ "id": id, "kind": "confirm", "title": "Proceed?" }),
        )
        .await;
    assert!(
        asked["note"].as_str().unwrap().contains(guide),
        "ask note: {asked}"
    );
}
// ---- rate limiting: the MCP surface classifies by tool, not HTTP method -----

#[tokio::test]
async fn mcp_reads_do_not_debit_the_write_budget() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("MCP read budget").await;
    // Room for exactly two writes a minute.
    let tight = app.mint_limited("agent:reader", &["read", "write"], None, 2);

    // Every read-only tool, several times over: none of these is a write, even
    // though each one arrives as POST /mcp.
    for _ in 0..2 {
        app.tool_ok(&tight, "takomo_whoami", json!({})).await;
        app.tool_ok(&tight, "takomo_list", json!({ "project": "tp" }))
            .await;
        app.tool_ok(&tight, "takomo_ready", json!({ "project": "tp" }))
            .await;
        app.tool_ok(&tight, "takomo_show", json!({ "id": id }))
            .await;
        app.tool_ok(&tight, "takomo_deps", json!({ "id": id }))
            .await;
        app.tool_ok(&tight, "takomo_questions", json!({ "project": "tp" }))
            .await;
        app.tool_ok(&tight, "takomo_projects", json!({})).await;
        app.tool_ok(&tight, "takomo_workflow", json!({ "project": "tp" }))
            .await;
        app.tool_ok(&tight, "takomo_roadmap", json!({ "project": "tp" }))
            .await;
    }

    // The whole two-write budget is still there.
    app.tool_ok(&tight, "takomo_comment", json!({ "id": id, "body": "one" }))
        .await;
    app.tool_ok(&tight, "takomo_comment", json!({ "id": id, "body": "two" }))
        .await;
}

#[tokio::test]
async fn mcp_writes_still_debit_and_the_429_names_what_was_spent() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("MCP write budget").await;
    let tight = app.mint_limited("agent:chatty", &["read", "write"], None, 1);

    app.tool_ok(&tight, "takomo_comment", json!({ "id": id, "body": "one" }))
        .await;

    let (payload, is_error) = app
        .tool(&tight, "takomo_comment", json!({ "id": id, "body": "two" }))
        .await;
    assert!(is_error, "second write should be refused: {payload}");
    assert_eq!(payload["code"], "rate.limited");
    assert_eq!(payload["status"], 429);
    let message = payload["message"].as_str().expect("message");
    assert!(
        message.contains("write budget of 1 writes/minute"),
        "429 names the budget the caller actually spent: {message}"
    );
    assert!(
        message.contains("reads are free"),
        "429 tells the caller reads still work: {message}"
    );
    assert!(
        payload["remedy"]
            .as_str()
            .is_some_and(|r| r.contains("Wait")),
        "429 carries a remedy: {payload}"
    );

    // …and that is true: the reads it points at still work while writes are
    // refused.
    app.tool_ok(&tight, "takomo_show", json!({ "id": id }))
        .await;
}

#[tokio::test]
async fn mcp_handshake_discovery_and_unknown_tools_are_free() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("MCP handshake budget").await;
    // Room for exactly one write a minute.
    let tight = app.mint_limited("agent:prober", &["read", "write"], None, 1);

    // Attaching to the server must not cost anything: an agent that cannot even
    // discover the tools without spending its write budget is the sharpest form
    // of this bug.
    for _ in 0..3 {
        app.ok_call(&tight, "initialize", init_params()).await;
        app.ok_call(&tight, "tools/list", json!({})).await;
    }

    // A tool that does not exist is not charged either — the router is about to
    // reject it, and billing a write for a call that never ran would make the
    // 429 message a lie.
    let resp = app
        .rpc(
            Some(&tight),
            "tools/call",
            json!({ "name": "takomo_nonexistent", "arguments": {} }),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json body");
    assert!(
        body.get("error").is_some() || body["result"]["isError"] == json!(true),
        "unknown tool should be rejected: {body}"
    );

    // The single write is still available.
    app.tool_ok(&tight, "takomo_comment", json!({ "id": id, "body": "one" }))
        .await;
}

#[tokio::test]
async fn read_tools_are_all_real_advertised_tools() {
    let app = TestApp::spawn().await;
    let list = app.ok_call(&app.worker, "tools/list", json!({})).await;
    let names: Vec<&str> = list["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    // A renamed or removed read tool would silently start debiting the write
    // budget again, so pin the free list against what the server advertises.
    for read in takomo::mcp::READ_TOOLS {
        assert!(
            names.contains(read),
            "READ_TOOLS names '{read}', which is not an advertised tool: {names:?}"
        );
    }
}
