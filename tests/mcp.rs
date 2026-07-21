//! Integration tests for the hosted MCP endpoint (`/mcp`).
//!
//! Spawns the real server on an ephemeral port and drives the streamable-HTTP
//! MCP transport over raw JSON-RPC with reqwest (stateless/json-response mode,
//! so each POST is a self-contained request/response). Covers the handshake,
//! tool discovery, a full ticket work loop through the internal store, and the
//! bearer-auth boundary (missing / invalid / share-token requests are rejected).

use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::Duration;
use takomo::server::{build_router, spawn_sweeper, AppState};
use takomo::store::{ShareKind, Store};

const PROTO: &str = "2025-06-18";

struct TestApp {
    base: String,
    /// read,write,human,admin — can drive human-gated transitions.
    human: String,
    /// read,write only.
    worker: String,
    /// A share token (`tks_...`) — must NOT be accepted at /mcp.
    share: String,
    client: reqwest::Client,
    _tmp: tempfile::TempDir,
}

fn scopes(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

async fn spawn() -> TestApp {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = Store::open(tmp.path().join("test.db")).expect("open store");
    store
        .create_project("tp", "Test Project", None, "test:setup")
        .expect("create project");
    let (_, human) = store
        .create_token(
            "human:admin",
            &scopes(&["read", "write", "human", "admin"]),
            None,
            10_000,
            None,
        )
        .unwrap();
    let (_, worker) = store
        .create_token("agent:w1", &scopes(&["read", "write"]), None, 10_000, None)
        .unwrap();
    // A read-only share token, scoped to the project. It lives in the shares
    // table, not tokens, so the normal bearer path must reject it.
    let (_, share) = store
        .create_share(
            ShareKind::Project,
            "tp",
            "tp",
            takomo::ids::now_ms() + 3_600_000,
            "human:admin",
        )
        .unwrap();

    let state = AppState::new(store);
    spawn_sweeper(state.clone(), Duration::from_millis(250));
    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    TestApp {
        base: format!("http://{addr}"),
        human,
        worker,
        share,
        client: reqwest::Client::new(),
        _tmp: tmp,
    }
}

impl TestApp {
    /// Raw JSON-RPC POST to /mcp. `token` None omits the Authorization header.
    async fn rpc(&self, token: Option<&str>, method: &str, params: Value) -> reqwest::Response {
        let mut req = self
            .client
            .post(format!("{}/mcp", self.base))
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
    let app = spawn().await;

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
    let app = spawn().await;
    app.ok_call(&app.human, "initialize", init_params()).await;

    // whoami reflects the bearer identity.
    let who = app.tool_ok(&app.human, "takomo_whoami", json!({})).await;
    assert_eq!(who["whoami"]["actor"].as_str().unwrap(), "human:admin");

    // new — created in the workflow's initial state ("brief").
    let created = app
        .tool_ok(
            &app.human,
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
            &app.human,
            "takomo_transition",
            json!({ "id": id, "to": "spec" }),
        )
        .await;
    assert_eq!(to_spec["ticket"]["state"].as_str().unwrap(), "spec");
    let to_ready = app
        .tool_ok(
            &app.human,
            "takomo_transition",
            json!({ "id": id, "to": "ready" }),
        )
        .await;
    assert_eq!(to_ready["ticket"]["state"].as_str().unwrap(), "ready");

    // ready — the ticket now shows up in the ready queue.
    let ready = app
        .tool_ok(&app.human, "takomo_ready", json!({ "project": "tp" }))
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
        .tool_ok(&app.human, "takomo_claim", json!({ "id": id }))
        .await;
    assert!(
        claim["lease"]["fence"].is_number(),
        "claim returns a fence: {claim}"
    );

    // start — moves ready -> implementing (auto-resolves the held fence).
    let started = app
        .tool_ok(&app.human, "takomo_start", json!({ "id": id }))
        .await;
    assert_eq!(started["ticket"]["state"].as_str().unwrap(), "implementing");

    // implementing -> review (needs the claim; fence resolved automatically).
    let to_review = app
        .tool_ok(
            &app.human,
            "takomo_transition",
            json!({ "id": id, "to": "review" }),
        )
        .await;
    assert_eq!(to_review["ticket"]["state"].as_str().unwrap(), "review");

    // done — review -> done (needs scope:human + no open children).
    let done = app
        .tool_ok(&app.human, "takomo_done", json!({ "id": id }))
        .await;
    assert_eq!(done["transitioned_to"].as_str().unwrap(), "done");
    assert_eq!(done["ticket"]["state"].as_str().unwrap(), "done");
}

#[tokio::test]
async fn hosted_mcp_relays_store_errors_for_self_correction() {
    let app = spawn().await;
    app.ok_call(&app.human, "initialize", init_params()).await;

    let created = app
        .tool_ok(
            &app.human,
            "takomo_new",
            json!({ "project": "tp", "title": "illegal move" }),
        )
        .await;
    let id = created["ticket"]["id"].as_str().unwrap().to_string();

    // brief -> done is not a legal edge; the store's teaching error must come
    // back as a tool-level error with allowed_transitions, not a 500.
    let (payload, is_error) = app
        .tool(
            &app.human,
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
    let app = spawn().await;

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
    let shared = app.rpc(Some(&app.share), "initialize", init_params()).await;
    assert_eq!(
        shared.status(),
        StatusCode::UNAUTHORIZED,
        "share token must be rejected at /mcp"
    );

    // Sanity: the share string really is a share token, and it does authorize
    // the share-scoped read endpoint — proving the /mcp rejection is about the
    // endpoint boundary, not a malformed token.
    let share_ok = app
        .client
        .get(format!("{}/v1/shares/self", app.base))
        .header("Authorization", format!("Bearer {}", app.share))
        .send()
        .await
        .unwrap();
    assert_eq!(
        share_ok.status(),
        StatusCode::OK,
        "share token works on its own endpoint"
    );
}
