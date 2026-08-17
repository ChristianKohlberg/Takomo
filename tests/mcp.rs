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

/// Park a ticket on an open question over MCP and hand back `(ticket, question)`.
async fn mcp_parked_question(app: &TestApp, title: &str, q: Value) -> (String, String) {
    let created = app
        .tool_ok(
            &app.worker,
            "takomo_new",
            json!({ "project": "tp", "title": title, "type": "task" }),
        )
        .await;
    let id = created["ticket"]["id"].as_str().unwrap().to_string();
    for to in ["spec", "ready"] {
        app.tool_ok(
            &app.admin,
            "takomo_transition",
            json!({ "id": id, "to": to }),
        )
        .await;
    }
    app.tool_ok(&app.worker, "takomo_claim", json!({ "id": id }))
        .await;
    app.tool_ok(&app.worker, "takomo_start", json!({ "id": id }))
        .await;
    let mut args = json!({ "id": id });
    for (k, v) in q.as_object().unwrap() {
        args[k] = v.clone();
    }
    let asked = app.tool_ok(&app.worker, "takomo_ask", args).await;
    let qid = asked["question"]["id"].as_str().unwrap().to_string();
    (id, qid)
}

/// An answer link is a bearer credential handed to someone outside the org, so
/// `takomo_answer_link` must carry exactly the guarantees
/// `POST /v1/questions/{id}/answer-link` does — same delegation gate, same
/// lifetime precedence, same shown-once warning. It used to carry its own copy of
/// that policy, and had already drifted: it ignored the project's
/// `answer_link_ttl_seconds` entirely and reported neither the applied TTL nor
/// where it came from.
#[tokio::test]
async fn mcp_answer_link_matches_the_rest_minting_policy() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;
    let (_, qid) = mcp_parked_question(
        &app,
        "mcp answer link",
        json!({ "kind": "confirm", "title": "Ship it?" }),
    )
    .await;

    // No project default set: the built-in 7 days, reported as such.
    let minted = app
        .tool_ok(&app.admin, "takomo_answer_link", json!({ "id": qid }))
        .await;
    let link = &minted["answer_link"];
    assert_eq!(link["ttl_seconds"], 604_800, "{link}");
    assert_eq!(link["ttl_source"], "default", "{link}");
    assert!(link["path"].as_str().unwrap().contains("#a="), "{link}");
    assert!(
        link["warning"]
            .as_str()
            .unwrap_or_default()
            .contains("shown ONCE"),
        "the credential warning is the same one every mint carries: {link}"
    );
    // The grant is real: the outsider answers this one question with it.
    let token = link["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("tka_"), "{token}");
    let (s, answered) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");

    // A project default now applies to MCP mints too — the drift this closes.
    let (s, updated) = app
        .put(
            &app.admin,
            "/v1/projects/tp/answer-link-ttl",
            json!({ "ttl_seconds": 86_400 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{updated}");
    let (_, q2) = mcp_parked_question(
        &app,
        "mcp answer link 2",
        json!({ "kind": "confirm", "title": "And this?" }),
    )
    .await;
    let link = app
        .tool_ok(&app.admin, "takomo_answer_link", json!({ "id": q2 }))
        .await["answer_link"]
        .clone();
    assert_eq!(link["ttl_seconds"], 86_400, "{link}");
    assert_eq!(link["ttl_source"], "project", "{link}");

    // An explicit ttl still wins outright.
    let link = app
        .tool_ok(
            &app.admin,
            "takomo_answer_link",
            json!({ "id": q2, "ttl_seconds": 3_600 }),
        )
        .await["answer_link"]
        .clone();
    assert_eq!(link["ttl_seconds"], 3_600, "{link}");
    assert_eq!(link["ttl_source"], "explicit", "{link}");

    // The two out-of-range cases are distinguished, not collapsed into one
    // message: an agent that sent 0 and one that sent 60 days need different
    // corrections.
    let (err, is_err) = app
        .tool(
            &app.admin,
            "takomo_answer_link",
            json!({ "id": q2, "ttl_seconds": 0 }),
        )
        .await;
    assert!(is_err, "{err}");
    assert_eq!(err["code"], "answer_link.ttl", "{err}");
    assert!(
        err["message"].as_str().unwrap().contains("positive"),
        "{err}"
    );
    let (err, is_err) = app
        .tool(
            &app.admin,
            "takomo_answer_link",
            json!({ "id": q2, "ttl_seconds": 2_592_001 }),
        )
        .await;
    assert!(is_err, "{err}");
    assert_eq!(err["code"], "answer_link.ttl", "{err}");
    assert!(
        err["message"].as_str().unwrap().contains("maximum"),
        "{err}"
    );

    // A write-only worker cannot delegate at all.
    let (err, is_err) = app
        .tool(&app.worker, "takomo_answer_link", json!({ "id": q2 }))
        .await;
    assert!(is_err, "minting needs the human scope: {err}");
    assert_eq!(err["code"], "auth.scope", "{err}");
}

/// You can only delegate authority you hold: minting for an `approve` question
/// needs the matching `expert:<tag>` scope on **both** surfaces, and the refusal
/// names the scope that would satisfy it.
#[tokio::test]
async fn mcp_answer_link_delegates_approve_only_with_expertise() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;
    let (_, qid) = mcp_parked_question(
        &app,
        "mcp approve link",
        json!({ "kind": "approve", "title": "OK legally?", "expertise": ["domain:legal"] }),
    )
    .await;

    // A plain human — even an admin — holds no domain expertise here.
    let (err, is_err) = app
        .tool(&app.admin, "takomo_answer_link", json!({ "id": qid }))
        .await;
    assert!(is_err, "{err}");
    assert_eq!(err["code"], "question.approve_expertise", "{err}");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("expert:domain:legal"),
        "the refusal must name the scope that would satisfy it: {err}"
    );

    // The domain expert mints it, and the link satisfies the approve gate.
    let counsel = app.mint(
        "human:counsel",
        &["read", "write", "human", "expert:domain:legal"],
        None,
    );
    let link = app
        .tool_ok(&counsel, "takomo_answer_link", json!({ "id": qid }))
        .await["answer_link"]
        .clone();
    let token = link["token"].as_str().unwrap().to_string();
    let (s, answered) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["ticket"]["state"], "ready", "{answered}");
}

/// A question that is no longer open has nothing to delegate.
#[tokio::test]
async fn mcp_answer_link_refuses_a_closed_question() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;
    let (_, qid) = mcp_parked_question(
        &app,
        "mcp closed link",
        json!({ "kind": "confirm", "title": "Ship it?" }),
    )
    .await;
    app.tool_ok(
        &app.admin,
        "takomo_answer",
        json!({ "id": qid, "answer": "yes" }),
    )
    .await;
    let (err, is_err) = app
        .tool(&app.admin, "takomo_answer_link", json!({ "id": qid }))
        .await;
    assert!(is_err, "{err}");
    assert_eq!(err["code"], "question.not_open", "{err}");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("nothing to answer"),
        "{err}"
    );
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

/// The project-conventions nudge on an ask is agent-facing *instruction*, so REST
/// and MCP must not word it differently — an agent would be told to follow a
/// different house style depending on the transport it happened to use. Both
/// notes now come from one function, and this pins the wording so a future edit
/// to either surface cannot quietly reintroduce the split.
#[tokio::test]
async fn ask_conventions_nudge_is_identical_on_rest_and_mcp() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;
    for (path, body) in [
        ("/v1/projects/tp/language", json!({ "language": "German" })),
        (
            "/v1/projects/tp/style",
            json!({ "style_guide": "Two sentences max." }),
        ),
    ] {
        let (s, out) = app.put(&app.admin, path, body).await;
        assert_eq!(s, StatusCode::OK, "{out}");
    }
    const LANG: &str = " This project expects the question (and any options) written in German — re-ask in German if this one wasn't.";
    const STYLE: &str = " This project's style guide for what you write: Two sentences max.";

    // Ask the same question over both surfaces, on its own ticket each time, and
    // compare the tail each one appends.
    let id = app.create_ticket("nudge over rest").await;
    let fence = app.to_implementing(&id).await;
    let (_, rest) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Weiter?", "fence": fence }),
        )
        .await;
    let rest_note = rest["note"].as_str().unwrap();

    let created = app
        .tool_ok(
            &app.worker,
            "takomo_new",
            json!({ "project": "tp", "title": "nudge over mcp", "type": "task" }),
        )
        .await;
    let mcp_id = created["ticket"]["id"].as_str().unwrap().to_string();
    for to in ["spec", "ready"] {
        app.tool_ok(
            &app.admin,
            "takomo_transition",
            json!({ "id": mcp_id, "to": to }),
        )
        .await;
    }
    app.tool_ok(&app.worker, "takomo_start", json!({ "id": mcp_id }))
        .await;
    let mcp = app
        .tool_ok(
            &app.worker,
            "takomo_ask",
            json!({ "id": mcp_id, "kind": "confirm", "title": "Weiter?" }),
        )
        .await;
    let mcp_note = mcp["note"].as_str().unwrap();

    for note in [rest_note, mcp_note] {
        assert!(note.ends_with(&format!("{LANG}{STYLE}")), "note: {note}");
    }
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

