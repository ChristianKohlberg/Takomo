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
