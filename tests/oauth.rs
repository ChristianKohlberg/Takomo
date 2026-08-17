//! Integration tests for the OAuth 2.1 authorization server (`/oauth/*` and the
//! two `.well-known` discovery documents).
//!
//! These drive the real server over HTTP, and specifically over the parts a
//! hosted MCP client (claude.ai, ChatGPT, the Gemini app) actually exercises:
//! unauthenticated discovery, dynamic registration, a browser-shaped consent
//! POST, and a form-encoded token exchange. The redirects are *not* followed —
//! seeing the `Location` header is the point, and the registered callback
//! (`https://client.example/callback`) does not exist.
//!
//! What is deliberately covered beyond the happy path: the refusals that would
//! otherwise be silent security holes (an unregistered redirect target, a replayed
//! code, a reused refresh token, a hostile client name reaching the consent page)
//! and the one privilege rule this feature adds — `admin` is never granted through
//! consent, whatever token approves it.

use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use takomo::api::oauth::OauthConfig;
use takomo::ids::pkce_s256_challenge;

mod common;
use common::TestApp;

/// A PKCE verifier: 46 characters from the unreserved set (RFC 7636 requires
/// 43-128).
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyz0123456789-._~ABCDEF";
const REDIRECT: &str = "https://client.example/callback";
const CLIENT_STATE: &str = "opaque-client-state-42";

/// A client that does not follow redirects, so a 302 back to the (nonexistent)
/// callback is observable rather than a connection error.
fn no_redirect() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build client")
}

/// Every hidden field of a rendered consent page, in order.
///
/// The point of reading them out of the HTML rather than composing a POST by hand:
/// what the form carries back is *part of the request*, so a test that invents the
/// body cannot see a field the page emits wrongly (or emits at all when the client
/// sent nothing). Both are how an omitted `scope` came to mean `read` alone.
fn hidden_fields(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for chunk in html.split(r#"<input type="hidden" name=""#).skip(1) {
        let Some((name, rest)) = chunk.split_once(r#"" value=""#) else {
            continue;
        };
        let Some((value, _)) = rest.split_once(r#"">"#) else {
            continue;
        };
        out.push((name.to_string(), value.replace("&amp;", "&")));
    }
    out
}

/// One query parameter out of a URL, without pulling in a URL parser.
fn query_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| {
            v.replace("%2D", "-")
                .replace("%2E", ".")
                .replace("%20", " ")
        })
    })
}

impl TestApp {
    /// POST a form-urlencoded body without following redirects.
    async fn form(&self, path: &str, fields: &[(&str, &str)]) -> reqwest::Response {
        no_redirect()
            .post(self.url(path))
            .form(&fields.iter().collect::<Vec<_>>())
            .send()
            .await
            .expect("form post")
    }

    /// Register a public client and return its `client_id`.
    async fn register_client(&self, name: &str, uris: &[&str]) -> String {
        let (status, body) = self
            .json(
                self.request(Method::POST, "/oauth/register")
                    .json(&json!({ "client_name": name, "redirect_uris": uris })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "registration failed: {body}");
        body["client_id"].as_str().expect("client_id").to_string()
    }

    /// The authorization URL for a fresh flow.
    fn authorize_url(&self, client_id: &str) -> String {
        format!(
            "/oauth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=https%3A%2F%2Fclient.example%2Fcallback\
             &code_challenge={}&code_challenge_method=S256&state={CLIENT_STATE}&scope=read+write",
            pkce_s256_challenge(VERIFIER)
        )
    }

    /// Approve a consent request with `token`, returning the raw response.
    async fn consent(&self, client_id: &str, token: &str, scopes: &[&str]) -> reqwest::Response {
        let challenge = pkce_s256_challenge(VERIFIER);
        let mut fields = vec![
            ("client_id", client_id),
            ("redirect_uri", REDIRECT),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("response_type", "code"),
            ("state", CLIENT_STATE),
            ("scope", "read write"),
            ("token", token),
            ("action", "approve"),
        ];
        for scope in scopes {
            fields.push(("grant_scope", scope));
        }
        self.form("/oauth/authorize", &fields).await
    }

    /// GET a consent page and return its HTML.
    async fn consent_html(&self, path: &str) -> String {
        let resp = no_redirect()
            .get(self.url(path))
            .send()
            .await
            .expect("request");
        assert_eq!(resp.status(), StatusCode::OK, "consent page should render");
        resp.text().await.unwrap_or_default()
    }

    /// Submit a rendered consent page, carrying back exactly the hidden fields it
    /// emitted — the browser's behaviour, and the only way a test sees a field the
    /// page should not have emitted at all.
    async fn submit_consent(&self, page: &str, extra: &[(&str, &str)]) -> reqwest::Response {
        let hidden = hidden_fields(page);
        let mut fields: Vec<(&str, &str)> = hidden
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        fields.extend_from_slice(extra);
        self.form("/oauth/authorize", &fields).await
    }

    /// Run consent to completion and return the authorization code.
    async fn authorization_code(&self, client_id: &str, token: &str) -> String {
        let resp = self.consent(client_id, token, &["read", "write"]).await;
        assert_eq!(resp.status(), StatusCode::FOUND, "consent should redirect");
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .expect("Location header")
            .to_string();
        assert_eq!(
            query_param(&location, "state").as_deref(),
            Some(CLIENT_STATE),
            "the client's state must come back untouched: {location}"
        );
        query_param(&location, "code").expect("code in redirect")
    }

    /// POST the token endpoint and return (status, body).
    async fn token_call(&self, fields: &[(&str, &str)]) -> (StatusCode, Value) {
        let resp = self.form("/oauth/token", fields).await;
        let status = resp.status();
        let body = resp.json::<Value>().await.unwrap_or(Value::Null);
        (status, body)
    }

    /// The full flow: register, consent as `token`, exchange. Returns the token
    /// endpoint's response body.
    async fn full_flow(&self, token: &str) -> Value {
        let client_id = self.register_client("Test Client", &[REDIRECT]).await;
        self.full_flow_with(&client_id, token).await
    }

    /// [`TestApp::full_flow`] for a client already registered, when the test cares
    /// which client it is.
    async fn full_flow_with(&self, client_id: &str, token: &str) -> Value {
        let code = self.authorization_code(client_id, token).await;
        let (status, body) = self
            .token_call(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("client_id", client_id),
                ("redirect_uri", REDIRECT),
                ("code_verifier", VERIFIER),
            ])
            .await;
        assert_eq!(status, StatusCode::OK, "code exchange failed: {body}");
        body
    }

    /// The id `takomo token list` would show for this token — the handle an operator
    /// revokes by.
    async fn token_id_of(&self, token: &str) -> String {
        let (status, who) = self.get(token, "/v1/whoami").await;
        assert_eq!(status, StatusCode::OK, "whoami failed: {who}");
        who["token_id"].as_str().expect("token_id").to_string()
    }

    /// Is `token` accepted by the MCP surface? Uses `initialize`, the one frame a
    /// client sends before anything else, and which the middleware guards.
    async fn mcp_accepts(&self, token: &str) -> bool {
        let resp = self
            .request(Method::POST, "/mcp")
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "probe", "version": "0" }
                }
            }))
            .send()
            .await
            .expect("mcp request");
        resp.status() == StatusCode::OK
    }
}

// ---------------------------------------------------------------------------
// Discovery