/// Move a freshly created ticket to `ready`, the state real work is claimed in.
/// The test workflow starts tickets in `brief`, which is not claimable, and it is
/// `ready` -> `implementing` that is claim-gated — so this is the state that shows
/// whether a renewed lease still authorises work.
async fn claimable(app: &TestApp, title: &str) -> String {
    let id = app.create_ticket(title).await;
    for to in ["spec", "ready"] {
        app.tool_ok(
            &app.admin,
            "takomo_transition",
            json!({ "id": id, "to": to }),
        )
        .await;
    }
    id
}

/// An MCP-only agent must be able to keep a lease alive (takomo-m3yl).
///
/// Before this, `POST /v1/tickets/{id}/heartbeat` existed but had no MCP tool and
/// no MCP claim path took `ttl_seconds` — so an agent on that transport got 900
/// seconds, could not extend them, and could not ask for more up front. Long work
/// then lost its claim, fencing correctly refused its writes, and the only way
/// forward was back through a claimable state, giving up the ticket's place in the
/// queue. Both levers are asserted here because either one alone still leaves the
/// agent unable to hold work it is actively doing.
#[tokio::test]
async fn mcp_can_hold_a_lease_it_is_working_on() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;

    let list = app.ok_call(&app.admin, "tools/list", json!({})).await;
    let names: Vec<&str> = list["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"takomo_heartbeat"),
        "the heartbeat REST route needs an MCP tool, or one transport cannot renew: {names:?}"
    );

    let id = claimable(&app, "hold this lease").await;

    // Claiming with an explicit ttl must actually shorten the lease: a tool that
    // accepted `ttl_seconds` and dropped it would pass a smoke test but leave the
    // agent on the 900s default it was trying to change.
    let short = app
        .tool_ok(
            &app.admin,
            "takomo_claim",
            json!({ "id": id, "ttl_seconds": 5 }),
        )
        .await;
    let short_expiry = short["lease"]["expires_at"]
        .as_str()
        .expect("lease carries expires_at")
        .to_string();
    let fence = short["lease"]["fence"].as_i64().expect("fence");
    // Bound it against the wall clock, not just against the next lease. A tool
    // that accepted `ttl_seconds` and dropped it would hand back the 900s default,
    // and every "later than the previous one" comparison would still pass.
    let short_secs = (chrono::DateTime::parse_from_rfc3339(&short_expiry)
        .expect("expires_at is RFC3339")
        .timestamp()
        - chrono::Utc::now().timestamp())
    .max(0);
    assert!(
        short_secs <= 60,
        "ttl_seconds=5 must actually shorten the lease, but it expires in {short_secs}s          — the 900s default was used and the argument was dropped"
    );

    // …and heartbeating must push it out. Compared against the 5s lease above, so
    // this asserts renewal, not merely that a second call succeeds.
    let beat = app
        .tool_ok(
            &app.admin,
            "takomo_heartbeat",
            json!({ "id": id, "ttl_seconds": 900 }),
        )
        .await;
    let beat_expiry = beat["lease"]["expires_at"]
        .as_str()
        .expect("renewed lease carries expires_at")
        .to_string();
    assert!(
        beat_expiry > short_expiry,
        "heartbeat must extend the lease: {short_expiry} -> {beat_expiry}"
    );
    assert_eq!(
        beat["lease"]["fence"].as_i64(),
        Some(fence),
        "a beat renews the lease without bumping the fence — a new fence would \
         invalidate the writes of the very worker that is holding on"
    );

    // The fence is resolved from the store, so the caller never has to track it;
    // an explicit one still works, which is what a wrapper holding it in memory
    // passes.
    app.tool_ok(
        &app.admin,
        "takomo_heartbeat",
        json!({ "id": id, "fence": fence }),
    )
    .await;

    // Still the holder's, so claim-gated work proceeds.
    let started = app
        .tool_ok(&app.admin, "takomo_start", json!({ "id": id }))
        .await;
    assert_eq!(started["ticket"]["state"].as_str().unwrap(), "implementing");
}

/// Heartbeat's refusals have to teach, because each one means the agent is about
/// to lose (or has already lost) work it thinks it holds.
#[tokio::test]
async fn mcp_heartbeat_refusals_say_what_to_do() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;
    let id = claimable(&app, "never claimed").await;

    // Never held it: distinct from "held it and lost it", and the remedy differs
    // (claim first, versus stop writing).
    let (payload, is_error) = app
        .tool(&app.admin, "takomo_heartbeat", json!({ "id": id }))
        .await;
    assert!(is_error, "heartbeat without a lease must fail: {payload}");
    assert_eq!(
        payload["code"].as_str(),
        Some("heartbeat.no_lease"),
        "unclaimed heartbeat needs its own code, not a fence mismatch on a lease \
         that was never this caller's: {payload}"
    );
    assert!(
        payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("takomo_claim"),
        "the message must name the MCP tool that fixes it: {payload}"
    );

    // Someone else's lease is not renewable by us, whatever fence we present.
    app.tool_ok(&app.worker, "takomo_claim", json!({ "id": id }))
        .await;
    let (stolen, is_error) = app
        .tool(&app.admin, "takomo_heartbeat", json!({ "id": id }))
        .await;
    assert!(
        is_error,
        "heartbeating another actor's lease must fail: {stolen}"
    );

    // An expired lease cannot be revived — that is the whole point of a lease, so
    // the agent has to learn it from the error rather than from a silent success.
    let other = claimable(&app, "expires immediately").await;
    app.tool_ok(
        &app.admin,
        "takomo_claim",
        json!({ "id": other, "ttl_seconds": 1 }),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let (expired, is_error) = app
        .tool(&app.admin, "takomo_heartbeat", json!({ "id": other }))
        .await;
    assert!(
        is_error,
        "an expired lease must not be heartbeatable back to life: {expired}"
    );
}

// ---- takomo_link: one key per call, `null` deletes (takomo-12ax) ------------

/// `takomo_link` must send only the key it was asked to write.
///
/// It used to read the ticket's `links` (outside the transaction), insert the new
/// key client-side, and send the whole object back — a read-modify-write straddling
/// a transaction boundary. The store already merges links per key *inside* the
/// transaction, so the pre-merge bought nothing and cost a lost update: any key
/// deleted between the tool's read and its write came back from the dead. REST,
/// which sends `{"<key>": <value>}` and lets the store merge, cannot lose an
/// update this way.
///
/// The interleaving is forced rather than raced. A second connection to the same
/// SQLite file takes the write lock, so the tool's write parks behind it (the
/// store's `busy_timeout` is 5s) *after* it has already read `links`; the delete
/// then commits inside that window. The raw `UPDATE` is exactly what the store's
/// per-key merge issues for `links: {"branch": null}` — the point of doing it on
/// the other connection is the timing, not a different code path (the
/// `null`-deletes-through-REST path is asserted below).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_link_cannot_resurrect_a_link_deleted_mid_call() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;
    let id = app.create_ticket("mcp link lost update").await;

    // The link that must stay deleted once someone deletes it.
    let first = app
        .tool_ok(
            &app.admin,
            "takomo_link",
            json!({ "id": id, "key": "branch", "value": "feat/doomed" }),
        )
        .await;
    assert_eq!(first["links"]["branch"], "feat/doomed");

    let conn = rusqlite::Connection::open(app.db_path()).expect("second connection");
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    conn.execute_batch("BEGIN IMMEDIATE")
        .expect("take write lock");

    let deleter_id = id.clone();
    let deleter = std::thread::spawn(move || {
        // Long enough that the tool call below has certainly read the ticket and
        // is now blocked on the write lock.
        std::thread::sleep(std::time::Duration::from_millis(400));
        conn.execute(
            "UPDATE tickets SET links = ?2 WHERE id = ?1",
            (&deleter_id, "{}"),
        )
        .expect("delete the branch link");
        conn.execute_batch("COMMIT").expect("commit the delete");
    });

    // A *different* key, so nothing about this call is about `branch` — yet the
    // old client-side merge would carry `branch` along and re-insert it.
    let linked = app
        .tool_ok(
            &app.admin,
            "takomo_link",
            json!({ "id": id, "key": "pr", "value": "https://example.test/pr/1" }),
        )
        .await;
    deleter.join().expect("deleter thread");

    assert_eq!(linked["links"]["pr"], "https://example.test/pr/1");
    assert!(
        linked["links"].get("branch").is_none(),
        "takomo_link resurrected a link deleted after it read the ticket — it must \
         send only the key it was given and let the store merge: {linked}"
    );

    // …and that is the stored state, not just what the tool echoed.
    let (status, ticket) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        ticket["links"].get("branch").is_none(),
        "the deleted link is back in the store: {}",
        ticket["links"]
    );
    assert_eq!(ticket["links"]["pr"], "https://example.test/pr/1");
}

