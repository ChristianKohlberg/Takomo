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
        (k == key).then(|| v.replace("%2D", "-").replace("%2E", "."))
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
        let code = self.authorization_code(&client_id, token).await;
        let (status, body) = self
            .token_call(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("client_id", &client_id),
                ("redirect_uri", REDIRECT),
                ("code_verifier", VERIFIER),
            ])
            .await;
        assert_eq!(status, StatusCode::OK, "code exchange failed: {body}");
        body
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
    for scope in ["read", "write", "human", "offline_access"] {
        assert!(
            page.contains(&format!(r#"name="grant_scope" value="{scope}" checked"#)),
            "an omitted scope must offer {scope}, pre-checked: {page}"
        );
    }
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
    assert!(!app.mcp_accepts(&access1).await, "family revoked");
    assert!(!app.mcp_accepts(&access2).await, "family revoked");

    // And the successor is dead too, so the thief cannot refresh onward.
    let (status, after) = app
        .token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh2),
            ("client_id", &client_id),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{after}");
    assert_eq!(after["error"], "invalid_grant");
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

// ---------------------------------------------------------------------------
// Retention

/// Registration is unauthenticated by specification, so the rows it writes have to
/// be reclaimable: one that produced neither a code nor a refresh token is a dead
/// connection attempt. One that produced either is left alone however old it is,
/// because its refresh token may still be a live connection.
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