/// The two documents a client fetches before it has any credential. They must be
/// reachable **without** one — a 401 here is the dead end that makes a hosted
/// client impossible to connect, which is exactly what this instance returned
/// before these routes existed.
#[tokio::test]
async fn discovery_is_unauthenticated_and_self_describing() {
    let app = TestApp::spawn_with_oauth().await;

    let resp = app
        .request(Method::GET, "/.well-known/oauth-protected-resource")
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK);
    let prm: Value = resp.json().await.expect("json");
    assert_eq!(
        prm["resource"], format!("{}/mcp", app.base),
        "the resource identifier must be the MCP endpoint, byte-identical to what a user types into a connector dialog"
    );
    assert_eq!(prm["authorization_servers"][0], app.base);
    assert_eq!(prm["bearer_methods_supported"][0], "header");

    let resp = app
        .request(Method::GET, "/.well-known/oauth-authorization-server")
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK);
    let asm: Value = resp.json().await.expect("json");
    assert_eq!(asm["issuer"], app.base);
    assert_eq!(
        asm["authorization_endpoint"],
        format!("{}/oauth/authorize", app.base)
    );
    assert_eq!(asm["token_endpoint"], format!("{}/oauth/token", app.base));
    assert_eq!(
        asm["registration_endpoint"],
        format!("{}/oauth/register", app.base)
    );
    // The MCP authorization spec requires S256 to be advertised so a client can
    // verify support before starting a flow; `plain` must not appear.
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(
        asm["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );
    // Claude selects CIMD only if this is advertised. takomo does not implement
    // it, so its absence is what makes a client fall back to registration.
    assert!(
        asm.get("client_id_metadata_document_supported").is_none(),
        "CIMD must not be advertised while it is not implemented: {asm}"
    );

    // `admin` is not on offer, on either document.
    for doc in [&prm, &asm] {
        let scopes = doc["scopes_supported"]
            .as_array()
            .expect("scopes_supported");
        assert!(
            !scopes.iter().any(|s| s == "admin"),
            "admin must never be advertised as grantable: {doc}"
        );
    }
}

/// The suffixed probe path (RFC 9728 §3.1), which a client tries when the
/// protected resource lives at a path rather than at the origin.
#[tokio::test]
async fn protected_resource_metadata_answers_the_suffixed_probe_path() {
    let app = TestApp::spawn_with_oauth().await;
    let resp = app
        .request(Method::GET, "/.well-known/oauth-protected-resource/mcp")
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["resource"], format!("{}/mcp", app.base));
}

/// The `401` from `/mcp` is where discovery starts. Without this header a client
/// has nothing to follow and reports an unexplained failure.
#[tokio::test]
async fn mcp_401_carries_the_resource_metadata_challenge() {
    let app = TestApp::spawn_with_oauth().await;
    let resp = app
        .request(Method::POST, "/mcp")
        .header("Content-Type", "application/json")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let challenge = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        challenge,
        format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
            app.base
        )
    );
}

/// With no public URL configured there is no issuer to advertise, so the header
/// must not appear — pointing a client at a flow this server cannot run is worse
/// than not mentioning it.
#[tokio::test]
async fn without_a_public_url_there_is_no_challenge_and_no_authorization_server() {
    let app = TestApp::spawn().await;
    let resp = app
        .request(Method::POST, "/mcp")
        .header("Content-Type", "application/json")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(
        resp.headers().get("www-authenticate").is_none(),
        "no issuer means no challenge"
    );

    // And the endpoints say so, in words aimed at the operator who has to fix it.
    let resp = app
        .request(Method::GET, "/.well-known/oauth-protected-resource")
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"], "temporarily_unavailable");
    assert!(
        body["remedy"]
            .as_str()
            .unwrap_or("")
            .contains("TAKOMO_PUBLIC_URL"),
        "the remedy must name the variable to set: {body}"
    );
    // OAuth being off has two causes and this handler cannot tell them apart:
    // unset, or set to something unusable as an issuer — and since takomo-z919 the
    // latter keeps serving instead of stopping the server, so it is a state a live
    // instance is genuinely in. Claiming "not set" would send an operator whose
    // value IS set looking in the wrong place, so the description must own the
    // ambiguity and point at the startup line, which is what knows.
    // Asserted on meaning rather than wording, which is why "either" carries it:
    // the sentence must present two possibilities, not commit to one.
    let description = body["error_description"].as_str().unwrap_or("");
    assert!(
        description.contains("either")
            && description.contains("unset")
            && description.contains("usable"),
        "the description must cover both causes rather than asserting one: {body}"
    );
    assert!(
        description.contains("startup"),
        "…and point at the one place that can tell them apart: {body}"
    );
}

// ---------------------------------------------------------------------------
// Dynamic client registration