/// The other half of the asymmetry: `value` was `String`, so MCP could set a link
/// but never delete one, while REST has deleted with `links: {"key": null}` all
/// along. Also pins the merge the tool depends on — writing one key must leave the
/// others alone.
#[tokio::test]
async fn mcp_link_deletes_with_null_and_leaves_other_keys_alone() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;
    let id = app.create_ticket("mcp link delete").await;

    app.tool_ok(
        &app.admin,
        "takomo_link",
        json!({ "id": id, "key": "branch", "value": "feat/x" }),
    )
    .await;
    // A second key set through REST, to prove the tool merges rather than replaces.
    let (status, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "design": "https://example.test/doc" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let added = app
        .tool_ok(
            &app.admin,
            "takomo_link",
            json!({ "id": id, "key": "pr", "value": "https://example.test/pr/2" }),
        )
        .await;
    assert_eq!(added["links"]["branch"], "feat/x");
    assert_eq!(added["links"]["design"], "https://example.test/doc");
    assert_eq!(added["links"]["pr"], "https://example.test/pr/2");

    // null deletes exactly that key.
    let deleted = app
        .tool_ok(
            &app.admin,
            "takomo_link",
            json!({ "id": id, "key": "branch", "value": null }),
        )
        .await;
    assert!(
        deleted["links"].get("branch").is_none(),
        "value=null must delete the key: {deleted}"
    );
    assert_eq!(deleted["links"]["pr"], "https://example.test/pr/2");
    assert_eq!(deleted["links"]["design"], "https://example.test/doc");

    // Omitting `value` entirely is the same request (`Option<String>` → null), and
    // deleting a key that is not there is a no-op, not an error: an agent cleaning
    // up should not have to read first to know whether it may.
    let absent = app
        .tool_ok(
            &app.admin,
            "takomo_link",
            json!({ "id": id, "key": "never-set" }),
        )
        .await;
    assert!(
        absent["links"].get("never-set").is_none(),
        "deleting an unset key must not create it: {absent}"
    );
    assert_eq!(absent["links"]["pr"], "https://example.test/pr/2");

    // Same view over REST — the two transports agree on the stored links.
    let (status, ticket) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        ticket["links"],
        json!({ "design": "https://example.test/doc", "pr": "https://example.test/pr/2" })
    );
}

// ---------------------------------------------------------------------------
// Initiatives over MCP: the surface an agent actually uses to open an idea and
// feed it over time.

/// The whole loop an agent runs: open an initiative, append a note, append a
/// colleague's feedback, attach a document, then read back what accumulated.
#[tokio::test]
async fn initiative_accumulates_inputs_over_mcp() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;

    let created = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_new",
            json!({
                "project": "tp",
                "title": "Name the thing",
                "summary": "Every project needs a good name.",
                "labels": ["naming"],
                "tags": ["person:ada"],
            }),
        )
        .await;
    let id = created["initiative"]["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("ini-"), "unexpected id shape: {id}");
    assert_eq!(created["initiative"]["status"], "open");
    assert_eq!(created["initiative"]["rollup"]["entries"], 0);
    assert_eq!(created["initiative"]["rollup"]["megabytes"], 0.0);

    // A research finding from an agent.
    let appended = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_append",
            json!({
                "id": id,
                "kind": "research",
                "source": "agent:w1",
                "source_uri": "https://example.test/trademark-search",
                "title": "Trademark landscape",
                "text": "No conflicting marks in class 42.",
            }),
        )
        .await;
    assert_eq!(appended["entry"]["kind"], "research");
    assert_eq!(appended["entry"]["source"], "agent:w1");
    assert_eq!(appended["entry"]["has_content"], false);
    // The append response already reports what the collection now weighs, so an
    // agent never has to follow up with a read to know.
    assert_eq!(appended["initiative"]["rollup"]["entries"], 1);
    assert_eq!(
        appended["initiative"]["rollup"]["chars"],
        "No conflicting marks in class 42.".chars().count()
    );

    // A colleague's feedback, written earlier and pasted in now: two different
    // timestamps, both correct.
    let feedback = app
        .tool_ok(
            &app.human,
            "takomo_initiative_append",
            json!({
                "id": id,
                "kind": "feedback",
                "source": "person:ada",
                "text": "Prefer something pronounceable in German.",
                "origin_at": "2026-07-01T09:00:00Z",
            }),
        )
        .await;
    assert_eq!(feedback["entry"]["origin_at"], "2026-07-01T09:00:00.000Z");
    assert_eq!(feedback["entry"]["author"], "human:reviewer");
    assert_ne!(
        feedback["entry"]["origin_at"], feedback["entry"]["created_at"],
        "when it was written and when it landed are separate facts"
    );

    // An attached document.
    let doc = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_append",
            json!({
                "id": id,
                "kind": "document",
                "source": "person:ada",
                "text": "Full search report.",
                // "hello takomo"
                "content_base64": "aGVsbG8gdGFrb21v",
                "mime": "text/plain",
                "filename": "report.txt",
            }),
        )
        .await;
    assert_eq!(doc["entry"]["has_content"], true);
    assert_eq!(doc["entry"]["content_bytes"], 12);
    let rollup = &doc["initiative"]["rollup"];
    assert_eq!(rollup["entries"], 3);
    assert_eq!(rollup["attachments"], 1);
    assert_eq!(rollup["attachment_bytes"], 12);

    // takomo_initiative_show returns the rollup plus the entries, newest first.
    let shown = app
        .tool_ok(&app.worker, "takomo_initiative_show", json!({ "id": id }))
        .await;
    let entries = shown["initiative"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["kind"], "document");
    assert_eq!(entries[2]["kind"], "research");
    assert_eq!(shown["initiative"]["rollup"]["entries"], 3);

    // And the bytes come back over the REST content route, byte for byte.
    let entry_id = entries[0]["id"].as_str().unwrap();
    let resp = app
        .authed(
            Method::GET,
            &app.worker,
            &format!("/v1/initiatives/{id}/entries/{entry_id}/content"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"hello takomo");
}

/// The list tool, and the fact that `person:ada` on an initiative means the same
/// thing it means on a ticket — it registers in the project's tag registry.
#[tokio::test]
async fn initiative_list_filters_and_shares_the_tag_registry() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;
    for (title, status) in [("Naming", "open"), ("Pricing", "parked")] {
        app.tool_ok(
            &app.worker,
            "takomo_initiative_new",
            json!({ "project": "tp", "title": title, "status": status, "tags": ["person:ada"] }),
        )
        .await;
    }

    let listed = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_list",
            json!({ "project": "tp", "status": "open" }),
        )
        .await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    assert_eq!(listed["items"][0]["title"], "Naming");

    let by_tag = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_list",
            json!({ "project": "tp", "tag": "person:ada" }),
        )
        .await;
    assert_eq!(by_tag["items"].as_array().unwrap().len(), 2);

    // Tagging lazily registered the handle, exactly as it does from a ticket.
    let (status, registry) = app
        .get(&app.worker, "/v1/projects/tp/tags?kind=person")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(registry["items"][0]["ref"], "person:ada");
}

/// Status is a label an owner moves, not a workflow: nothing gates the order, and
/// `distilled` is how an initiative records that it became tickets.
#[tokio::test]
async fn initiative_update_edits_the_description_only() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;
    let created = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_new",
            json!({ "project": "tp", "title": "Name the thing" }),
        )
        .await;
    let id = created["initiative"]["id"].as_str().unwrap().to_string();

    let updated = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_update",
            json!({
                "id": id,
                "status": "distilled",
                "summary": "Became takomo-9xyz.",
                "metadata_merge": { "distilled_into": ["tp-1a2b"] },
            }),
        )
        .await;
    assert_eq!(updated["initiative"]["status"], "distilled");
    assert_eq!(updated["initiative"]["summary"], "Became takomo-9xyz.");
    assert_eq!(
        updated["initiative"]["metadata"]["distilled_into"][0],
        "tp-1a2b"
    );
    assert_eq!(updated["initiative"]["version"], 2);

    // A patch with nothing in it is a teaching refusal, not a silent no-op that
    // bumps the version.
    let (body, is_error) = app
        .tool(&app.worker, "takomo_initiative_update", json!({ "id": id }))
        .await;
    assert!(is_error);
    assert_eq!(body["code"], "validation.no_changes");

    // A parked initiative is still appendable — parking is not closing.
    app.tool_ok(
        &app.worker,
        "takomo_initiative_update",
        json!({ "id": id, "status": "parked" }),
    )
    .await;
    app.tool_ok(
        &app.worker,
        "takomo_initiative_append",
        json!({ "id": id, "kind": "note", "source": "agent:w1", "text": "One more thought." }),
    )
    .await;
}

/// Every refusal an agent can walk into on the append path, and each has to say
/// what to do instead.
#[tokio::test]
async fn initiative_append_refuses_bad_input_with_guidance() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;
    let created = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_new",
            json!({ "project": "tp", "title": "Name the thing" }),
        )
        .await;
    let id = created["initiative"]["id"].as_str().unwrap().to_string();

    let cases: Vec<(&str, Value)> = vec![
        // Neither text nor an attachment: provenance for nothing.
        (
            "validation.entry_empty",
            json!({ "id": id, "kind": "note", "source": "agent:w1" }),
        ),
        // Provenance is the point of an entry, so it is required.
        (
            "validation.entry_source",
            json!({ "id": id, "kind": "note", "source": "  ", "text": "x" }),
        ),
        // A free-form kind is still a slug.
        (
            "validation.entry_kind",
            json!({ "id": id, "kind": "Research Notes", "source": "agent:w1", "text": "x" }),
        ),
        // A mangled upload must never become silently truncated bytes.
        (
            "validation.entry_content_base64",
            json!({
                "id": id, "kind": "document", "source": "agent:w1",
                "content_base64": "not base64!!", "mime": "text/plain",
            }),
        ),
        // Bytes nobody can label are bytes nobody can use.
        (
            "validation.entry_attachment_unlabeled",
            json!({
                "id": id, "kind": "document", "source": "agent:w1",
                "content_base64": "aGk=",
            }),
        ),
        // The media type is served back in a header, so it is restricted to one.
        (
            "validation.entry_mime",
            json!({
                "id": id, "kind": "document", "source": "agent:w1",
                "content_base64": "aGk=", "mime": "text/plain; charset=\"x\"\r\nX-Evil: 1",
            }),
        ),
        // A wrong provenance date is worse than a missing one.
        (
            "validation.origin_at",
            json!({
                "id": id, "kind": "note", "source": "agent:w1", "text": "x",
                "origin_at": "last tuesday",
            }),
        ),
    ];
    for (code, args) in cases {
        let (body, is_error) = app
            .tool(&app.worker, "takomo_initiative_append", args.clone())
            .await;
        assert!(is_error, "{code} should have been refused: {body}");
        assert_eq!(body["code"], code, "wrong code for {args}: {body}");
        assert!(
            body["message"].as_str().unwrap_or_default().len() > 40,
            "a refusal must teach, not just reject: {body}"
        );
    }

    // An unknown initiative is a 404 even for a well-formed entry.
    let (body, is_error) = app
        .tool(
            &app.worker,
            "takomo_initiative_append",
            json!({ "id": "ini-nope", "kind": "note", "source": "agent:w1", "text": "x" }),
        )
        .await;
    assert!(is_error);
    assert_eq!(body["status"], 404);
}

/// The attachment cap is what keeps a document upload from stalling every claim
/// and transition in the store, so it is enforced on the decoded bytes.
#[tokio::test]
async fn initiative_attachment_cap_is_enforced_on_decoded_bytes() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;
    let created = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_new",
            json!({ "project": "tp", "title": "Big ideas" }),
        )
        .await;
    let id = created["initiative"]["id"].as_str().unwrap().to_string();

    let over = takomo::store::MAX_ENTRY_CONTENT_BYTES + 1;
    let encoded = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(vec![b'x'; over])
    };
    let (body, is_error) = app
        .tool(
            &app.worker,
            "takomo_initiative_append",
            json!({
                "id": id, "kind": "document", "source": "agent:w1",
                "content_base64": encoded, "filename": "huge.bin",
            }),
        )
        .await;
    assert!(is_error);
    assert_eq!(body["code"], "initiative.attachment_too_large");
    assert_eq!(body["details"]["bytes"], over);
    assert_eq!(
        body["details"]["max_bytes"],
        takomo::store::MAX_ENTRY_CONTENT_BYTES
    );
    assert!(
        body["remedy"]
            .as_str()
            .unwrap_or_default()
            .contains("source_uri"),
        "the remedy should point at referencing the document instead: {body}"
    );

    // An attachment at exactly the cap lands — and this is doing more work than it
    // looks. At 5 MiB the base64 body is ~7 MB, well past axum's 2 MB default
    // request-body limit, so this asserts that a caller can actually reach the cap
    // the error message promises. A limit applied in front of /mcp would fail here
    // with a transport error instead of an entry.
    let at_cap = takomo::store::MAX_ENTRY_CONTENT_BYTES;
    let ok = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(vec![b'x'; at_cap])
    };
    assert!(
        ok.len() > 2 * 1024 * 1024,
        "this test is only meaningful if the encoded body exceeds the default 2 MB \
         body limit; it is {} bytes",
        ok.len()
    );
    let landed = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_append",
            json!({
                "id": id, "kind": "document", "source": "agent:w1",
                "content_base64": ok, "filename": "exactly-the-cap.bin",
            }),
        )
        .await;
    assert_eq!(landed["entry"]["content_bytes"], at_cap);
    assert_eq!(landed["initiative"]["rollup"]["megabytes"], 5.0);

    // And the bytes come back out whole, all 5 MiB of them, through the content
    // route — the response path has no size limit of its own either.
    let entry_id = landed["entry"]["id"].as_str().unwrap();
    let resp = app
        .authed(
            Method::GET,
            &app.worker,
            &format!("/v1/initiatives/{id}/entries/{entry_id}/content"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().len(), at_cap);
}

/// The per-initiative total is a separate bound from the per-attachment one, and
/// refuses the entry that would cross it rather than truncating anything.
#[tokio::test]
async fn initiative_total_size_cap_is_separate_from_the_attachment_cap() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;
    let created = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_new",
            json!({ "project": "tp", "title": "Big ideas" }),
        )
        .await;
    let id = created["initiative"]["id"].as_str().unwrap().to_string();

    // Filling 1 GiB honestly would mean ~205 max-size uploads and a gigabyte of
    // disk in a unit test. The rollup sums the `text_bytes` / `content_bytes`
    // columns, so the nearly-full state is staged by writing ONE row whose recorded
    // size is large while its stored text is not — synthetic in the size column,
    // exact in the arithmetic this test is about. The entry that crosses the line
    // then goes through the real tool, over real HTTP.
    let near = takomo::store::MAX_INITIATIVE_BYTES - 16;
    {
        let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
        conn.execute(
            "INSERT INTO initiative_entries (id, initiative, project, kind, text, chars, \
             text_bytes, content_bytes, source, meta, author, created_at) \
             VALUES ('ie-staged', ?1, 'tp', 'note', 'staged', 6, ?2, 0, 'test:setup', '{}', \
             'test:setup', 0)",
            rusqlite::params![id, near],
        )
        .expect("stage a nearly-full initiative");
    }

    let (body, is_error) = app
        .tool(
            &app.worker,
            "takomo_initiative_append",
            json!({
                "id": id, "kind": "note", "source": "agent:w1",
                "text": "seventeen chars..",
            }),
        )
        .await;
    assert!(is_error, "the crossing entry should be refused: {body}");
    assert_eq!(body["code"], "initiative.too_large");
    assert_eq!(body["status"], 409);
    assert_eq!(
        body["details"]["max_bytes"],
        takomo::store::MAX_INITIATIVE_BYTES
    );
    assert!(
        body["details"]["would_be"].as_i64().unwrap() > takomo::store::MAX_INITIATIVE_BYTES,
        "details should show the size it would have reached: {body}"
    );

    // Nothing landed, and an entry that still fits is accepted — the cap refuses
    // the crossing write, it does not close the initiative.
    let shown = app
        .tool_ok(&app.worker, "takomo_initiative_show", json!({ "id": id }))
        .await;
    assert_eq!(shown["initiative"]["rollup"]["entries"], 1);
    app.tool_ok(
        &app.worker,
        "takomo_initiative_append",
        json!({ "id": id, "kind": "note", "source": "agent:w1", "text": "tiny" }),
    )
    .await;
}