/// The fields that constrain later security decisions are validated strictly; the
/// rest of RFC 7591's metadata is ignored rather than refused, because real
/// clients send plenty of it.
#[tokio::test]
async fn registration_validates_redirect_uris_and_ignores_unknown_metadata() {
    let app = TestApp::spawn_with_oauth().await;

    let cases: [(Value, &str); 5] = [
        (json!({}), "no redirect_uris at all"),
        (
            json!({ "redirect_uris": ["/callback"] }),
            "a relative reference",
        ),
        (
            json!({ "redirect_uris": ["http://evil.example/cb"] }),
            "plain http on a non-loopback host",
        ),
        (
            json!({ "redirect_uris": ["https://client.example/cb#frag"] }),
            "a fragment, which RFC 6749 §3.1.2 forbids",
        ),
        (
            json!({ "redirect_uris": ["https://a.example/1", "https://a.example/2",
                                      "https://a.example/3", "https://a.example/4",
                                      "https://a.example/5", "https://a.example/6"] }),
            "more redirect URIs than the ceiling allows",
        ),
    ];
    for (body, why) in cases {
        let (status, out) = app
            .json(app.request(Method::POST, "/oauth/register").json(&body))
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "registration with {why} must be refused: {out}"
        );
        assert_eq!(out["error"], "invalid_redirect_uri", "for {why}: {out}");
    }

    // A loopback http URI is accepted: RFC 8252 native clients (Claude Code among
    // them) redirect to one on an ephemeral port.
    let (status, _) = app
        .json(
            app.request(Method::POST, "/oauth/register")
                .json(&json!({ "redirect_uris": ["http://127.0.0.1:3118/callback"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Unrecognized metadata members must not be rejected (RFC 7591 §2) — this is
    // the one handler in the codebase that deliberately does not `reject_unknown`.
    let (status, body) = app
        .json(app.request(Method::POST, "/oauth/register").json(&json!({
            "redirect_uris": [REDIRECT],
            "client_name": "Some Product",
            "client_uri": "https://client.example",
            "logo_uri": "https://client.example/logo.png",
            "software_id": "abc-123",
            "contacts": ["ops@client.example"],
            "scope": "read write",
        })))
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "unknown metadata must be ignored, not refused: {body}"
    );
    assert_eq!(body["token_endpoint_auth_method"], "none");
    assert!(body.get("client_secret").is_none(), "public clients only");
}

/// The global registration budget refuses with a number to wait, not just prose: the
/// caller here is a connector with no human watching it, so `Retry-After` is what
/// makes the difference between one retry and a tight loop.
#[tokio::test]
async fn the_registration_budget_says_how_long_to_wait() {
    let app = TestApp::spawn_with_oauth().await;
    let mut refused = None;
    // The window is global — there is no caller identity to key one by — so it is
    // reached the same way a script pointed at the endpoint would reach it.
    for _ in 0..40 {
        let resp = app
            .request(Method::POST, "/oauth/register")
            .json(&json!({ "redirect_uris": [REDIRECT] }))
            .send()
            .await
            .expect("request");
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            refused = Some(resp);
            break;
        }
    }
    let resp = refused.expect("the global registration budget must be reachable");
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .expect("every 429 in this codebase carries Retry-After");
    let secs: i64 = retry_after.parse().expect("Retry-After is whole seconds");
    assert!(
        (1..=60).contains(&secs),
        "Retry-After must fall inside the one-minute window: {secs}"
    );
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"], "temporarily_unavailable");
    assert!(
        body["remedy"].as_str().unwrap_or("").contains(&retry_after),
        "the remedy should name the same wait as the header: {body}"
    );
}

/// Asking to be a confidential client is refused rather than silently downgraded:
/// a client that believes it authenticates with a secret, and does not, has a
/// security model that is wrong in a way it cannot detect.
#[tokio::test]
async fn registration_refuses_a_confidential_client() {
    let app = TestApp::spawn_with_oauth().await;
    let (status, body) = app
        .json(app.request(Method::POST, "/oauth/register").json(&json!({
            "redirect_uris": [REDIRECT],
            "token_endpoint_auth_method": "client_secret_post",
        })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_client_metadata");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap_or("")
            .contains("PKCE"),
        "the description should explain what replaces the secret: {body}"
    );
}

/// A `client_name` that can forge a line of text is refused. Registration is
/// unauthenticated and this value is rendered — on the consent page, and in the
/// terminal listing an operator reads to decide which connection to revoke, where an
/// escape sequence can erase the row above it.
#[tokio::test]
async fn registration_refuses_a_client_name_that_can_forge_a_display() {
    let app = TestApp::spawn_with_oauth().await;
    for (name, why) in [
        (
            "Claude\nchatgpt",
            "a newline, which splits one row into two",
        ),
        ("Claude\rchatgpt", "a carriage return"),
        ("Claude\x1b[2K\x1b[1A", "an ANSI escape that erases a line"),
        ("Claude\u{202e}", "a bidirectional override"),
    ] {
        let (status, body) = app
            .json(
                app.request(Method::POST, "/oauth/register")
                    .json(&json!({ "redirect_uris": [REDIRECT], "client_name": name })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a name carrying {why} must be refused: {body}"
        );
        assert_eq!(
            body["error"], "invalid_client_metadata",
            "for {why}: {body}"
        );
    }

    // The ordinary case is untouched — including punctuation and non-ASCII, which
    // are not the problem.
    let (status, body) = app
        .json(
            app.request(Method::POST, "/oauth/register").json(
                &json!({ "redirect_uris": [REDIRECT], "client_name": "Claude – Ökobüro (v2)" }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["client_name"], "Claude – Ökobüro (v2)");
}

/// …and a name already stored before that check existed still lists safely. The
/// validation and this filter are deliberately both present: the boundary keeps such
/// bytes out, the sink keeps an old row — or a future writer of that column — from
/// reaching a terminal.
#[tokio::test]
async fn a_hostile_stored_client_name_is_never_rendered_raw() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Claude", &[REDIRECT]).await;
    let issued = app.full_flow_with(&client_id, &app.human).await;
    let derived_id = app
        .token_id_of(issued["access_token"].as_str().unwrap())
        .await;

    // Straight into the table, bypassing the endpoint that now refuses this.
    let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
    conn.execute(
        "UPDATE oauth_clients SET client_name = ?2 WHERE client_id = ?1",
        rusqlite::params![client_id, "Claude\u{1b}[2K\u{1b}[1A\nfake row"],
    )
    .expect("plant a hostile name");
    drop(conn);

    let (_, list) = app.get(&app.admin, "/v1/tokens").await;
    let derived = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == derived_id)
        .expect("listed")
        .clone();
    let label = derived["oauth_client"]["label"].as_str().expect("label");
    assert!(
        !label.chars().any(|c| c.is_control()),
        "the display label must carry no control characters: {label:?}"
    );
    assert_eq!(label, "Claude?[2K?[1A?fake row");

    // Same answer through the store, which is what `takomo token list` prints.
    let listed = app.open_store().list_tokens().expect("list tokens");
    let label = listed
        .iter()
        .find(|t| t.id == derived_id)
        .and_then(|t| t.oauth_client.as_ref())
        .expect("connection")
        .label();
    assert!(
        !label.chars().any(|c| c.is_control()),
        "the CLI must not print raw escapes: {label:?}"
    );
}

// ---------------------------------------------------------------------------
// The authorization code flow, end to end

/// The whole point: a token obtained purely over OAuth works on `/mcp`.
#[tokio::test]
async fn the_code_flow_issues_a_token_that_works_on_mcp() {
    let app = TestApp::spawn_with_oauth().await;
    let issued = app.full_flow(&app.human).await;

    assert_eq!(issued["token_type"], "Bearer");
    assert_eq!(issued["expires_in"], 3600);
    let access = issued["access_token"].as_str().expect("access_token");
    let refresh = issued["refresh_token"].as_str().expect("refresh_token");
    assert!(
        access.starts_with("tk_"),
        "an access token is an ordinary takomo token: {access}"
    );
    assert!(refresh.starts_with("tkr_"));
    // Only what was checked on the consent screen, which is narrower than the
    // `human` token's own scopes — RFC 6749 §5.1 requires echoing it when it
    // differs from the request.
    assert_eq!(issued["scope"], "read write");

    assert!(
        app.mcp_accepts(access).await,
        "the issued token must reach /mcp"
    );

    // It is an ordinary token row, so the existing surfaces see it: an operator
    // can find and revoke exactly this connection.
    let (status, who) = app.get(access, "/v1/whoami").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        who["actor"], "human:reviewer",
        "the actor is the consenting human's"
    );
    let mut scopes: Vec<&str> = who["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    scopes.sort();
    assert_eq!(scopes, vec!["read", "write"]);

    let (status, list) = app.get(&app.admin, "/v1/tokens").await;
    assert_eq!(status, StatusCode::OK);
    let issued_row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == who["token_id"])
        .expect("the OAuth-issued token appears in the token list");
    assert!(
        issued_row["expires_at"].is_string(),
        "an OAuth-issued token always expires, unlike a hand-minted one: {issued_row}"
    );
}

/// The privilege rule this feature adds. Consent narrows; it never widens — and
/// the token most operators have to hand is an admin one.
#[tokio::test]
async fn consent_never_grants_admin_even_from_an_admin_token() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Greedy Client", &[REDIRECT]).await;

    // Ask for admin explicitly, and check it too — neither is honoured.
    let challenge = pkce_s256_challenge(VERIFIER);
    let resp = app
        .form(
            "/oauth/authorize",
            &[
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
                ("response_type", "code"),
                ("state", CLIENT_STATE),
                ("scope", "read write human admin"),
                ("grant_scope", "read"),
                ("grant_scope", "write"),
                ("grant_scope", "human"),
                ("grant_scope", "admin"),
                ("token", &app.admin),
                ("action", "approve"),
            ],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let location = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let code = query_param(&location, "code").expect("code");

    let (status, body) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "exchange failed: {body}");
    let access = body["access_token"].as_str().unwrap();
    assert!(
        !body["scope"].as_str().unwrap().contains("admin"),
        "admin must not appear in the granted scope: {body}"
    );

    let (status, who) = app.get(access, "/v1/whoami").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !who["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "admin"),
        "the issued token must not carry admin: {who}"
    );
    // And it is refused where admin is actually required.
    let (status, err) = app
        .post(
            access,
            "/v1/projects",
            json!({ "id": "sneaky", "name": "Sneaky" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "admin-only route must refuse: {err}"
    );
    assert_eq!(err["code"], "auth.scope");
}

/// The consenting token's project allowlist is inherited, so an OAuth connection
/// cannot reach further than the human who approved it.
#[tokio::test]
async fn the_issued_token_inherits_the_project_allowlist() {
    let app = TestApp::spawn_with_oauth().await;
    app.create_project_with("other", common::simple_workflow())
        .await;
    let scoped = app.mint("human:scoped", &["read", "write"], Some(&["tp"]));

    let issued = app.full_flow(&scoped).await;
    let access = issued["access_token"].as_str().unwrap();

    let (status, _) = app
        .post(
            access,
            "/v1/tickets",
            json!({ "project": "tp", "title": "In scope" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, err) = app
        .post(
            access,
            "/v1/tickets",
            json!({ "project": "other", "title": "Out of scope" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "allowlist must be inherited: {err}"
    );
    assert_eq!(err["code"], "auth.project");
}

/// A flow that names no `scope` is asking to act as the human who approves it, so
/// what they leave checked is what the client gets.
///
/// Driven through the page's own hidden fields, because the bug this covers was in
/// the round trip rather than in either end of it: the form posted back `scope=`
/// where the client had sent nothing, an empty value parsed as "asked for no
/// scopes", and that narrowed to `read` alone — silently, after the human had
/// checked everything on offer.
#[tokio::test]
async fn an_omitted_scope_offers_everything_and_grants_what_stays_checked() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Scopeless Client", &[REDIRECT]).await;

    let page = app
        .consent_html(&format!(
            "/oauth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=https%3A%2F%2Fclient.example%2Fcallback\
             &code_challenge={}&code_challenge_method=S256",
            pkce_s256_challenge(VERIFIER)
        ))
        .await;
    for scope in ["read", "write", "human"] {
        assert!(
            page.contains(&format!(r#"name="grant_scope" value="{scope}" checked"#)),
            "an omitted scope must offer {scope}, pre-checked: {page}"
        );
    }
    // `offline_access` is stated, not offered: it grants nothing, the refresh token
    // is issued either way, and the only thing a checkbox could do here is make the
    // connection die within the hour.
    assert!(
        !page.contains(r#"value="offline_access""#),
        "offline_access must not be a checkbox: {page}"
    );
    assert!(
        page.contains("always issues a refresh token"),
        "…and the page must say what happens instead: {page}"
    );
    // What the client did not send must not travel back as an empty value.
    let carried: Vec<String> = hidden_fields(&page).into_iter().map(|(k, _)| k).collect();
    for absent in ["scope", "state", "resource"] {
        assert!(
            !carried.iter().any(|k| k == absent),
            "the form must not carry an empty {absent}: {carried:?}"
        );
    }

    let resp = app
        .submit_consent(
            &page,
            &[
                ("grant_scope", "read"),
                ("grant_scope", "write"),
                ("grant_scope", "human"),
                ("token", app.human.as_str()),
                ("action", "approve"),
            ],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND, "consent should redirect");
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("Location header")
        .to_string();
    assert!(
        query_param(&location, "state").is_none(),
        "state must not be echoed to a client that never sent one (RFC 6749 §4.1.2): {location}"
    );
    let code = query_param(&location, "code").expect("code in redirect");

    let (status, body) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "code exchange failed: {body}");
    let echoed = body["scope"].as_str().expect("scope").to_string();
    let access = body["access_token"].as_str().expect("access_token");
    let (status, who) = app.get(access, "/v1/whoami").await;
    assert_eq!(status, StatusCode::OK);
    for scope in ["read", "write", "human"] {
        assert!(
            echoed.split(' ').any(|s| s == scope),
            "the echoed scope must name {scope}: {echoed}"
        );
        assert!(
            who["scopes"].as_array().unwrap().iter().any(|s| s == scope),
            "the issued token must actually carry {scope}: {who}"
        );
    }
    // Echoed even though no checkbox for it was ever submitted, because the client
    // asked for it and got what it asked for: a refresh token.
    assert!(
        echoed.split(' ').any(|s| s == "offline_access"),
        "offline_access must still be echoed on the granted scope: {echoed}"
    );
    assert!(
        body["refresh_token"]
            .as_str()
            .is_some_and(|r| !r.is_empty()),
        "…because one was issued: {body}"
    );
}

/// `state` is opaque: RFC 6749 §4.1.2 wants it back exactly as received, and it
/// survives a trip through the consent form as hidden field. Pinned with a trailing
/// `+` — which arrives as a space, since a form body decodes `+` that way — because
/// trimming it here would echo a value the client never sent and fail the strict
/// comparison the parameter exists for.
#[tokio::test]
async fn an_opaque_state_survives_the_consent_round_trip_untouched() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;

    let page = app
        .consent_html(&format!(
            "/oauth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=https%3A%2F%2Fclient.example%2Fcallback\
             &code_challenge={}&code_challenge_method=S256&scope=read&state=abc+",
            pkce_s256_challenge(VERIFIER)
        ))
        .await;
    assert!(
        hidden_fields(&page)
            .iter()
            .any(|(k, v)| k == "state" && v == "abc "),
        "the form must carry the state as received: {:?}",
        hidden_fields(&page)
    );

    let resp = app
        .submit_consent(
            &page,
            &[
                ("grant_scope", "read"),
                ("token", app.human.as_str()),
                ("action", "approve"),
            ],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("Location header")
        .to_string();
    assert_eq!(
        query_param(&location, "state").as_deref(),
        Some("abc "),
        "the trailing space must come back: {location}"
    );
}

/// A scope the human unchecks stays unchecked, including when a failed submission
/// sends the form back. Re-checking it would hand a client authority that had just
/// been declined, on the one page whose whole purpose is narrowing.
#[tokio::test]
async fn a_failed_consent_re_render_keeps_what_the_human_unchecked() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let page = app.consent_html(&app.authorize_url(&client_id)).await;

    // Uncheck `write`, then fail on a truncated token.
    let resp = app
        .submit_consent(
            &page,
            &[
                ("grant_scope", "read"),
                ("token", "tk_truncated"),
                ("action", "approve"),
            ],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let again = resp.text().await.unwrap_or_default();
    assert!(again.contains("not recognized"), "{again}");
    assert!(
        again.contains(r#"name="grant_scope" value="read" checked"#),
        "what was checked stays checked: {again}"
    );
    assert!(
        again.contains(r#"name="grant_scope" value="write">"#),
        "what the human unchecked must come back unchecked: {again}"
    );

    // Fix the token and approve. The declined scope must still not be granted.
    let resp = app
        .submit_consent(
            &again,
            &[
                ("grant_scope", "read"),
                ("token", app.human.as_str()),
                ("action", "approve"),
            ],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("Location header")
        .to_string();
    let code = query_param(&location, "code").expect("code in redirect");
    let (status, body) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "exchange failed: {body}");
    assert_eq!(body["scope"], "read");
    let access = body["access_token"].as_str().unwrap();
    let (_, who) = app.get(access, "/v1/whoami").await;
    assert!(
        !who["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "write"),
        "a scope unchecked before an error must not be granted after it: {who}"
    );
}

/// A code is single-use, and because a replay cannot be told apart from a stolen
/// code racing the real client, it takes down what the code already bought.
#[tokio::test]
async fn a_replayed_code_is_refused_and_revokes_what_it_bought() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let code = app.authorization_code(&client_id, &app.human).await;
    let exchange: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("client_id", &client_id),
        ("redirect_uri", REDIRECT),
        ("code_verifier", VERIFIER),
    ];

    let (status, first) = app.token_call(&exchange).await;
    assert_eq!(status, StatusCode::OK, "first exchange: {first}");
    let access = first["access_token"].as_str().unwrap().to_string();
    assert!(app.mcp_accepts(&access).await);

    let (status, second) = app.token_call(&exchange).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(second["error"], "invalid_grant");
    assert!(
        second["error_description"]
            .as_str()
            .unwrap()
            .contains("revoked"),
        "the description must say that the replay had consequences: {second}"
    );
    assert!(
        !app.mcp_accepts(&access).await,
        "the token the replayed code bought must stop working"
    );
}

/// …and it keeps taking it down after a sweep has run.
///
/// The replay defence lives entirely in the spent code's row, so how long that row
/// is kept *is* how long the defence exists. Swept at expiry — which for a code is
/// within the minute — a replayed code matches nothing, reports "no such grant"
/// like a typo, and everything it bought keeps working. Sweeping explicitly rather
/// than hoping the background tick lands late also takes the timing out of the test
/// above, which was a race against it.
#[tokio::test]
async fn a_spent_code_outlives_the_sweep_so_a_late_replay_is_still_detected() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let code = app.authorization_code(&client_id, &app.human).await;
    let exchange: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("client_id", &client_id),
        ("redirect_uri", REDIRECT),
        ("code_verifier", VERIFIER),
    ];

    let (status, first) = app.token_call(&exchange).await;
    assert_eq!(status, StatusCode::OK, "first exchange: {first}");
    let access = first["access_token"].as_str().unwrap().to_string();

    app.open_store().sweep_expired_oauth().expect("sweep");

    let (status, replay) = app.token_call(&exchange).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(replay["error"], "invalid_grant");
    assert!(
        replay["error_description"]
            .as_str()
            .unwrap_or("")
            .contains("already redeemed"),
        "a swept-over replay must still be reported as a replay, not as an unknown grant: {replay}"
    );
    assert!(
        !app.mcp_accepts(&access).await,
        "the sweep must not cost the replay its revocation"
    );
}

/// A wrong verifier is refused — and must NOT consume the code, or observing a
/// redirect would be enough to deny service to the legitimate client.
#[tokio::test]
async fn a_bad_pkce_verifier_is_refused_without_burning_the_code() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let code = app.authorization_code(&client_id, &app.human).await;

    let (status, body) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            (
                "code_verifier",
                "a-different-verifier-that-is-long-enough-x",
            ),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");

    // The real client still gets through.
    let (status, body) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a failed PKCE attempt must not consume the code: {body}"
    );
}

/// A code issued to one client cannot be redeemed by another, even with the right
/// verifier.
#[tokio::test]
async fn a_code_cannot_be_redeemed_by_a_different_client() {
    let app = TestApp::spawn_with_oauth().await;
    let mine = app.register_client("Mine", &[REDIRECT]).await;
    let theirs = app.register_client("Theirs", &[REDIRECT]).await;
    let code = app.authorization_code(&mine, &app.human).await;

    let (status, body) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &theirs),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");
}

// ---------------------------------------------------------------------------
// Refresh

/// Rotation on every use, reuse detection on the family. The previous *access*
/// token deliberately keeps working until it expires — a client refreshes
/// proactively while requests may still be in flight on it.
#[tokio::test]
async fn refresh_rotates_and_reuse_takes_down_the_family() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let code = app.authorization_code(&client_id, &app.human).await;
    let (_, first) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    let access1 = first["access_token"].as_str().unwrap().to_string();
    let refresh1 = first["refresh_token"].as_str().unwrap().to_string();

    let (status, second) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh1),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "refresh failed: {second}");
    let access2 = second["access_token"].as_str().unwrap().to_string();
    let refresh2 = second["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(refresh1, refresh2, "the refresh token must rotate");
    assert!(app.mcp_accepts(&access2).await);
    assert!(
        app.mcp_accepts(&access1).await,
        "the superseded access token must stay valid until it expires, or a proactive refresh would fail requests in flight"
    );

    // Replaying the rotated token is the signal that one of the two holders is not
    // the real client. Both lose everything.
    let (status, reuse) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh1),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(reuse["error"], "invalid_grant");
    // Reported *as* reuse — this is the one path where a credential may have been
    // stolen, and it must not be worded like the administrative revocations below.
    assert!(
        reuse["error_description"]
            .as_str()
            .unwrap_or("")
            .contains("already redeemed"),
        "presenting a rotated token is reuse and must say so: {reuse}"
    );
    assert!(!app.mcp_accepts(&access1).await, "family revoked");
    assert!(!app.mcp_accepts(&access2).await, "family revoked");

    // And the successor is dead too, so the thief cannot refresh onward. It was
    // never rotated, though — it was taken down with the family — so the refusal is
    // the ended-connection one, not a second theft claim.
    let (status, after) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh2),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{after}");
    assert_eq!(after["error"], "invalid_grant");
    let after_desc = after["error_description"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        after_desc.contains("ended at the takomo server"),
        "a never-rotated credential must not be reported as redeemed twice: {after}"
    );
    assert!(
        !after_desc.contains("already redeemed"),
        "the two refusals must not be swapped: {after}"
    );
}

/// Revoking the token a human consented with ends every connector derived from it.
///
/// The consent snapshot is frozen so it cannot widen, but it is a delegation, not an
/// independent credential: without this check rotation would keep minting hour-long
/// access tokens carrying a revoked token's authority, renewing its own 30-day
/// window forever, and `takomo token revoke` on the human's token would do nothing
/// an operator could observe.
#[tokio::test]
async fn revoking_the_consenting_token_ends_the_connector() {
    let app = TestApp::spawn_with_oauth().await;
    let consenting = app.mint("human:temporary", &["read", "write"], None);
    let (status, who) = app.get(&consenting, "/v1/whoami").await;
    assert_eq!(status, StatusCode::OK);
    let parent = who["token_id"].as_str().expect("token_id").to_string();

    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let refresh = {
        let code = app.authorization_code(&client_id, &consenting).await;
        let (status, body) = app
            .token_call(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("code_verifier", VERIFIER),
            ])
            .await;
        assert_eq!(status, StatusCode::OK, "exchange failed: {body}");
        body["refresh_token"].as_str().unwrap().to_string()
    };
    // A code consented for but not yet redeemed, to check the same rule on the
    // other credential-minting path.
    let unredeemed = app.authorization_code(&client_id, &consenting).await;

    let (status, _) = app
        .delete(&app.admin, &format!("/v1/tokens/{parent}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "revoke the human's token");

    let (status, refused) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["error"], "invalid_grant");
    assert!(
        refused["error_description"]
            .as_str()
            .unwrap_or("")
            .contains("consented"),
        "the description must say which credential went away: {refused}"
    );
    assert!(
        refused["remedy"]
            .as_str()
            .unwrap_or("")
            .contains("/oauth/authorize"),
        "the remedy must point at approving again: {refused}"
    );

    let (status, refused) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &unredeemed),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a code consented with a since-revoked token must not mint anything: {refused}"
    );
    assert_eq!(refused["error"], "invalid_grant");
}

/// …and a consenting token that merely EXPIRES does not, which is the deliberate
/// asymmetry.
///
/// A revocation is an operator deciding a connection must stop; an expiry is
/// bookkeeping typed once, months earlier. Cascading on expiry would turn a stale
/// `--expires` flag into an outage inside someone's chat client and would bound the
/// connection by a clock that, unlike the refresh window, does not slide.
#[tokio::test]
async fn an_expired_consenting_token_leaves_the_connector_running() {
    let app = TestApp::spawn_with_oauth().await;
    let store = app.open_store();
    let (parent, consenting) = store
        .create_token(
            "human:seasonal",
            &["read".to_string(), "write".to_string()],
            None,
            10_000,
            Some(takomo::ids::now_ms() + 60_000),
            None,
        )
        .expect("mint an expiring token");

    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let code = app.authorization_code(&client_id, &consenting).await;
    let (status, first) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "exchange failed: {first}");
    let refresh = first["refresh_token"].as_str().unwrap().to_string();

    // Push that token past its expiry, touching nothing else — the derived access
    // token has an expiry of its own and must keep its.
    let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
    conn.execute(
        "UPDATE tokens SET expires_at = ?1 WHERE id = ?2",
        rusqlite::params![takomo::ids::now_ms() - 60_000, parent.id],
    )
    .expect("backdate the expiry");
    drop(conn);

    // The consenting token itself is finished…
    let (status, _) = app.get(&consenting, "/v1/whoami").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the parent has expired");
    // …and the connection it approved keeps working.
    let (status, second) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an expiry must not end a working connector: {second}"
    );
    let access = second["access_token"].as_str().expect("access_token");
    assert!(
        app.mcp_accepts(access).await,
        "and the token it minted must reach /mcp"
    );
}