/// Initiative writes are writes: they need the `write` scope, they debit the
/// per-token write budget, and they are bounded by the project allowlist. The two
/// read tools are free, like every other read on this surface.
#[tokio::test]
async fn initiative_tools_respect_scopes_projects_and_the_write_budget() {
    let app = TestApp::spawn().await;
    let reader = app.mint("agent:reader", &["read"], None);
    let outsider = app.mint("agent:elsewhere", &["read", "write"], Some(&["other"]));
    app.ok_call(&app.worker, "initialize", init_params()).await;

    let created = app
        .tool_ok(
            &app.worker,
            "takomo_initiative_new",
            json!({ "project": "tp", "title": "Name the thing" }),
        )
        .await;
    let id = created["initiative"]["id"].as_str().unwrap().to_string();

    // read-only token: reads work, writes do not.
    app.tool_ok(&reader, "takomo_initiative_show", json!({ "id": id }))
        .await;
    let (body, is_error) = app
        .tool(
            &reader,
            "takomo_initiative_append",
            json!({ "id": id, "kind": "note", "source": "x", "text": "y" }),
        )
        .await;
    assert!(is_error);
    assert_eq!(body["status"], 403);

    // A token scoped to another project cannot reach this initiative by id, on
    // either the read or the write tool.
    for tool in ["takomo_initiative_show", "takomo_initiative_append"] {
        let (body, is_error) = app
            .tool(
                &outsider,
                tool,
                json!({ "id": id, "kind": "note", "source": "x", "text": "y" }),
            )
            .await;
        assert!(is_error, "{tool} should be refused: {body}");
        assert_eq!(body["status"], 403, "{tool}: {body}");
    }
    // …nor create one there.
    let (body, is_error) = app
        .tool(
            &outsider,
            "takomo_initiative_new",
            json!({ "project": "tp", "title": "Sneaky" }),
        )
        .await;
    assert!(is_error);
    assert_eq!(body["status"], 403);

    // The write tools are declared writes, so they are metered. A budget of one
    // write buys exactly one append; the read tools stay free afterwards.
    let thrifty = app.mint_limited("agent:thrifty", &["read", "write"], None, 1);
    app.tool_ok(
        &thrifty,
        "takomo_initiative_append",
        json!({ "id": id, "kind": "note", "source": "agent:thrifty", "text": "first" }),
    )
    .await;
    let (body, is_error) = app
        .tool(
            &thrifty,
            "takomo_initiative_append",
            json!({ "id": id, "kind": "note", "source": "agent:thrifty", "text": "second" }),
        )
        .await;
    assert!(is_error);
    assert_eq!(
        body["status"], 429,
        "a second write should be refused: {body}"
    );
    app.tool_ok(&thrifty, "takomo_initiative_list", json!({}))
        .await;
}

/// The five initiative tools must be discoverable, and the two read tools must be
/// declared reads — an unlisted name is charged as a write, which is the safe
/// direction but wrong for a read.
#[tokio::test]
async fn initiative_tools_are_discoverable_and_classified() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;
    let listed = app.ok_call(&app.worker, "tools/list", json!({})).await;
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "takomo_initiative_new",
        "takomo_initiative_append",
        "takomo_initiative_update",
        "takomo_initiative_list",
        "takomo_initiative_show",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
    assert!(takomo::mcp::READ_TOOLS.contains(&"takomo_initiative_list"));
    assert!(takomo::mcp::READ_TOOLS.contains(&"takomo_initiative_show"));
    assert!(!takomo::mcp::READ_TOOLS.contains(&"takomo_initiative_append"));
}

/// The whole checklist loop an agent actually runs, over MCP: file a check, file
/// its generated cases, record a verdict, push the release you merged, then read
/// what that invalidated. This is the surface the feature exists to serve — a
/// human never has to touch any of it.
#[tokio::test]
async fn mcp_drives_the_full_checklist_loop() {
    let app = TestApp::spawn().await;

    let (check, is_err) = app
        .tool(
            &app.worker,
            "takomo_check_file",
            json!({
                "project": "tp",
                "title": "Create a claim",
                "layer": "ui",
                "severity": "blocking",
                "body": "Open claims, start one, submit it.",
                "globs": ["src/claims/**"],
            }),
        )
        .await;
    assert!(!is_err, "check_file failed: {check}");
    let check_id = check["id"].as_str().expect("check id").to_string();
    assert_eq!(check["policy"]["verification"], "agent");

    let (filed, is_err) = app
        .tool(
            &app.worker,
            "takomo_cases_file",
            json!({
                "check": check_id,
                "cases": [
                    { "key": "happy", "label": "happy path", "seeded": true,
                      "assignment": { "guardian": "none" } },
                    { "key": "guardian", "label": "guardian required",
                      "assignment": { "guardian": "required" } },
                ],
            }),
        )
        .await;
    assert!(!is_err, "cases_file failed: {filed}");
    assert_eq!(filed["added"], 2);
    assert_eq!(filed["live"], 2);

    // The worklist is what an agent asks for rather than reasoning over the tree.
    let (wl, _) = app
        .tool(&app.worker, "takomo_worklist", json!({ "project": "tp" }))
        .await;
    assert_eq!(wl["agent"]["cases"], 2, "{wl}");
    assert_eq!(wl["human"]["cases"], 0);
    let first_case = wl["agent"]["items"][0]["case"]
        .as_str()
        .expect("a case id")
        .to_string();

    let (verdict, is_err) = app
        .tool(
            &app.worker,
            "takomo_verdict",
            json!({ "case": first_case, "verdict": "pass" }),
        )
        .await;
    assert!(!is_err, "verdict failed: {verdict}");
    assert_eq!(verdict["agent"]["verdict"], "pass");
    assert_eq!(verdict["state"], "verified");

    // Pushing the release reports back what it invalidated, so the agent learns
    // the consequence of its own merge without a second call.
    let (rel, is_err) = app
        .tool(
            &app.worker,
            "takomo_release_push",
            json!({
                "project": "tp",
                "ref": "v2.0.0",
                "touched_paths": ["src/claims/create.rs"],
                "orphan_globs": [],
            }),
        )
        .await;
    assert!(!is_err, "release_push failed: {rel}");
    assert_eq!(rel["seq"], 1);
    assert_eq!(rel["impact"]["stale_cases"], 1, "the verified case: {rel}");

    let (gate, _) = app
        .tool(&app.worker, "takomo_gate", json!({ "project": "tp" }))
        .await;
    assert_eq!(
        gate["blocked"], true,
        "a blocking check is unverified: {gate}"
    );

    let (cov, _) = app
        .tool(&app.worker, "takomo_coverage", json!({ "project": "tp" }))
        .await;
    assert_eq!(cov["cases"]["total"], 2);
    assert_eq!(cov["cases"]["stale"], 1);
    assert_eq!(cov["cases"]["never"], 1);
    assert_eq!(cov["percent"], 0, "nothing currently verified: {cov}");
}

/// An agent cannot sign a person's name, and the MCP surface does not even offer
/// the option: `takomo_verdict` has no `actor_kind`, so a human approval has to
/// come through the REST route with a human-scoped token.
#[tokio::test]
async fn mcp_verdicts_are_always_agent_verdicts() {
    let app = TestApp::spawn().await;
    let (check, _) = app
        .tool(
            &app.worker,
            "takomo_check_file",
            json!({ "project": "tp", "title": "Create a claim",
                    "verification": "agent_then_human" }),
        )
        .await;
    let check_id = check["id"].as_str().unwrap().to_string();
    app.tool(
        &app.worker,
        "takomo_cases_file",
        json!({ "check": check_id, "cases": [{ "key": "only" }] }),
    )
    .await;
    let (wl, _) = app
        .tool(&app.worker, "takomo_worklist", json!({ "project": "tp" }))
        .await;
    let case = wl["agent"]["items"][0]["case"]
        .as_str()
        .unwrap()
        .to_string();

    let (out, _) = app
        .tool(
            &app.worker,
            "takomo_verdict",
            json!({ "case": case, "verdict": "pass" }),
        )
        .await;
    assert_eq!(out["agent"]["verdict"], "pass");
    assert!(
        out["human"]["verdict"].is_null(),
        "MCP never records a human verdict: {out}"
    );

    // Under agent_then_human it now waits for a person, and the worklist says so.
    let (wl, _) = app
        .tool(&app.worker, "takomo_worklist", json!({ "project": "tp" }))
        .await;
    assert_eq!(wl["human"]["cases"], 1, "{wl}");
    assert_eq!(wl["human"]["items"][0]["reason"], "awaiting_human");

    // An `actor_kind` argument does not exist on the tool, so sending one is a
    // schema violation rather than a quiet escalation of authority.
    let (err, is_err) = app
        .tool(
            &app.worker,
            "takomo_verdict",
            json!({ "case": case, "verdict": "pass", "actor_kind": "human" }),
        )
        .await;
    assert!(
        is_err || err["human"]["verdict"].is_null(),
        "an unknown argument must never produce a human verdict: {err}"
    );
}