/// The fine-grained lever: revoking a connection's own token ends **that**
/// connection and nothing else.
///
/// Marking the row alone would not end anything for long — the client answers the
/// 401 by rotating and is back inside a round trip, on a fresh 30-day window — so
/// revocation has to take the refresh family with it. And it must take only that
/// family: two connectors approved by the same human are two connections, and being
/// able to cut one is the whole reason to reach for this instead of revoking the
/// token they were both approved with.
#[tokio::test]
async fn revoking_a_derived_token_ends_that_connection_and_no_other() {
    let app = TestApp::spawn_with_oauth().await;
    let first = app.full_flow(&app.human).await;
    let second = app.full_flow(&app.human).await;

    let doomed = first["access_token"].as_str().unwrap().to_string();
    let doomed_refresh = first["refresh_token"].as_str().unwrap().to_string();
    let spared = second["access_token"].as_str().unwrap().to_string();
    let spared_refresh = second["refresh_token"].as_str().unwrap().to_string();

    let doomed_id = app.token_id_of(&doomed).await;
    let doomed_client = client_id_of(&app, &doomed_id);
    let spared_client = client_id_of(&app, &app.token_id_of(&spared).await);
    assert_ne!(
        doomed_client, spared_client,
        "the two flows must be two connections"
    );

    let (status, _) = app
        .delete(&app.admin, &format!("/v1/tokens/{doomed_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert!(
        !app.mcp_accepts(&doomed).await,
        "the revoked access token must stop reaching /mcp at once"
    );
    // The client's move on that 401 is to refresh. That must not hand it a
    // replacement, or revocation costs the connector one round trip and nothing else.
    let (status, refused) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &doomed_refresh),
            ("client_id", &doomed_client),
        ])
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a revoked connection must not be able to rotate back: {refused}"
    );
    assert_eq!(refused["error"], "invalid_grant");
    // And it must say what happened. Reporting a deliberate revocation as detected
    // reuse sends whoever reads the client's log into an incident response over an
    // operator's own action.
    let desc = refused["error_description"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        desc.contains("ended at the takomo server"),
        "the refusal must name the revocation: {refused}"
    );
    assert!(
        !desc.contains("already redeemed") && !desc.contains("compromised"),
        "…and must not read as credential theft: {refused}"
    );
    assert!(
        refused["remedy"]
            .as_str()
            .unwrap_or("")
            .contains("/oauth/authorize"),
        "the remedy must say reconnecting works: {refused}"
    );

    // The other connection is untouched, credential and refresh alike.
    assert!(
        app.mcp_accepts(&spared).await,
        "a second connection from the same human must keep working"
    );
    let (status, rotated) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &spared_refresh),
            ("client_id", &spared_client),
        ])
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "…and must still be able to rotate: {rotated}"
    );
}

/// Which client a connection was issued to, read out of the ledger.
///
/// `full_flow` registers its own client and does not hand the id back, and the
/// refresh grant is checked against it — so a test that wants an unambiguous
/// refusal (this revocation, not a client mismatch) has to send the real one.
fn client_id_of(app: &TestApp, token_id: &str) -> String {
    let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
    conn.query_row(
        "SELECT client_id FROM oauth_issued WHERE token_id = ?1",
        rusqlite::params![token_id],
        |row| row.get::<_, String>(0),
    )
    .expect("the ledger names the client this token was issued to")
}

/// …and an ordinary hand-minted token revokes exactly as it always did. This is a
/// shared path, so the blast radius for a token that never came from OAuth has to
/// stay at zero.
#[tokio::test]
async fn revoking_an_ordinary_token_leaves_oauth_connections_alone() {
    let app = TestApp::spawn_with_oauth().await;
    let issued = app.full_flow(&app.human).await;
    let access = issued["access_token"].as_str().unwrap().to_string();
    let refresh = issued["refresh_token"].as_str().unwrap().to_string();

    let plain = app.mint("agent:disposable", &["read", "write"], None);
    let plain_id = app.token_id_of(&plain).await;
    let (status, _) = app
        .delete(&app.admin, &format!("/v1/tokens/{plain_id}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = app.get(&plain, "/v1/whoami").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the revoked token itself stops working"
    );
    assert!(
        app.mcp_accepts(&access).await,
        "an unrelated OAuth connection must not be caught up in it"
    );
    let client_id = client_id_of(&app, &app.token_id_of(&access).await);
    let (status, rotated) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "…nor its ability to rotate: {rotated}"
    );
}

/// The listing has to say which connection a row is, or the lever above cannot be
/// aimed: an OAuth access token is an ordinary token row, an expiry does not
/// distinguish it from a hand-minted one, two connectors approved by the same human
/// are identical in actor, scopes and projects — and revoking the wrong row is not
/// reversible.
#[tokio::test]
async fn the_token_listing_names_the_connection_a_row_belongs_to() {
    let app = TestApp::spawn_with_oauth().await;
    let named = app.register_client("Claude", &[REDIRECT]).await;
    let code = app.authorization_code(&named, &app.human).await;
    let (status, issued) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &named),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "exchange failed: {issued}");
    let derived_id = app
        .token_id_of(issued["access_token"].as_str().unwrap())
        .await;
    let plain_id = app
        .token_id_of(&app.mint("agent:hand-minted", &["read"], None))
        .await;

    let (status, list) = app.get(&app.admin, "/v1/tokens").await;
    assert_eq!(status, StatusCode::OK);
    let row = |id: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("token {id} should be listed: {list}"))
    };

    let derived = row(&derived_id);
    assert_eq!(derived["oauth_client"]["client_name"], "Claude");
    assert_eq!(derived["oauth_client"]["client_id"], named);
    assert_eq!(
        derived["oauth_client"]["label"], "Claude",
        "a human recognizes the name, not the client_id: {derived}"
    );

    // Absent, not null: /v1 evolves additively and a hand-minted token has to
    // serialize exactly as it always did.
    let plain = row(&plain_id);
    assert!(
        plain.get("oauth_client").is_none(),
        "a hand-minted token must be untouched: {plain}"
    );

    // The CLI's `token list` renders the same store answer, so what it shows is
    // pinned here rather than by spawning a binary.
    let listed = app.open_store().list_tokens().expect("list tokens");
    let connection = listed
        .iter()
        .find(|t| t.id == derived_id)
        .and_then(|t| t.oauth_client.as_ref())
        .expect("the store must join the connection through");
    assert_eq!(connection.label(), "Claude");
    assert!(listed
        .iter()
        .find(|t| t.id == plain_id)
        .is_some_and(|t| t.oauth_client.is_none()));
}