/// Read tools must not be charged against the write budget, or an agent reading
/// its worklist would spend the allowance it needs to record verdicts.
#[tokio::test]
async fn checklist_read_tools_are_not_write_charged() {
    let app = TestApp::spawn().await;
    for name in [
        "takomo_coverage",
        "takomo_gate",
        "takomo_checks",
        "takomo_releases",
        "takomo_worklist",
    ] {
        assert!(
            takomo::mcp::READ_TOOLS.contains(&name),
            "{name} must be classified as a read tool"
        );
    }
    // And they work on a token with no write scope at all.
    let reader = app.mint("agent:ro", &["read"], None);
    let (out, is_err) = app
        .tool(&reader, "takomo_coverage", json!({ "project": "tp" }))
        .await;
    assert!(!is_err, "a read-only token can read coverage: {out}");
}

/// An agent may propose a cadence, and what it proposes fires nothing.
///
/// This is the security property of the whole feature, so it is pinned at the
/// surface an agent actually uses: the tool succeeds, the schedule is inert, and
/// the response says so in words rather than leaving the agent to infer it from a
/// status string.
#[tokio::test]
async fn an_agent_can_propose_a_schedule_but_it_fires_nothing() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;

    let out = app
        .tool_ok(
            &app.worker,
            "takomo_schedule_new",
            json!({
                "project": "tp",
                "name": "Weekly review",
                "every": "week",
                "on": ["mon"],
                "at": "09:00",
                "tz": "Europe/Berlin",
                "title": "Weekly review — {week}",
                "rationale": "Filed by hand three weeks running."
            }),
        )
        .await;

    assert_eq!(out["schedule"]["status"], "pending");
    assert!(
        out["schedule"]["next_slot"].is_null(),
        "a proposal must carry no next slot — the sweep's partial index is what \
         makes it inert, and that index only sees rows that have one: {out}"
    );
    let note = out["note"]
        .as_str()
        .expect("a note explaining what happened");
    assert!(
        note.contains("NOT active") && note.contains("Do NOT wait"),
        "the note must tell the agent not to poll something a human may never \
         activate: {note}"
    );
    // It still previews what it would do, so a reviewer can judge it.
    assert_eq!(out["schedule"]["upcoming"].as_array().unwrap().len(), 3);
    assert_eq!(out["schedule"]["proposed_by"], "agent:w1");

    // And the sweeper cannot fire it, however overdue it looks.
    let id = out["schedule"]["id"].as_str().unwrap().to_string();
    app.force_schedule_slot(&id, 1_000);
    assert_eq!(
        app.open_store().materialize_due().expect("sweep"),
        0,
        "a pending schedule must not fire"
    );
}

/// The MCP path shares the REST validators rather than reimplementing them, so
/// the two surfaces cannot drift into accepting what the other refuses.
#[tokio::test]
async fn the_schedule_tool_refuses_what_rest_refuses() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.worker, "initialize", init_params()).await;

    // Weekdays on a daily cadence: an error, not an ignored extra, because
    // accepting it would fire seven times a week when one was asked for.
    let (payload, is_error) = app
        .tool(
            &app.worker,
            "takomo_schedule_new",
            json!({
                "project": "tp", "name": "Backup", "every": "day",
                "on": ["mon"], "at": "06:30", "title": "Verify the backup"
            }),
        )
        .await;
    assert!(
        is_error,
        "a day cadence with weekdays must be refused: {payload}"
    );
    assert!(
        payload.to_string().contains("every: week"),
        "and the refusal should name the cadence they probably meant: {payload}"
    );

    // A human's own schedule is born active — the flag governs proposals.
    app.ok_call(&app.human, "initialize", init_params()).await;
    let out = app
        .tool_ok(
            &app.human,
            "takomo_schedule_new",
            json!({
                "project": "tp", "name": "Backup", "every": "day",
                "at": "06:30", "title": "Verify the backup — {date}"
            }),
        )
        .await;
    assert_eq!(out["schedule"]["status"], "active");
    assert!(out["schedule"]["next_slot"].is_string());
}

/// `takomo_schedules` reads, and reads carry the history an agent needs to decide
/// whether recurring work is actually getting done.
#[tokio::test]
async fn the_schedules_read_tool_carries_the_occurrence_history() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.human, "initialize", init_params()).await;
    let created = app
        .tool_ok(
            &app.human,
            "takomo_schedule_new",
            json!({
                "project": "tp", "name": "Weekly review", "every": "week",
                "on": ["mon"], "at": "09:00", "title": "Weekly review — {week}"
            }),
        )
        .await;
    let id = created["schedule"]["id"].as_str().unwrap().to_string();
    app.post(&app.human, &format!("/v1/schedules/{id}/run"), json!({}))
        .await;

    let out = app
        .tool_ok(&app.human, "takomo_schedules", json!({ "project": "tp" }))
        .await;
    let rows = out["schedules"].as_array().expect("schedules");
    assert_eq!(rows.len(), 1, "{out}");
    let occ = rows[0]["occurrences"].as_array().expect("occurrences");
    assert_eq!(occ.len(), 1, "the read tool carries the history: {out}");
    assert_eq!(occ[0]["outcome"], "open");
}

/// `tools/list` is what every session reads before it can call anything, so
/// what rides along in it is charged to the agent's context 49 times over.
/// `schemars` stamps the same JSON Schema dialect URI into every tool, where no
/// client acts on it; `slim_tools` drops it.
#[tokio::test]
async fn tools_list_carries_no_schema_dialect_boilerplate() {
    let app = TestApp::spawn().await;
    let list = app.ok_call(&app.worker, "tools/list", json!({})).await;
    let tools = list["tools"].as_array().expect("tools array");
    assert!(!tools.is_empty(), "no tools advertised");

    for t in tools {
        assert!(
            t["inputSchema"].get("$schema").is_none(),
            "tool {} still advertises the dialect URI",
            t["name"]
        );
        // The schema must still be a usable object — stripping one key must not
        // have flattened what a client validates against.
        assert_eq!(
            t["inputSchema"]["type"], "object",
            "tool {} lost its schema shape",
            t["name"]
        );
    }

    // Tools that take arguments must still describe them: a slimmer payload that
    // dropped `properties` would save tokens by making the surface unusable.
    let ready = tools
        .iter()
        .find(|t| t["name"] == "takomo_ready")
        .expect("takomo_ready listed");
    assert!(
        ready["inputSchema"]["properties"]["project"].is_object(),
        "takomo_ready lost its documented arguments: {ready}"
    );
}

/// The ready queue reports how much it did not return. Without `total`, a full
/// page and a queue that happens to be exactly that long are indistinguishable,
/// and an agent draining work reads a fraction of it as if it were all of it.
#[tokio::test]
async fn mcp_ready_reports_the_whole_queue_not_just_the_page() {
    let app = TestApp::spawn().await;
    for i in 0..5 {
        let created = app
            .tool_ok(
                &app.worker,
                "takomo_new",
                json!({ "project": "tp", "title": format!("ready item {i}") }),
            )
            .await;
        // A new ticket lands in `brief`, which is not claimable; the ready queue
        // only offers work that has been through the approval path.
        let id = created["ticket"]["id"]
            .as_str()
            .unwrap_or_else(|| created["id"].as_str().expect("created id"))
            .to_string();
        app.to_ready(&id).await;
    }

    let full = app
        .tool_ok(&app.worker, "takomo_ready", json!({ "project": "tp" }))
        .await;
    let all = full["items"].as_array().expect("items").len() as i64;
    assert!(all >= 5, "expected the seeded work to be ready: {full}");
    assert_eq!(
        full["total"], all,
        "an unclipped page's total is its length"
    );
    assert!(
        full["note"].is_null(),
        "a complete page must not claim to be partial: {full}"
    );

    // A page smaller than the queue says so, in words and in `total`.
    let page = app
        .tool_ok(
            &app.worker,
            "takomo_ready",
            json!({ "project": "tp", "limit": 2 }),
        )
        .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert_eq!(page["limit"], 2);
    assert_eq!(page["total"], all, "total counts the queue, not the page");
    let note = page["note"]
        .as_str()
        .expect("a clipped page explains itself");
    assert!(
        note.contains(&all.to_string()) && note.contains("limit"),
        "the note should say how many there are and how to get them: {note}"
    );

    // Out-of-range limits are clamped, not refused — the same contract as REST.
    let clamped = app
        .tool_ok(
            &app.worker,
            "takomo_ready",
            json!({ "project": "tp", "limit": 9999 }),
        )
        .await;
    assert_eq!(
        clamped["limit"], 200,
        "limit clamps to the documented ceiling"
    );
}