/// `client_name` is optional in RFC 7591, so a nameless client must still be
/// identifiable — by its `client_id`, which is the only handle there is.
#[tokio::test]
async fn a_nameless_client_is_still_identifiable_in_the_listing() {
    let app = TestApp::spawn_with_oauth().await;
    let (status, registered) = app
        .json(
            app.request(Method::POST, "/oauth/register")
                .json(&json!({ "redirect_uris": [REDIRECT] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{registered}");
    let client_id = registered["client_id"].as_str().unwrap().to_string();

    let code = app.authorization_code(&client_id, &app.human).await;
    let (status, issued) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "exchange failed: {issued}");
    let derived_id = app
        .token_id_of(issued["access_token"].as_str().unwrap())
        .await;

    let (_, list) = app.get(&app.admin, "/v1/tokens").await;
    let derived = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == derived_id)
        .expect("listed")
        .clone();
    assert_eq!(derived["oauth_client"]["client_id"], client_id);
    assert!(
        derived["oauth_client"]["client_name"].is_null(),
        "an unregistered name must read as absent, not as an empty string: {derived}"
    );
    assert_eq!(
        derived["oauth_client"]["label"], client_id,
        "the label falls back to the only handle there is: {derived}"
    );
}

// ---------------------------------------------------------------------------
// Retention

/// Registration is unauthenticated by specification, so the rows it writes have to
/// be reclaimable: one with neither a code nor a refresh token referencing it is a
/// dead connection attempt. One that still holds either survives, which is what
/// protects a connector in use — its refresh token may be a live connection.
#[tokio::test]
async fn an_unused_client_registration_is_swept_and_a_used_one_survives() {
    let app = TestApp::spawn_with_oauth().await;
    let unused = app.register_client("Never Used", &[REDIRECT]).await;
    let used = app.register_client("Real Client", &[REDIRECT]).await;
    let _code = app.authorization_code(&used, &app.human).await;

    // Backdate both past the retention window — the alternative is waiting a day.
    let stale =
        takomo::ids::now_ms() - (takomo::store::UNUSED_CLIENT_RETENTION_SECONDS * 1000 + 60_000);
    let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
    conn.execute(
        "UPDATE oauth_clients SET created_at = ?1",
        rusqlite::params![stale],
    )
    .expect("backdate");
    drop(conn);

    app.open_store().sweep_expired_oauth().expect("sweep");

    // Observed the way a client would find out: by starting a flow.
    let page = app.consent_html(&app.authorize_url(&used)).await;
    assert!(
        page.contains("Real Client"),
        "a registration that has been used must survive: {page}"
    );
    let resp = no_redirect()
        .get(app.url(&app.authorize_url(&unused)))
        .send()
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "an unused registration must be gone"
    );
    let body = resp.text().await.unwrap_or_default();
    assert!(body.contains("No client is registered"), "{body}");
}

// ---------------------------------------------------------------------------
// Refusals at the authorization endpoint

/// The open-redirect guard. An unregistered target is reported as a page, never
/// as a redirect — sending even an error to an unvalidated location is the hole.
#[tokio::test]
async fn an_unregistered_redirect_uri_is_never_redirected_to() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;

    let resp = no_redirect()
        .get(app.url(&format!(
            "/oauth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=https%3A%2F%2Fattacker.example%2Fsteal\
             &code_challenge={}&code_challenge_method=S256",
            pkce_s256_challenge(VERIFIER)
        )))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(
        resp.headers().get("location").is_none(),
        "nothing may be redirected to an unregistered URI"
    );
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("not one this client registered"),
        "the page should explain the literal match: {body}"
    );

    // An unknown client_id is the same class of refusal.
    let resp = no_redirect()
        .get(app.url(
            "/oauth/authorize?response_type=code&client_id=oc_nope&redirect_uri=https%3A%2F%2Fclient.example%2Fcallback&code_challenge=x&code_challenge_method=S256",
        ))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(resp.headers().get("location").is_none());
}

/// Once the redirect target *is* known good, protocol errors go back to the
/// client as RFC 6749 §4.1.2.1 requires, carrying `state` so it can correlate.
#[tokio::test]
async fn protocol_errors_redirect_once_the_target_is_validated() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;

    for (query, expected) in [
        ("response_type=token", "unsupported_response_type"),
        (
            "response_type=code&code_challenge_method=plain",
            "invalid_request",
        ),
        ("response_type=code", "invalid_request"),
    ] {
        let resp = no_redirect()
            .get(app.url(&format!(
                "/oauth/authorize?{query}&client_id={client_id}\
                 &redirect_uri=https%3A%2F%2Fclient.example%2Fcallback&state={CLIENT_STATE}"
            )))
            .send()
            .await
            .expect("request");
        assert_eq!(resp.status(), StatusCode::FOUND, "for {query}");
        let location = resp.headers().get("location").unwrap().to_str().unwrap();
        assert!(location.starts_with(REDIRECT), "for {query}: {location}");
        assert_eq!(
            query_param(location, "error").as_deref(),
            Some(expected),
            "for {query}"
        );
        assert_eq!(
            query_param(location, "state").as_deref(),
            Some(CLIENT_STATE)
        );
    }
}

/// Declining is a normal outcome of the flow and has to be reported as one.
#[tokio::test]
async fn denying_consent_redirects_access_denied() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let challenge = pkce_s256_challenge(VERIFIER);
    let resp = app
        .form(
            "/oauth/authorize",
            &[
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
                ("response_type", "code"),
                ("state", CLIENT_STATE),
                ("action", "deny"),
            ],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert_eq!(
        query_param(location, "error").as_deref(),
        Some("access_denied")
    );
    assert_eq!(
        query_param(location, "state").as_deref(),
        Some(CLIENT_STATE)
    );
}

/// A credential the server does not accept re-renders the form with an
/// explanation, rather than redirecting a failure the human could not act on.
#[tokio::test]
async fn a_bad_consent_credential_re_renders_the_form() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;

    for (token, expected) in [
        ("", "Paste a takomo token"),
        ("tk_not_a_real_token_at_all", "not recognized"),
    ] {
        let resp = app.consent(&client_id, token, &["read", "write"]).await;
        assert_eq!(resp.status(), StatusCode::OK, "for token '{token}'");
        assert!(
            resp.headers().get("location").is_none(),
            "no redirect on a failed consent"
        );
        let body = resp.text().await.unwrap_or_default();
        assert!(body.contains(expected), "for token '{token}': {body}");
    }

    // A token that carries none of the checked scopes is refused the same way,
    // rather than issuing a credential that can do nothing.
    let read_only = app.mint("agent:ro", &["read"], None);
    let resp = app.consent(&client_id, &read_only, &["write"]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.text().await.unwrap_or_default();
    assert!(body.contains("Nothing would be granted"), "{body}");
}

// ---------------------------------------------------------------------------
// The consent page itself