/// takomo_questions used to pass no limit at all, so it took the store's 500-row
/// cap and reported nothing about it: an agent with more open questions than
/// that read the first 500 and had no way to learn there were more (takomo-5ktp).
#[tokio::test]
async fn mcp_questions_pages_instead_of_silently_capping() {
    let app = TestApp::spawn().await;
    for i in 0..4 {
        let created = app
            .tool_ok(
                &app.worker,
                "takomo_new",
                json!({ "project": "tp", "title": format!("q host {i}") }),
            )
            .await;
        let id = created["ticket"]["id"]
            .as_str()
            .unwrap_or_else(|| created["id"].as_str().expect("created id"))
            .to_string();
        app.tool_ok(
            &app.worker,
            "takomo_ask",
            json!({ "id": id, "mode": "advisory", "kind": "confirm", "title": format!("question {i}") }),
        )
        .await;
    }

    let all = app
        .tool_ok(&app.human, "takomo_questions", json!({ "project": "tp" }))
        .await;
    assert_eq!(all["items"].as_array().unwrap().len(), 4, "{all}");
    assert_eq!(all["total"], 4);
    assert_eq!(all["limit"], 500, "the documented default page");
    assert!(all["next_cursor"].is_null(), "{all}");

    // Paged: total is the queue, the cursor continues it, and the pages are
    // disjoint.
    let first = app
        .tool_ok(
            &app.human,
            "takomo_questions",
            json!({ "project": "tp", "limit": 3 }),
        )
        .await;
    assert_eq!(first["items"].as_array().unwrap().len(), 3);
    assert_eq!(first["total"], 4, "total counts the queue, not the page");
    assert_eq!(first["next_cursor"], 3);
    assert!(
        first["note"]
            .as_str()
            .unwrap_or_default()
            .contains("cursor=3"),
        "the note should hand back the cursor to use: {first}"
    );

    let second = app
        .tool_ok(
            &app.human,
            "takomo_questions",
            json!({ "project": "tp", "limit": 3, "cursor": 3 }),
        )
        .await;
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert!(second["next_cursor"].is_null(), "{second}");

    let ids = |v: &serde_json::Value| -> Vec<String> {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q["id"].as_str().unwrap().to_string())
            .collect()
    };
    let (p1, p2) = (ids(&first), ids(&second));
    assert!(
        p1.iter().all(|i| !p2.contains(i)),
        "pages must not overlap: {p1:?} vs {p2:?}"
    );

    // Clamped rather than refused, matching REST.
    let clamped = app
        .tool_ok(
            &app.human,
            "takomo_questions",
            json!({ "project": "tp", "limit": 99999 }),
        )
        .await;
    assert_eq!(clamped["limit"], 500);
}

/// `takomo_move` over MCP: the same move the REST surface performs, including
/// the subtree default an agent relies on when it names an epic.
#[tokio::test]
async fn hosted_mcp_moves_an_epic_with_its_subtree() {
    let app = TestApp::spawn().await;
    app.ok_call(&app.admin, "initialize", init_params()).await;

    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "beta", "name": "Beta" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");

    let epic = app.create_typed("Billing", "epic", None).await;
    let child = app.create_typed("Invoices", "task", Some(&epic)).await;

    let out = app
        .tool_ok(
            &app.admin,
            "takomo_move",
            json!({ "tickets": [epic], "to_project": "beta" }),
        )
        .await;
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["total"], 2, "the epic and its child: {out}");

    let shown = app
        .tool_ok(&app.worker, "takomo_show", json!({ "id": child }))
        .await;
    assert_eq!(shown["ticket"]["project"], "beta", "{shown}");
    assert_eq!(
        shown["ticket"]["id"],
        child.as_str(),
        "a move never rewrites an id: {shown}"
    );

    // A claimed ticket refuses the whole move here too — the rule lives in the
    // store, not in one surface's handler.
    let (s, out2) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [epic], "to_project": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "moving back is symmetric: {out2}");
    assert_eq!(out2["total"], 2, "{out2}");
}

// The archive gate reaches MCP, because it lives in the store rather than in a
// REST handler: an agent working over `/mcp` gets the same teaching 409, with
// the same code, from the same guard. Worth proving on this surface separately —
// MCP has no HTTP method to classify a write by, so a gate implemented in the
// REST middleware would have left this hole wide open.
#[tokio::test]
async fn archived_project_refuses_mcp_writes_but_not_reads() {
    let app = TestApp::spawn().await;
    let id = app
        .tool_ok(
            &app.worker,
            "takomo_new",
            json!({ "project": "tp", "title": "Filed before the freeze" }),
        )
        .await["ticket"]["id"]
        .as_str()
        .expect("ticket id")
        .to_string();

    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);

    for (tool, args) in [
        (
            "takomo_new",
            json!({ "project": "tp", "title": "after the freeze" }),
        ),
        ("takomo_claim", json!({ "id": id })),
        (
            "takomo_comment",
            json!({ "id": id, "body": "anyone there?" }),
        ),
        ("takomo_start", json!({ "id": id })),
    ] {
        let (out, is_error) = app.tool(&app.worker, tool, args).await;
        assert!(is_error, "{tool} must be refused: {out}");
        assert_eq!(out["code"], "project.archived", "{tool}: {out}");
    }

    // Reading is untouched on this surface too.
    let shown = app
        .tool_ok(&app.worker, "takomo_show", json!({ "id": id }))
        .await;
    assert_eq!(shown["ticket"]["state"], "brief", "{shown}");
    let projects = app.tool_ok(&app.worker, "takomo_projects", json!({})).await;
    let tp = projects["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "tp")
        .expect("tp is listed");
    assert_eq!(
        tp["archived"], true,
        "the freeze is visible to agents: {tp}"
    );
}