/// `client_name` arrives through an unauthenticated endpoint and is rendered next
/// to a credential field, so it is the highest-value XSS target in the codebase.
#[tokio::test]
async fn the_consent_page_escapes_a_hostile_client_name() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app
        .register_client("<script>alert('pwn')</script>", &[REDIRECT])
        .await;

    let resp = no_redirect()
        .get(app.url(&app.authorize_url(&client_id)))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK);
    // The form must not be framable, and must only be able to post back here.
    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(csp.contains("form-action 'self'"), "csp: {csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "csp: {csp}");

    let body = resp.text().await.unwrap_or_default();
    assert!(
        !body.contains("<script>alert"),
        "a hostile client_name must be escaped: {body}"
    );
    assert!(
        body.contains("&lt;script&gt;"),
        "…and still shown, escaped: {body}"
    );
    // The scopes the flow asked for are on the form for the human to narrow.
    assert!(
        body.contains(r#"name="grant_scope" value="read""#),
        "{body}"
    );
    assert!(
        body.contains(r#"name="grant_scope" value="write""#),
        "{body}"
    );
    assert!(
        !body.contains(r#"value="admin""#),
        "admin must never be offered: {body}"
    );
}

// ---------------------------------------------------------------------------
// Token endpoint hygiene

/// The refusals a misconfigured client runs into first. Each has to name what is
/// missing — "invalid_request" alone is what makes these hard to debug.
#[tokio::test]
async fn the_token_endpoint_reports_what_a_request_is_missing() {
    let app = TestApp::spawn_with_oauth().await;

    let cases: [(Vec<(&str, &str)>, &str); 4] = [
        (
            vec![("grant_type", "authorization_code")],
            "invalid_request",
        ),
        (vec![("client_id", "oc_x")], "invalid_request"),
        (
            vec![("grant_type", "client_credentials"), ("client_id", "oc_x")],
            "unsupported_grant_type",
        ),
        (
            vec![("grant_type", "authorization_code"), ("client_id", "oc_x")],
            "invalid_request",
        ),
    ];
    for (fields, expected) in cases {
        let (status, body) = app.token_call(&fields).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "for {fields:?}: {body}");
        assert_eq!(body["error"], expected, "for {fields:?}: {body}");
        assert!(
            body["remedy"].as_str().is_some_and(|r| !r.is_empty()),
            "every refusal carries a remedy: {body}"
        );
    }
}

/// RFC 6749 §5.1: a token response must not be cached. It carries a bearer
/// credential in a plain JSON body.
#[tokio::test]
async fn token_responses_are_not_cacheable() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let code = app.authorization_code(&client_id, &app.human).await;
    let resp = app
        .form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("code_verifier", VERIFIER),
            ],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
}

// ---------------------------------------------------------------------------
// Configuration

/// Every value derived from `TAKOMO_PUBLIC_URL` is compared byte-for-byte by a
/// client, so it is validated at startup rather than producing a connection that
/// fails inside someone else's product.
#[test]
fn public_url_validation_is_strict_about_what_it_accepts() {
    assert_eq!(
        OauthConfig::from_public_url("https://takomo.example.com/")
            .expect("trailing slash is trimmed, not refused")
            .issuer(),
        "https://takomo.example.com"
    );
    assert_eq!(
        OauthConfig::from_public_url("https://takomo.example.com")
            .unwrap()
            .resource(),
        "https://takomo.example.com/mcp"
    );
    // Loopback http is what the test suite and a local trial run on.
    assert!(OauthConfig::from_public_url("http://127.0.0.1:8080").is_ok());
    assert!(OauthConfig::from_public_url("http://localhost:8080").is_ok());

    for (bad, why) in [
        ("http://takomo.example.com", "plain http off loopback"),
        ("takomo.example.com", "no scheme"),
        ("https://takomo.example.com/takomo", "a path prefix"),
        ("https://takomo.example.com?x=1", "a query string"),
        ("", "empty"),
    ] {
        let err =
            OauthConfig::from_public_url(bad).expect_err(&format!("{why} must be refused: {bad}"));
        assert!(!err.is_empty(), "the refusal must explain itself ({why})");
    }
}

/// The consent page's CSP has to name the client's callback origin in
/// `form-action`, or approving is blocked by the browser rather than by us.
///
/// `form-action` is enforced against every hop of the navigation a form starts,
/// not just its POST target — and the POST to `/oauth/authorize` answers with a
/// 302 to the client's `redirect_uri`. With `form-action 'self'` alone, Chrome
/// refuses that hop, so a human who typed a valid token and clicked Approve sees
/// nothing happen and the client reports only that it could not connect. Denying
/// redirects the same way, so it fails the same way.
#[tokio::test]
async fn the_consent_csp_allows_the_navigation_approving_actually_makes() {
    let app = TestApp::spawn_with_oauth().await;
    let client_id = app.register_client("Test Client", &[REDIRECT]).await;

    let resp = no_redirect()
        .get(app.url(&app.authorize_url(&client_id)))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK);
    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    assert!(
        csp.contains("form-action 'self' https://client.example"),
        "form-action must name the callback ORIGIN — and only the origin, not the \
         path, which CSP would compare with a prefix match: {csp}"
    );
    assert!(
        !csp.contains("/callback"),
        "the path must not leak into the policy: {csp}"
    );

    // The rest of the policy is unchanged — this widened one directive, and a
    // page that runs no script keeps saying so.
    for directive in [
        "default-src 'none'",
        "base-uri 'none'",
        "frame-ancestors 'none'",
    ] {
        assert!(csp.contains(directive), "{directive} missing from {csp}");
    }

    // A loopback client (RFC 8252 — Claude Code is one) carries its port, since
    // an origin that drops it is a different origin to the browser.
    let native = app
        .register_client("Native Client", &["http://127.0.0.1:49152/cb"])
        .await;
    let resp = no_redirect()
        .get(app.url(&format!(
            "/oauth/authorize?response_type=code&client_id={native}\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcb\
             &code_challenge={}&code_challenge_method=S256",
            pkce_s256_challenge(VERIFIER)
        )))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK);
    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        csp.contains("form-action 'self' http://127.0.0.1:49152"),
        "a loopback callback keeps its port: {csp}"
    );

    // The refusal page redirects nowhere, so it widens nothing.
    let resp = no_redirect()
        .get(app.url(&format!(
            "/oauth/authorize?response_type=code&client_id={client_id}\
             &redirect_uri=https%3A%2F%2Fattacker.example%2Fsteal\
             &code_challenge={}&code_challenge_method=S256",
            pkce_s256_challenge(VERIFIER)
        )))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        csp.contains("form-action 'self'") && !csp.contains("attacker.example"),
        "a refused request must not name its own rejected target: {csp}"
    );
}

/// A connection approved by Ada is still Ada.
///
/// The consent snapshot copies the actor, the scopes and the project allowlist,
/// so it must copy the *person* too — otherwise a hosted client (claude.ai,
/// ChatGPT) would connect as an anonymous holder of her scopes, and every
/// question addressed to her by name would be unanswerable from the one surface
/// she actually reads. Inherited, never granted: consent can narrow what a client
/// may do, but it cannot attach an identity the consenting credential lacked.
#[tokio::test]
async fn an_issued_token_inherits_the_consenting_person() {
    let app = TestApp::spawn_with_oauth().await;
    let store = app.open_store();
    let ada = store
        .create_user(
            &takomo::store::UserCreate {
                handle: "ada".to_string(),
                name: Some("Ada Lovelace".to_string()),
                email: None,
                meta: None,
                projects: vec!["tp".to_string()],
            },
            "test:setup",
        )
        .expect("create user");
    let (_, consenting) = store
        .create_token(
            "human:ada",
            &["read".to_string(), "write".to_string(), "human".to_string()],
            None,
            10_000,
            None,
            Some("ada"),
        )
        .expect("mint a token bound to Ada");

    let client_id = app.register_client("Test Client", &[REDIRECT]).await;
    let code = app.authorization_code(&client_id, &consenting).await;
    let (status, issued) = app
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "exchange failed: {issued}");
    let access = issued["access_token"].as_str().unwrap().to_string();
    let refresh = issued["refresh_token"].as_str().unwrap().to_string();

    let (status, me) = app.get(&access, "/v1/whoami").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["user"]["handle"], "ada", "the connection is Ada: {me}");
    assert_eq!(me["user"]["id"], ada.id, "{me}");

    // And it survives rotation, which is what makes the connection hers a month
    // later rather than only in the first hour.
    let (status, rotated) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    let rotated_access = rotated["access_token"].as_str().unwrap().to_string();
    let (_, still_her) = app.get(&rotated_access, "/v1/whoami").await;
    assert_eq!(still_her["user"]["handle"], "ada", "{still_her}");
}