/// The append tool's own description is what makes agents write range anchors,
/// and it is the only thing that does — nothing validates `meta`, so an agent
/// that is not told about `quote`/`prefix`/`suffix` keeps anchoring notes to a
/// paragraph number that a later revision silently invalidates.
///
/// This exists because that drift already happened once: the document model
/// grew range anchors and folders, `docs/initiatives.md` was updated, and the
/// prompt that actually drives the behaviour was not. A doc nobody reads at
/// runtime cannot enforce a convention; this test can.
#[tokio::test]
async fn the_initiative_tools_teach_the_document_model_they_expect() {
    let app = TestApp::spawn().await;
    let list = app.ok_call(&app.worker, "tools/list", json!({})).await;
    let tools = list["tools"].as_array().expect("tools array");

    let describe = |name: &str| -> String {
        tools
            .iter()
            .find(|t| t["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("tools/list missing {name}"))
            .to_string()
    };

    // `quote` alone does not discriminate — the description has always mentioned
    // "a customer quote" for `meta.origin`. It is `prefix`/`suffix`/`orphaned`
    // that are only there if the anchor is really being taught, so do not thin
    // this list down to the obvious word.
    let append = describe("takomo_initiative_append");
    for token in ["quote", "prefix", "suffix", "orphaned"] {
        assert!(
            append.contains(token),
            "takomo_initiative_append must teach the range anchor: missing '{token}'"
        );
    }
    assert!(
        append.contains("pane") && append.contains("cites") && append.contains("proposed"),
        "takomo_initiative_append must still teach panes, citations and amendments"
    );

    // Folders are the other thing an agent cannot discover by inspecting a
    // schema: `metadata` is a free-form object, so only prose names the key.
    for name in ["takomo_initiative_new", "takomo_initiative_update"] {
        let tool = describe(name);
        assert!(
            tool.contains("path"),
            "{name} must document metadata.path — the folder a document is filed in"
        );
    }
}

// ---- MCP parity fixes (takomo-u3hd / w5zk / rsil / qr3t / knen) ------------

/// `takomo_start` must not leave a claim behind when the transition it pairs
/// with is refused — the compound verb is one store transaction.
#[tokio::test]
async fn mcp_start_rolls_back_a_claim_when_transition_fails() {
    let app = TestApp::spawn().await;
    let id = claimable(&app, "start rollback").await;

    let (payload, is_error) = app
        .tool(
            &app.worker,
            "takomo_start",
            json!({ "id": id, "to": "done" }),
        )
        .await;
    assert!(is_error, "ready -> done must be refused: {payload}");
    assert!(
        payload["allowed_transitions"].is_array(),
        "illegal start relays allowed_transitions: {payload}"
    );

    let status = app
        .tool_ok(&app.worker, "takomo_claim_status", json!({ "id": id }))
        .await;
    assert!(
        status["holder"].is_null(),
        "a failed start must not strand a claim: {status}"
    );
}

/// `takomo_block` must not leave a comment behind when the block transition fails.
#[tokio::test]
async fn mcp_block_rolls_back_a_comment_when_transition_fails() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("block rollback").await;

    let (payload, is_error) = app
        .tool(
            &app.worker,
            "takomo_block",
            json!({ "id": id, "comment": "orphan?" }),
        )
        .await;
    assert!(is_error, "brief -> blocked must be refused: {payload}");

    let (_, show) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let comments = show["comments"].as_array().map(|a| a.len()).unwrap_or(0);
    assert_eq!(comments, 0, "a failed block must not leave a comment");
}

/// Caller-supplied idempotency keys replay the original create instead of
/// double-creating on a retried MCP frame.
#[tokio::test]
async fn mcp_new_honours_an_idempotency_key() {
    let app = TestApp::spawn().await;
    let key = "mcp-idem-parity";
    let args = json!({
        "project": "tp",
        "title": "idem ticket",
        "idempotency_key": key,
    });
    let first = app.tool_ok(&app.worker, "takomo_new", args.clone()).await;
    let second = app.tool_ok(&app.worker, "takomo_new", args).await;
    assert_eq!(
        first["ticket"]["id"], second["ticket"]["id"],
        "the same key must replay the original ticket"
    );
}

/// MCP create accepts metadata and blocked_by like REST.
#[tokio::test]
async fn mcp_new_carries_metadata_and_blocked_by() {
    let app = TestApp::spawn().await;
    let blocker = app.create_ticket("blocker for mcp new").await;
    app.drive_to_done(&blocker).await;

    let created = app
        .tool_ok(
            &app.worker,
            "takomo_new",
            json!({
                "project": "tp",
                "title": "blocked at birth",
                "blocked_by": [blocker],
                "metadata": { "source": "mcp-test" },
            }),
        )
        .await;
    let id = created["ticket"]["id"].as_str().unwrap();
    let show = app
        .tool_ok(&app.worker, "takomo_show", json!({ "id": id }))
        .await;
    assert_eq!(show["ticket"]["metadata"]["source"], "mcp-test");
    let deps = app
        .tool_ok(
            &app.worker,
            "takomo_deps",
            json!({ "id": id, "direction": "blocked_by" }),
        )
        .await;
    let blocked_by: Vec<&str> = deps["deps"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["blocked_by"].as_str().unwrap())
        .collect();
    assert!(blocked_by.contains(&blocker.as_str()));
}

/// `takomo_claim` forwards ttl_seconds to the store lease.
#[tokio::test]
async fn mcp_claim_forwards_ttl_seconds() {
    let app = TestApp::spawn().await;
    let id = claimable(&app, "ttl on claim").await;
    let before = takomo::ids::now_ms();
    let claim = app
        .tool_ok(
            &app.worker,
            "takomo_claim",
            json!({ "id": id, "ttl_seconds": 42 }),
        )
        .await;
    let expires = claim["lease"]["expires_at"].as_str().expect("expires_at");
    let expires_ms = chrono::DateTime::parse_from_rfc3339(expires)
        .expect("rfc3339")
        .timestamp_millis();
    let delta = (expires_ms - before) / 1000;
    assert!(
        (38..=46).contains(&delta),
        "42s TTL expected, got {delta}s from claim {claim}"
    );
}

/// MCP ask accepts any JSON `recommended` on kinds without options (clarify).
#[tokio::test]
async fn mcp_ask_accepts_recommended_as_json() {
    let app = TestApp::spawn().await;
    let ticket = app.create_ticket("json rec ticket").await;
    app.to_implementing(&ticket).await;
    let asked = app
        .tool_ok(
            &app.worker,
            "takomo_ask",
            json!({
                "id": ticket,
                "kind": "clarify",
                "mode": "advisory",
                "title": "how to word it?",
                "recommended": { "text": "store UTC, render local" },
            }),
        )
        .await;
    assert_eq!(
        asked["question"]["recommended"],
        json!({ "text": "store UTC, render local" })
    );
}

/// Tool-mapping smoke tests for MCP tools that had no dedicated coverage.
#[tokio::test]
async fn mcp_untested_tools_reach_the_store_and_relay_errors() {
    let app = TestApp::spawn().await;

    // projects — lists seeded demo project.
    let projects = app.tool_ok(&app.worker, "takomo_projects", json!({})).await;
    assert!(
        projects["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "tp"),
        "takomo_projects should list tp: {projects}"
    );

    // comment — writes a comment row.
    let id = app.create_ticket("mcp comment tool").await;
    let commented = app
        .tool_ok(
            &app.worker,
            "takomo_comment",
            json!({ "id": id, "body": "via mcp" }),
        )
        .await;
    assert_eq!(commented["comment"]["body"], "via mcp");

    // dep + deps — add and read a dependency edge.
    let blocker = app.create_ticket("mcp dep blocker").await;
    app.drive_to_done(&blocker).await;
    let blocked = app.create_ticket("mcp dep blocked").await;
    app.tool_ok(
        &app.worker,
        "takomo_dep",
        json!({ "id": blocked, "blocked_by": blocker }),
    )
    .await;
    let graph = app
        .tool_ok(
            &app.worker,
            "takomo_deps",
            json!({ "id": blocked, "direction": "both" }),
        )
        .await;
    assert!(graph["deps"]["edges"].is_array());

    // promote — records a promotion on a done ticket.
    let done_id = app.create_ticket("mcp promote").await;
    app.drive_to_done(&done_id).await;
    let promo = app
        .tool_ok(
            &app.worker,
            "takomo_promote",
            json!({ "id": done_id, "target": "tp", "note": "shipped" }),
        )
        .await;
    assert_eq!(promo["promotion"]["target"], "tp");

    // archive — sets archived_at.
    let arch = app
        .tool_ok(&app.worker, "takomo_archive", json!({ "id": done_id }))
        .await;
    assert!(arch["ticket"]["archived_at"].is_string());

    // roadmap — project rollup (read mapping).
    let roadmap = app
        .tool_ok(&app.worker, "takomo_roadmap", json!({ "project": "tp" }))
        .await;
    assert!(roadmap["roadmap"].is_object());

    // questions / withdraw / reopen / reply / options — question thread tools.
    let (work, qid) = mcp_parked_question(
        &app,
        "mcp question tools",
        json!({
            "kind": "choose",
            "title": "which?",
            "options": ["x", "y"],
        }),
    )
    .await;
    let listed = app
        .tool_ok(
            &app.human,
            "takomo_questions",
            json!({ "project": "tp", "ticket": work }),
        )
        .await;
    assert!(
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["id"] == qid),
        "takomo_questions should list the open question"
    );

    // Human bounces back for research; agent replies.
    let (bounce_status, _) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/followup"),
            json!({ "message": "need more detail" }),
        )
        .await;
    assert_eq!(bounce_status, StatusCode::OK);
    let replied = app
        .tool_ok(
            &app.worker,
            "takomo_reply",
            json!({ "id": qid, "message": "here is the detail" }),
        )
        .await;
    assert_eq!(replied["question"]["awaiting"], "human");

    let revised = app
        .tool_ok(
            &app.worker,
            "takomo_options",
            json!({
                "id": qid,
                "options": ["x2", "y2"],
                "recommended": "x2",
                "reason": "research showed better choices",
            }),
        )
        .await;
    assert_eq!(revised["question"]["options"], json!(["x2", "y2"]));

    app.tool_ok(
        &app.human,
        "takomo_answer",
        json!({ "id": qid, "answer": "x2" }),
    )
    .await;

    let (reopen_payload, reopen_err) = app
        .tool(&app.worker, "takomo_reopen", json!({ "id": qid }))
        .await;
    assert!(
        reopen_err,
        "worker lacks human scope for reopen: {reopen_payload}"
    );
    let reopened = app
        .tool_ok(&app.human, "takomo_reopen", json!({ "id": qid }))
        .await;
    assert_eq!(reopened["question"]["status"], "open");

    // withdraw on a fresh question.
    let (_work2, qid2) = mcp_parked_question(
        &app,
        "mcp withdraw",
        json!({
            "kind": "confirm",
            "title": "still needed?",
        }),
    )
    .await;
    let withdrawn = app
        .tool_ok(
            &app.worker,
            "takomo_withdraw",
            json!({ "id": qid2, "reason": "figured it out" }),
        )
        .await;
    assert_eq!(withdrawn["question"]["status"], "withdrawn");

    // answer_link minting is covered elsewhere; still assert the tool name resolves.
    let list = app.ok_call(&app.worker, "tools/list", json!({})).await;
    let names: Vec<&str> = list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for tool in [
        "takomo_answer_link",
        "takomo_archive",
        "takomo_comment",
        "takomo_dep",
        "takomo_deps",
        "takomo_link",
        "takomo_options",
        "takomo_projects",
        "takomo_promote",
        "takomo_questions",
        "takomo_reopen",
        "takomo_reply",
        "takomo_roadmap",
        "takomo_withdraw",
    ] {
        assert!(names.contains(&tool), "tools/list missing {tool}");
    }
}
