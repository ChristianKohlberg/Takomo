//! OAuth 2.1 authorization server: discovery, dynamic client registration, the
//! consent screen, and the token endpoint.
//!
//! ## Why this exists
//!
//! `/mcp` authenticates with `Authorization: Bearer tk_...`, which every *local*
//! MCP client can send and no *hosted* one can: claude.ai, ChatGPT and the Gemini
//! app can only be handed a URL, and they expect to negotiate credentials over
//! OAuth. Without this module the only ways to connect them are an authless proxy
//! (making the URL the password on a tracker that accepts writes) or a token in a
//! query string (leaks through logs; prohibited by the MCP authorization spec).
//!
//! ## Two deliberate deviations from house style
//!
//! 1. **The error bodies are RFC 6749's, not takomo's.** Everywhere else a
//!    rejection carries `code` / `message` / `remedy` (see `crate::error`). An
//!    OAuth client parses `error` and nothing else, so these endpoints emit
//!    `{"error": "...", "error_description": "..."}` — with a `remedy` alongside,
//!    which RFC 6749 §5.2 permits and which keeps the teaching-error habit for
//!    the human who ends up reading it in a log.
//! 2. **Registration does not reject unknown fields.** Every other mutating
//!    handler calls `reject_unknown` so a typo cannot silently drop a field. RFC
//!    7591 §2 requires the opposite here: clients send whatever metadata they
//!    like (`client_uri`, `logo_uri`, `software_id`, …) and the server must
//!    ignore what it does not understand. Rejecting those would refuse every real
//!    client.
//!
//! ## What consent means without user accounts
//!
//! takomo has tokens, not users, and a `client_credentials` grant is not an
//! option — Claude requires every connection to be consented to by a human. So
//! the consent screen authenticates the human with **a takomo token they already
//! hold**: they paste one, see who is asking and for what, uncheck anything they
//! do not want to hand over, and approve. What the client receives is a *derived*
//! token — same actor, a subset of the scopes, the same project allowlist and
//! write budget, plus an expiry and its own revocation handle.
//!
//! That is strictly better than pasting the original token into a client's header
//! field, which is the alternative on offer today: this one expires, is revocable
//! on its own, is attributable to one client, and can never carry `admin` (see
//! [`GRANTABLE_SCOPES`]).

use super::{blocking_read, body_object, first, get_str, get_string_array, query_pairs, ApiJson};
use crate::ids::{now_ms, token_hash};
use crate::server::AppState;
use crate::store::{GrantRejection, GrantedAccess, OauthExchange, MAX_REDIRECT_URIS};
use axum::extract::{RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

/// The takomo scopes a consent may hand to a client.
///
/// `admin` is absent on purpose and must stay absent. It is the scope that mints
/// tokens, creates and deletes projects, and force-releases other workers'
/// leases — administration, not work. A chat client connected over OAuth has no
/// business holding it, and its owner pasting an admin token into the consent
/// screen (the common case: it may be the only token they have) must not
/// silently promote the connector. Consent narrows; it never widens.
const GRANTABLE_SCOPES: &[&str] = &["read", "write", "human"];

/// Advertised in `scopes_supported` but not a takomo scope: it asks for a refresh
/// token. This server issues one on every authorization-code exchange regardless,
/// because a hosted client that cannot refresh silently stops working after an
/// hour — so the scope is accepted, echoed back, and otherwise inert. Claude
/// appends it automatically when it appears in `scopes_supported`.
///
/// Being inert is why the consent screen states it as a sentence rather than
/// offering it as a checkbox: see [`consent_page`].
const OFFLINE_ACCESS: &str = "offline_access";

/// Registrations per minute, across all callers.
///
/// `POST /oauth/register` is unauthenticated by definition — that is what
/// "dynamic" means — so it is the one write endpoint here with no token to charge
/// and no identity to key a budget by. The budget is therefore global and
/// deliberately loose: a real deployment registers a handful of clients ever
/// (Claude re-registers on each fresh connection, ChatGPT once).
///
/// What it does is *pace* registration, not bound it: 30/minute is still ~43k
/// rows a day if something keeps asking. What keeps the table from growing
/// without limit is the sweep — past `UNUSED_CLIENT_RETENTION_SECONDS`, a
/// registration with no authorization code and no refresh token left referencing it
/// is deleted (see [`crate::store::Store::sweep_expired_oauth`]).
const REGISTRATIONS_PER_MINUTE: i64 = 30;

/// The key that global budget is charged to. A constant, because there is nothing
/// caller-specific to key it by; see [`REGISTRATIONS_PER_MINUTE`].
const REGISTER_BUDGET_KEY: &str = "oauth:register";

/// Everything the authorization server needs to describe itself, derived once at
/// startup from `TAKOMO_PUBLIC_URL`.
///
/// Held as an `Option` on [`AppState`]: with no public URL there is no issuer, no
/// `resource` identifier that could match what a user types into a connector
/// dialog, and no way to build a redirect back — so OAuth is off, and the
/// endpoints say so rather than half-working.
#[derive(Debug, Clone)]
pub struct OauthConfig {
    /// Public origin, no trailing slash, e.g. `https://takomo.example.com`. Also
    /// the OAuth issuer identifier.
    base: String,
}

impl OauthConfig {
    /// Validate an operator-supplied public base URL.
    ///
    /// Strict on purpose, and at startup rather than on first request: every
    /// value derived from this string is compared byte-for-byte by a client (the
    /// issuer against the discovery URL it was fetched from, the `resource`
    /// against the URL the user typed), so a trailing slash or a stray path is
    /// not cosmetic — it is a connection that fails with "couldn't reach the MCP
    /// server" and no clue why.
    pub fn from_public_url(raw: &str) -> Result<OauthConfig, String> {
        let base = raw.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err("TAKOMO_PUBLIC_URL is empty; unset it to disable OAuth, or set it to the public origin takomo is reached at (e.g. https://takomo.example.com)".to_string());
        }
        let rest = match base.strip_prefix("https://") {
            Some(rest) => rest,
            None => match base.strip_prefix("http://") {
                // Plain http is allowed only for loopback, which is what the test
                // suite and a local trial run on. Anywhere else it would publish
                // an issuer whose tokens travel in clear text, and OAuth 2.1
                // requires https for exactly that reason.
                Some(rest) if is_loopback_host(rest) => rest,
                Some(_) => return Err(format!(
                    "TAKOMO_PUBLIC_URL '{base}' uses plain http. OAuth requires https except on loopback: put TLS in front (a platform, a reverse proxy, Tailscale) and set the https URL."
                )),
                None => return Err(format!(
                    "TAKOMO_PUBLIC_URL '{base}' must be an absolute URL starting with https:// (or http:// on loopback)."
                )),
            },
        };
        if rest.is_empty() {
            return Err(format!("TAKOMO_PUBLIC_URL '{base}' has no host."));
        }
        if base.contains('?') || base.contains('#') {
            return Err(format!(
                "TAKOMO_PUBLIC_URL '{base}' must be a bare origin: no query string and no fragment."
            ));
        }
        // A path component would put takomo's own routes somewhere other than
        // where it advertises them, since every route here is mounted at the root.
        if rest.contains('/') {
            return Err(format!(
                "TAKOMO_PUBLIC_URL '{base}' must be an origin without a path — takomo serves /mcp and /oauth/* at the root. Use a subdomain rather than a path prefix, or strip the path."
            ));
        }
        Ok(OauthConfig { base })
    }

    /// The OAuth issuer identifier (RFC 8414): the bare origin.
    pub fn issuer(&self) -> &str {
        &self.base
    }

    /// The protected resource identifier. This is the MCP endpoint, and it must
    /// equal the URL the user enters into their client, path and all — Claude
    /// compares the two.
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.base)
    }

    /// Where a `401` points a client for RFC 9728 protected-resource metadata.
    pub fn resource_metadata_url(&self) -> String {
        format!("{}/.well-known/oauth-protected-resource", self.base)
    }

    /// The `WWW-Authenticate` challenge value for an unauthenticated MCP request.
    /// Without this header on a `401`, a hosted client has no way to discover
    /// where the authorization server is (Claude ignores the header on a `200`,
    /// and probing well-known paths is a fallback that costs round-trips).
    pub fn www_authenticate(&self) -> String {
        format!(
            "Bearer resource_metadata=\"{}\"",
            self.resource_metadata_url()
        )
    }
}

/// Is the host part of a URL (everything after the scheme) loopback?
fn is_loopback_host(rest: &str) -> bool {
    let host = rest.split(['/', ':']).next().unwrap_or("");
    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1"
}

// ---------------------------------------------------------------------------
// Error and redirect plumbing (RFC 6749 §4.1.2.1, §5.2)

/// An RFC 6749 error body. `remedy` is takomo's addition; extra members are
/// explicitly allowed and clients ignore them.
fn oauth_error(status: StatusCode, error: &str, description: &str, remedy: &str) -> Response {
    let mut resp = (
        status,
        Json(json!({
            "error": error,
            "error_description": description,
            "remedy": remedy,
        })),
    )
        .into_response();
    // RFC 6749 §5.1: token endpoint responses must not be cached. Applied to every
    // error here too — they can carry the reason a credential was refused.
    no_store(&mut resp);
    resp
}

fn no_store(resp: &mut Response) {
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().expect("static"));
    resp.headers_mut()
        .insert(header::PRAGMA, "no-cache".parse().expect("static"));
}

/// The error takomo returns when OAuth is asked for but not configured. Written
/// for the operator who will read it in a log, since a client will only show
/// "couldn't connect".
fn oauth_disabled() -> Response {
    oauth_error(
        StatusCode::NOT_FOUND,
        "temporarily_unavailable",
        // Says "unset or unusable" rather than naming one, deliberately: OAuth is
        // off in both cases, this handler cannot tell them apart, and since
        // takomo-z919 the unusable case no longer stops the server — so it is a
        // state a live instance can genuinely be serving in. Asserting "not set"
        // when it *is* set would send an operator looking in the wrong place.
        "This takomo instance has no OAuth authorization server: TAKOMO_PUBLIC_URL is either unset or not usable as an issuer, so the server cannot state its own identity or build a redirect back to a client. Its startup line says which of the two.",
        "Set TAKOMO_PUBLIC_URL to the public origin this server is reached at — a bare origin, e.g. https://takomo.example.com: no path, no query, no trailing slash, and plain http only on loopback. Restart, then check the startup line reads 'OAuth issuer ...' rather than 'OAuth OFF -'. Until then hosted clients cannot connect; local clients can still use a bearer token directly.",
    )
}

/// Percent-encode a value for a query string, escaping everything outside the
/// unreserved set. Deliberately conservative: `state` is opaque client data and
/// an `error_description` is prose, so anything that could end the value or start
/// a new parameter has to be encoded.
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build a redirect back to a client, appending to any query string the
/// registered URI already carries.
fn redirect_to(redirect_uri: &str, params: &[(&str, &str)]) -> Response {
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={}", encode_query_value(v)))
        .collect::<Vec<_>>()
        .join("&");
    let location = format!("{redirect_uri}{sep}{query}");
    let mut resp = StatusCode::FOUND.into_response();
    match location.parse() {
        Ok(value) => {
            resp.headers_mut().insert(header::LOCATION, value);
        }
        // A registered redirect URI is validated on the way in, so this is
        // unreachable in practice; failing closed beats emitting a header the
        // browser would interpret loosely.
        Err(_) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "The registered redirect_uri cannot be used in a Location header.",
                "Re-register the client with a plain absolute https URI.",
            )
        }
    }
    no_store(&mut resp);
    resp
}

/// Redirect an error back to the client, per RFC 6749 §4.1.2.1. Only ever called
/// with a `redirect_uri` that already matched the client's registration.
fn redirect_error(redirect_uri: &str, state: Option<&str>, error: &str, desc: &str) -> Response {
    let mut params = vec![("error", error), ("error_description", desc)];
    if let Some(s) = state {
        params.push(("state", s));
    }
    redirect_to(redirect_uri, &params)
}

// ---------------------------------------------------------------------------
// Discovery

/// `GET /.well-known/oauth-protected-resource` (RFC 9728), and the
/// `/.well-known/oauth-protected-resource/mcp` form a client probes when the
/// resource lives at a path.
///
/// Unauthenticated, and it has to be: this is the document that tells a client
/// how to *get* a credential. It reveals only what the server already announces
/// in a `WWW-Authenticate` header.
pub async fn protected_resource_metadata(State(state): State<Arc<AppState>>) -> Response {
    let Some(cfg) = state.oauth.as_ref() else {
        return oauth_disabled();
    };
    Json(json!({
        "resource": cfg.resource(),
        "authorization_servers": [cfg.issuer()],
        "scopes_supported": scopes_supported(),
        "bearer_methods_supported": ["header"],
        "resource_name": "takomo",
    }))
    .into_response()
}

/// `GET /.well-known/oauth-authorization-server` (RFC 8414).
pub async fn authorization_server_metadata(State(state): State<Arc<AppState>>) -> Response {
    let Some(cfg) = state.oauth.as_ref() else {
        return oauth_disabled();
    };
    let base = cfg.issuer();
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "scopes_supported": scopes_supported(),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        // Public clients only: PKCE is the client authentication here, so there is
        // no secret to present at the token endpoint. Advertising `none` is also
        // what a CIMD-capable client checks for — takomo does not advertise
        // `client_id_metadata_document_supported`, so such a client falls back to
        // dynamic registration, which is implemented below.
        "token_endpoint_auth_methods_supported": ["none"],
        // Required by the MCP authorization spec so a client can verify S256
        // support before starting a flow. `plain` is deliberately absent.
        "code_challenge_methods_supported": ["S256"],
    }))
    .into_response()
}

fn scopes_supported() -> Vec<&'static str> {
    let mut v = GRANTABLE_SCOPES.to_vec();
    v.push(OFFLINE_ACCESS);
    v
}

// ---------------------------------------------------------------------------
// Dynamic client registration (RFC 7591)

/// `POST /oauth/register`.
///
/// Unauthenticated, as the RFC requires, and therefore the one endpoint here that
/// needs its own budget — see [`REGISTRATIONS_PER_MINUTE`]. Unknown metadata
/// members are ignored rather than rejected (RFC 7591 §2); the fields that
/// actually constrain later security decisions are validated strictly.
pub async fn register(
    State(state): State<Arc<AppState>>,
    ApiJson(body): ApiJson<Value>,
) -> Response {
    if state.oauth.is_none() {
        return oauth_disabled();
    }
    if let Err(retry_after_secs) = crate::auth::debit_shared_window(
        &state.oauth_register_rate,
        REGISTER_BUDGET_KEY,
        REGISTRATIONS_PER_MINUTE,
    ) {
        // `Retry-After` on every 429 is a house convention (see spec/auth.md), and
        // the one place it matters most is here: this refusal reaches a connector
        // that has no human watching it, so a number to wait for is the difference
        // between one retry and a tight loop.
        let mut resp = oauth_error(
            StatusCode::TOO_MANY_REQUESTS,
            "temporarily_unavailable",
            &format!("This server accepts a limited number of client registrations per minute and that budget is exhausted. It frees up in {retry_after_secs}s."),
            &format!("Wait {retry_after_secs}s (see Retry-After) and retry the connection. If you are scripting registrations, register once and reuse the client_id."),
        );
        if let Ok(value) = retry_after_secs.to_string().parse() {
            resp.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return resp;
    }

    let obj = match body_object(&body) {
        Ok(obj) => obj,
        Err(_) => {
            return invalid_metadata("The registration request body must be a JSON object.");
        }
    };

    let redirect_uris = match get_string_array(obj, "redirect_uris") {
        Ok(Some(list)) => list,
        Ok(None) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_redirect_uri",
                "redirect_uris is required: takomo only issues authorization codes to a redirect target registered up front, because the exact-match check against that list is what prevents an open redirect.",
                "Register with {\"redirect_uris\": [\"https://your-client/callback\"]}.",
            );
        }
        Err(_) => return invalid_metadata("redirect_uris must be an array of strings."),
    };
    if redirect_uris.is_empty() || redirect_uris.len() > MAX_REDIRECT_URIS {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            &format!(
                "redirect_uris must contain between 1 and {MAX_REDIRECT_URIS} entries, got {}.",
                redirect_uris.len()
            ),
            "Register only the callback URIs this client actually uses.",
        );
    }
    for uri in &redirect_uris {
        if let Err(why) = validate_redirect_uri(uri) {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_redirect_uri",
                &format!("redirect_uri '{uri}' is not acceptable: {why}"),
                "Use an absolute https URI with no fragment (http is accepted only for loopback, for native clients).",
            );
        }
    }

    // Only `none` is meaningful: every client here is a public client, and issuing
    // a secret that a hosted product would have to store would be security
    // theatre. Saying so beats accepting the request and quietly not honouring it.
    match get_str(obj, "token_endpoint_auth_method") {
        Ok(Some(method)) if method != "none" => {
            return invalid_metadata(&format!(
                "token_endpoint_auth_method '{method}' is not supported. takomo registers public clients and authenticates the token exchange with PKCE, so the only accepted value is 'none'."
            ));
        }
        Err(_) => return invalid_metadata("token_endpoint_auth_method must be a string."),
        _ => {}
    }
    if let Ok(Some(types)) = get_string_array(obj, "grant_types") {
        if let Some(bad) = types
            .iter()
            .find(|t| !matches!(t.as_str(), "authorization_code" | "refresh_token"))
        {
            return invalid_metadata(&format!(
                "grant_type '{bad}' is not supported. This server implements authorization_code and refresh_token."
            ));
        }
    }
    if let Ok(Some(types)) = get_string_array(obj, "response_types") {
        if let Some(bad) = types.iter().find(|t| t.as_str() != "code") {
            return invalid_metadata(&format!(
                "response_type '{bad}' is not supported. This server implements the authorization code flow only, so the only accepted value is 'code'."
            ));
        }
    }

    let client_name = match get_str(obj, "client_name") {
        Ok(Some(name)) => {
            if let Err(why) = validate_client_name(&name) {
                return invalid_metadata(&format!("client_name is not acceptable: {why}"));
            }
            // By `chars`, so a 200-character cut cannot land inside a multi-byte one.
            name.chars().take(200).collect::<String>()
        }
        Ok(None) => String::new(),
        Err(_) => return invalid_metadata("client_name must be a string."),
    };

    match state
        .store
        .register_oauth_client(&client_name, &redirect_uris)
    {
        Ok(client) => {
            let mut resp = (StatusCode::CREATED, Json(client.to_json())).into_response();
            no_store(&mut resp);
            resp
        }
        Err(e) => e.into_response(),
    }
}

fn invalid_metadata(description: &str) -> Response {
    oauth_error(
        StatusCode::BAD_REQUEST,
        "invalid_client_metadata",
        description,
        "Fix the registration metadata and retry. Members this server does not recognize are ignored, so only the ones named in the description matter.",
    )
}

/// `client_name` is the other piece of client-supplied data this server *renders* —
/// on the consent page, in `takomo token list`, in `GET /v1/tokens` — and
/// registration is unauthenticated, so it is validated exactly like a redirect URI
/// below, and for the same reason.
///
/// Refused rather than quietly stripped: this codebase does not fail silently, and a
/// name carrying a newline or an ANSI escape is not a name anybody typed. RFC 7591
/// requires *unrecognized* metadata to be ignored, which is not licence to accept a
/// malformed value for a member the server understands and puts in front of a human.
fn validate_client_name(name: &str) -> Result<(), &'static str> {
    if name.chars().any(crate::store::display_hostile) {
        return Err("it contains a control character or a bidirectional override. Those forge the display rather than describe the client: in a terminal listing an escape sequence can erase the line above, and a newline turns one row into two.");
    }
    Ok(())
}

/// A redirect URI is the one piece of client-supplied data that later gets turned
/// into a `Location` header, so it is validated once here, strictly, and matched
/// literally afterwards.
fn validate_redirect_uri(uri: &str) -> Result<(), &'static str> {
    if uri.len() > 512 {
        return Err("it is longer than 512 characters");
    }
    if uri.contains('#') {
        return Err("it contains a fragment, which RFC 6749 §3.1.2 forbids in a redirect URI");
    }
    if uri.contains(['\r', '\n', ' ', '\t']) || uri.chars().any(|c| c.is_control()) {
        return Err("it contains whitespace or control characters");
    }
    let rest = match uri.strip_prefix("https://") {
        Some(rest) => rest,
        None => match uri.strip_prefix("http://") {
            // RFC 8252: a native client redirects to a loopback address on an
            // ephemeral port. Claude Code is such a client.
            Some(rest) if is_loopback_host(rest) => rest,
            Some(_) => return Err("plain http is only accepted for loopback addresses"),
            None => return Err("it is not an absolute http(s) URI"),
        },
    };
    if rest.is_empty() || rest.starts_with('/') {
        return Err("it has no host");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Authorization endpoint

/// The parts of an authorization request that survive the round trip through the
/// consent form.
struct AuthzRequest {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

/// A request parameter, with an empty or whitespace-only value read as **absent**.
///
/// Which is what it is. A form round trip cannot express "this parameter was not
/// sent": an HTML field with nothing in it arrives as `scope=`. Reading that as a
/// present-but-empty value is what silently narrowed a no-`scope` flow to `read`
/// alone, echoed `state=` back to a client that never sent one (RFC 6749 §4.1.2
/// says `state` MUST NOT be included when it was absent from the request), and
/// stored `resource=""`. So the whole authorization request goes through this,
/// rather than each parameter defending itself.
///
/// Surrounding whitespace goes with it, which is right for the three parameters
/// this server *interprets* — a scope list is whitespace-delimited, a `resource`
/// and a `redirect_uri` are compared against values that have none. It is wrong for
/// `state`; see [`opaque`].
fn present(pairs: &[(String, String)], key: &str) -> Option<String> {
    first(pairs, key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The same absent-if-empty rule, without the trim: for a parameter this server
/// only carries.
///
/// `state` is opaque client data that RFC 6749 §4.1.2 requires back exactly as
/// received, and `query_pairs` decodes `+` as a space — so a client whose `state`
/// ends in `+` sends a value ending in a space, and trimming it would echo back
/// something it never sent, failing the strict comparison the parameter exists for.
fn opaque(pairs: &[(String, String)], key: &str) -> Option<String> {
    first(pairs, key)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// `GET /oauth/authorize` — render the consent screen.
pub async fn authorize_get(
    State(state): State<Arc<AppState>>,
    RawQuery(query): RawQuery,
) -> Response {
    if state.oauth.is_none() {
        return oauth_disabled();
    }
    let pairs = query_pairs(query.as_deref());
    let (req, client_name) = match parse_authz_request(&state, &pairs).await {
        Ok(parsed) => parsed,
        Err(resp) => return resp,
    };
    let requested = requested_scopes(req.scope.as_deref());
    consent_page(&req, &client_name, &requested, None, None)
}

/// `POST /oauth/authorize` — the consent form's target.
///
/// No CSRF token, deliberately: until the human types a credential into it this
/// form carries no authority at all, and the credential is the one thing an
/// attacker who forged the request cannot supply. What does matter — that the
/// page cannot be framed, and that the form can only post back to this origin —
/// is enforced by the headers in [`consent_page`].
pub async fn authorize_post(State(state): State<Arc<AppState>>, body: String) -> Response {
    if state.oauth.is_none() {
        return oauth_disabled();
    }
    let pairs = query_pairs(Some(&body));
    let (req, client_name) = match parse_authz_request(&state, &pairs).await {
        Ok(parsed) => parsed,
        Err(resp) => return resp,
    };

    if first(&pairs, "action") == Some("deny") {
        return redirect_error(
            &req.redirect_uri,
            req.state.as_deref(),
            "access_denied",
            "The person at the consent screen declined.",
        );
    }

    // What the human left checked, intersected with what the flow asked for.
    let requested = requested_scopes(req.scope.as_deref());
    let checked: Vec<String> = super::all(&pairs, "grant_scope");
    let asked_for: Vec<String> = requested
        .iter()
        .filter(|s| checked.iter().any(|c| c == *s))
        .cloned()
        .collect();

    let token = first(&pairs, "token").unwrap_or("").trim().to_string();
    if token.is_empty() {
        return consent_page(
            &req,
            &client_name,
            &requested,
            Some(&checked),
            Some("Paste a takomo token to approve with. It is the credential this connection will act as."),
        );
    }

    let hash = token_hash(&token);
    let lookup = {
        let store_state = state.clone();
        blocking_read(move || store_state.store.lookup_token(&hash)).await
    };
    let row = match lookup {
        Ok(Some(row)) => row,
        Ok(None) => {
            return consent_page(
                &req,
                &client_name,
                &requested,
                Some(&checked),
                Some("That token is not recognized by this server. Check for a truncated copy/paste, or mint a fresh one with: takomo token create."),
            )
        }
        Err(e) => return e.into_response(),
    };
    let now = now_ms();
    if row.revoked_at.is_some() || row.expires_at.is_some_and(|exp| exp <= now) {
        return consent_page(
            &req,
            &client_name,
            &requested,
            Some(&checked),
            Some("That token is no longer valid — it has been revoked or has expired. Mint a fresh one with: takomo token create."),
        );
    }

    // Narrow to what this token actually carries. `admin` cannot appear, because
    // it is not in GRANTABLE_SCOPES and so was never offered.
    let granted: Vec<String> = asked_for
        .iter()
        .filter(|s| row.scopes.iter().any(|owned| owned == *s))
        .cloned()
        .collect();
    if granted.is_empty() {
        return consent_page(
            &req,
            &client_name,
            &requested,
            Some(&checked),
            Some("Nothing would be granted: none of the checked scopes are carried by that token (or none are checked). Pick a token with the scopes this client needs, or check at least one it has."),
        );
    }

    // `offline_access` rides along in the echoed scope string when the client asked
    // for it, and deliberately does not consult `checked`: it is not offered as a
    // checkbox, because the refresh token is issued either way. See
    // [`OFFLINE_ACCESS`].
    let mut scope_out = granted.clone();
    if requested.iter().any(|s| s == OFFLINE_ACCESS) {
        scope_out.push(OFFLINE_ACCESS.to_string());
    }

    let grant = GrantedAccess {
        actor: row.actor.clone(),
        scopes: granted,
        projects: row.projects.clone(),
        rate_limit: row.rate_limit,
        scope: scope_out.join(" "),
        granted_by: row.id.clone(),
    };
    match state.store.create_oauth_code(
        &req.client_id,
        &req.redirect_uri,
        &req.code_challenge,
        req.resource.as_deref(),
        &grant,
    ) {
        Ok(code) => {
            let mut params = vec![("code", code.as_str())];
            if let Some(s) = req.state.as_deref() {
                params.push(("state", s));
            }
            redirect_to(&req.redirect_uri, &params)
        }
        Err(e) => e.into_response(),
    }
}

/// Validate an authorization request, in the order security requires: the client
/// and its redirect URI first, because until those are known good there is
/// nowhere safe to send an error, and everything after them is reported by
/// redirect.
async fn parse_authz_request(
    state: &Arc<AppState>,
    pairs: &[(String, String)],
) -> Result<(AuthzRequest, String), Response> {
    let client_id = first(pairs, "client_id").unwrap_or("").to_string();
    if client_id.is_empty() {
        return Err(authorize_page_error(
            "This authorization request names no client_id, so there is no registration to check its redirect target against.",
        ));
    }
    let client = match state.store.get_oauth_client(&client_id) {
        Ok(Some(client)) => client,
        Ok(None) => {
            return Err(authorize_page_error(
                "No client is registered under that client_id on this server. If the client was registered against a different takomo instance, or the server's database was replaced, reconnect so it registers again.",
            ))
        }
        Err(e) => return Err(e.into_response()),
    };

    // An absent redirect_uri is allowed only when there is exactly one
    // registered, which is the one case where "which one" has no answer to get
    // wrong (RFC 6749 §3.1.2.3).
    let redirect_uri = match present(pairs, "redirect_uri") {
        Some(uri) if client.redirect_uris.contains(&uri) => uri,
        Some(_) => {
            return Err(authorize_page_error(
                "The redirect_uri in this request is not one this client registered. takomo matches it literally — a differing scheme, host, port, path, or trailing slash is a different URI — so nothing is redirected anywhere.",
            ))
        }
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        None => {
            return Err(authorize_page_error(
                "This request omits redirect_uri and the client registered more than one, so there is no unambiguous target.",
            ))
        }
    };

    let state_param = opaque(pairs, "state");
    let response_type = first(pairs, "response_type").unwrap_or("");
    if response_type != "code" {
        return Err(redirect_error(
            &redirect_uri,
            state_param.as_deref(),
            "unsupported_response_type",
            "This server implements the authorization code flow only; response_type must be 'code'.",
        ));
    }
    let method = first(pairs, "code_challenge_method").unwrap_or("");
    if method != "S256" {
        return Err(redirect_error(
            &redirect_uri,
            state_param.as_deref(),
            "invalid_request",
            "code_challenge_method must be S256. 'plain' is not accepted, and PKCE is not optional here: it is the only client authentication a public client has.",
        ));
    }
    let code_challenge = first(pairs, "code_challenge").unwrap_or("").to_string();
    if code_challenge.is_empty() {
        return Err(redirect_error(
            &redirect_uri,
            state_param.as_deref(),
            "invalid_request",
            "code_challenge is required (PKCE, RFC 7636).",
        ));
    }

    Ok((
        AuthzRequest {
            client_id,
            redirect_uri,
            code_challenge,
            state: state_param,
            scope: present(pairs, "scope"),
            resource: present(pairs, "resource"),
        },
        client.client_name,
    ))
}

/// The scopes a flow is asking for: what the client requested, filtered to what
/// this server can grant.
///
/// Unknown scope tokens are dropped rather than refused — a hosted client that
/// asks for one scope this server has never heard of should still connect. That is
/// not silent: the token response echoes the `scope` actually granted (RFC 6749
/// §5.1 requires it whenever it differs from the request), and the consent screen
/// shows the human exactly what is on offer.
///
/// A request with no `scope` at all means "act as me": the consent screen offers
/// everything grantable, pre-checked, and the intersection with the pasted token's
/// own scopes happens when it is approved.
fn requested_scopes(scope: Option<&str>) -> Vec<String> {
    let asked: Vec<&str> = scope.unwrap_or_default().split_whitespace().collect();
    // "Act as me" — no `scope` parameter, or one carrying nothing but whitespace,
    // which is the same statement. Spelled out rather than left to fall through:
    // the filter below produces an empty vec here, and every "is this set
    // acceptable" test over an empty vec is vacuously true. That is exactly how an
    // omitted `scope` came to mean `read` alone — the opposite of what it asks for.
    if asked.is_empty() {
        return scopes_supported().iter().map(|s| s.to_string()).collect();
    }
    let mut out: Vec<String> = scopes_supported()
        .iter()
        .filter(|s| asked.contains(s))
        .map(|s| s.to_string())
        .collect();
    // Every credential this server issues can at least read; a request naming only
    // scopes it does not offer would otherwise present an empty consent form.
    // `offline_access` does not count, because it grants nothing on its own.
    if !out.iter().any(|s| GRANTABLE_SCOPES.contains(&s.as_str())) {
        out.insert(0, "read".to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// Token endpoint

/// `POST /oauth/token`.
///
/// Reads `application/x-www-form-urlencoded`, which RFC 6749 §4.1.3 mandates and
/// which is worth stating because it is a common failure: a JSON-only body parser
/// answers `415` here and the client reports an unexplained connection failure.
/// The body is parsed with the same hand-rolled pair splitter the query strings
/// use, so no extractor can reject it before the teaching error is reachable.
pub async fn token(State(state): State<Arc<AppState>>, body: String) -> Response {
    if state.oauth.is_none() {
        return oauth_disabled();
    }
    let pairs = query_pairs(Some(&body));
    let grant_type = first(&pairs, "grant_type").unwrap_or("");
    let client_id = first(&pairs, "client_id").unwrap_or("").to_string();
    if client_id.is_empty() {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "client_id is required. A public client does not authenticate at the token endpoint, so client_id in the form body is the only thing identifying which client is redeeming this grant.",
            "Send client_id in the form body alongside the grant.",
        );
    }

    let outcome = match grant_type {
        "authorization_code" => {
            let code = first(&pairs, "code").unwrap_or("");
            let verifier = first(&pairs, "code_verifier").unwrap_or("");
            let redirect_uri = first(&pairs, "redirect_uri").unwrap_or("");
            if code.is_empty() || verifier.is_empty() {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "Both code and code_verifier are required for the authorization_code grant.",
                    "Send the code from the redirect together with the PKCE code_verifier whose S256 hash you sent as code_challenge.",
                );
            }
            state
                .store
                .exchange_oauth_code(code, &client_id, redirect_uri, verifier)
        }
        "refresh_token" => {
            let refresh = first(&pairs, "refresh_token").unwrap_or("");
            if refresh.is_empty() {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required for the refresh_token grant.",
                    "Send the refresh_token from the previous token response.",
                );
            }
            state.store.refresh_oauth_token(refresh, &client_id)
        }
        "" => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "grant_type is required.",
                "Send grant_type=authorization_code or grant_type=refresh_token in a form-urlencoded body.",
            )
        }
        other => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                &format!("grant_type '{other}' is not supported. This server implements authorization_code and refresh_token; client_credentials in particular is not offered, because every credential here represents a human's consent."),
                "Use grant_type=authorization_code, then grant_type=refresh_token to renew. For a non-interactive agent, mint a token directly instead: takomo token create.",
            )
        }
    };

    match outcome {
        Ok(OauthExchange::Issued(tokens)) => {
            let mut resp = Json(json!({
                "access_token": tokens.access_token,
                "token_type": "Bearer",
                "expires_in": tokens.expires_in,
                "refresh_token": tokens.refresh_token,
                "scope": tokens.scope,
            }))
            .into_response();
            no_store(&mut resp);
            resp
        }
        // Every refusal below is `invalid_grant`: RFC 6749 §5.2 puts "invalid,
        // expired, revoked, does not match the redirection URI, or was issued to
        // another client" under that one code. The description is what
        // distinguishes them for whoever is debugging.
        Ok(OauthExchange::Rejected(why)) => oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            match why {
                GrantRejection::Unknown => "No such grant. An authorization code is single-use and short-lived, and a refresh token is retired the moment it is rotated, so a repeated exchange lands here.",
                GrantRejection::Expired => "This grant has expired. Authorization codes live about a minute; refresh tokens expire after 30 days of not being used.",
                GrantRejection::Replayed => "This grant was already redeemed. Because a replay cannot be told apart from a stolen credential racing the real one, everything issued from it has now been revoked.",
                GrantRejection::ClientMismatch => "This grant was issued to a different client_id.",
                GrantRejection::RedirectMismatch => "The redirect_uri does not match the one this authorization code was issued for. It must be byte-identical to the value sent to /oauth/authorize.",
                GrantRejection::PkceMismatch => "The code_verifier does not match the code_challenge this authorization code was bound to (PKCE, RFC 7636).",
                GrantRejection::ConsentWithdrawn => "The takomo credential this connection was consented with has been revoked (or deleted), so this connection was revoked with it and nothing further can be issued: a human has to approve again. Note that this is revocation specifically — a consenting token that merely expired would not have stopped the connection.",
                GrantRejection::ConnectionRevoked => "This connection has been ended at the takomo server: its credentials were revoked, so this refresh token is no longer valid and nothing further can be issued for it. Either an operator revoked this connection's token, or reuse was detected on another credential of the same connection and everything in it was revoked together. Nothing here says this refresh token was itself misused.",
            },
            match why {
                GrantRejection::Replayed => "Start a fresh authorization: send the user back through /oauth/authorize. If you did not initiate this exchange, treat the credential as compromised — it has been revoked here.",
                GrantRejection::ConsentWithdrawn => "Send the user back through /oauth/authorize to approve again, with a takomo token that has not been revoked (mint one with: takomo token create). If you did not expect this, the operator revoked that token deliberately — ask them before reconnecting.",
                GrantRejection::ConnectionRevoked => "Reconnecting through /oauth/authorize works — a fresh consent issues a new connection. But someone revoked this one on purpose, so find out why before doing that.",
                _ => "Start a fresh authorization at /oauth/authorize and exchange the new code once.",
            },
        ),
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------------------
// The consent screen

/// Minimal HTML escaping for text interpolated into the consent page.
///
/// `client_name` arrives through an **unauthenticated** registration endpoint, so
/// it is attacker-controlled by construction, and it is rendered next to a field
/// the human is about to paste a credential into. Everything interpolated goes
/// through here.
fn esc(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// An error the consent flow cannot report by redirect, because the redirect
/// target itself is what could not be established. Rendered as a page, never sent
/// onward — redirecting an error to an unvalidated URI is the open redirect this
/// whole ordering exists to avoid.
fn authorize_page_error(message: &str) -> Response {
    let page = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>takomo — authorization request refused</title>
{STYLE}</head>
<body><main>
<h1>This authorization request was refused</h1>
<p class="err">{}</p>
<p class="muted">Nothing was redirected. takomo will not send a response — not even an error — to a location it has not verified against a client registration.</p>
</main></body></html>"#,
        esc(message)
    );
    html_response(StatusCode::BAD_REQUEST, page)
}

/// The consent screen: who is asking, for what, and a field for the credential
/// that authorizes it.
///
/// `selected` is `None` on the first render, where everything requested is
/// pre-checked, and `Some` on a re-render after a failed submission, where it is
/// what the human actually left checked. Re-checking a box they deliberately
/// cleared would hand a client a scope it had been declined, on a page whose whole
/// purpose is narrowing.
fn consent_page(
    req: &AuthzRequest,
    client_name: &str,
    requested: &[String],
    selected: Option<&[String]>,
    error: Option<&str>,
) -> Response {
    let who = if client_name.trim().is_empty() {
        format!("An unnamed client (<code>{}</code>)", esc(&req.client_id))
    } else {
        format!(
            "<strong>{}</strong> (<code>{}</code>)",
            esc(client_name),
            esc(&req.client_id)
        )
    };

    let mut scope_rows = String::new();
    for scope in requested {
        // `offline_access` is stated, not offered. It grants no authority of its
        // own — it asks for a refresh token, which this server issues either way —
        // so a checkbox for it would be a control that changes nothing, and the one
        // behaviour it appears to promise (unchecking it) would be a trap: a hosted
        // client that cannot refresh silently stops working an hour after it
        // connects. A non-choice presented as a choice is worse than a plain
        // sentence saying what will happen.
        if scope == OFFLINE_ACCESS {
            continue;
        }
        let (label, note) = match scope.as_str() {
            "read" => ("read", "See projects, tickets, comments and questions."),
            "write" => (
                "write",
                "Create and change tickets, claim work, comment, ask questions.",
            ),
            "human" => (
                "human",
                "Answer ask-a-human questions and drive human-gated transitions.",
            ),
            other => (other, "Requested by the client."),
        };
        let checked = match selected {
            None => " checked",
            Some(list) if list.iter().any(|s| s == scope) => " checked",
            Some(_) => "",
        };
        scope_rows.push_str(&format!(
            r#"<li><label><input type="checkbox" name="grant_scope" value="{v}"{c}> <code>{v}</code> — {n}</label></li>"#,
            v = esc(label),
            c = checked,
            n = esc(note),
        ));
    }

    let error_block = match error {
        Some(msg) => format!(r#"<p class="err">{}</p>"#, esc(msg)),
        None => String::new(),
    };
    // A field with nothing in it is omitted, not emitted empty: `value=""` would
    // post back `scope=` / `state=` / `resource=` where the client sent nothing at
    // all, and the POST is parsed by the same code as the original request — so an
    // empty field is the round trip inventing a parameter. See [`present`].
    let hidden = [
        ("client_id", Some(req.client_id.as_str())),
        ("redirect_uri", Some(req.redirect_uri.as_str())),
        ("code_challenge", Some(req.code_challenge.as_str())),
        ("code_challenge_method", Some("S256")),
        ("response_type", Some("code")),
        ("state", req.state.as_deref()),
        ("scope", req.scope.as_deref()),
        ("resource", req.resource.as_deref()),
    ]
    .iter()
    .filter_map(|(k, v)| v.map(|v| (k, v)))
    .map(|(k, v)| format!(r#"<input type="hidden" name="{}" value="{}">"#, k, esc(v)))
    .collect::<String>();

    let page = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>takomo — connect a client</title>
{STYLE}</head>
<body><main>
<h1>Connect a client to takomo</h1>
<p>{who} wants to act on this takomo instance on your behalf.</p>
{error_block}
<form method="post" action="/oauth/authorize" autocomplete="off">
{hidden}
<h2>It is asking for</h2>
<ul class="scopes">{scope_rows}</ul>
<p class="muted small">This connection stays alive without asking you again: takomo always issues a refresh token, because a client that cannot refresh stops working an hour after you connect it. Ending it is <code>takomo token revoke</code> on this connection's own entry, which stops this one and nothing else — or on the token you approve with below, which stops every connection approved with it.</p>
<h2>Approve as</h2>
<p class="muted">Paste a takomo token. The client does not receive it: takomo issues a <em>separate</em> token with the same actor, the scopes you leave checked above, the same project allowlist and the same write budget — plus an expiry, and its own entry in <code>takomo token list</code> — named after the client asking, so you can find it and revoke just this connection, refresh token and all.</p>
<p class="muted">The <code>admin</code> scope is never granted this way, whatever the token you paste carries.</p>
<p><input type="password" name="token" placeholder="tk_..." size="44" autocomplete="off" spellcheck="false" autofocus></p>
<p class="actions">
  <button type="submit" name="action" value="approve" class="primary">Approve</button>
  <button type="submit" name="action" value="deny">Deny</button>
</p>
</form>
<p class="muted small">Check the address bar before typing: this page should be served by your own takomo host over https. takomo never asks for a credential anywhere else.</p>
</main></body></html>"#
    );
    html_response(StatusCode::OK, page)
}

/// Shared styling for the two OAuth pages. Inline, like the rest of takomo's
/// HTML: one self-contained document, no second request, nothing to cache-bust.
const STYLE: &str = r#"<style>
:root { color-scheme: light dark; }
body { margin: 0; padding: 2rem 1rem; font: 15px/1.55 ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 34rem; margin: 0 auto; }
h1 { font-size: 1.35rem; margin: 0 0 1rem; }
h2 { font-size: 1rem; margin: 1.6rem 0 .5rem; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .92em; }
.muted { opacity: .78; }
.small { font-size: .87em; }
.err { padding: .7rem .85rem; border-radius: 6px; background: #fdecea; color: #8a1c11; }
@media (prefers-color-scheme: dark) { .err { background: #3a1512; color: #f7c6c0; } }
ul.scopes { list-style: none; padding: 0; margin: 0; }
ul.scopes li { padding: .35rem 0; }
input[type=password] { padding: .5rem .6rem; font: inherit; width: 100%; box-sizing: border-box; }
.actions { display: flex; gap: .6rem; margin-top: 1.2rem; }
button { padding: .55rem 1.1rem; font: inherit; cursor: pointer; border-radius: 6px; border: 1px solid currentColor; background: transparent; }
button.primary { background: #1f6feb; border-color: #1f6feb; color: #fff; }
</style>"#;

/// HTML with the headers these two pages specifically need.
///
/// Not `api::secure_html`, which the board and inbox use: that policy sets
/// `form-action 'none'`, which would block the consent form's own POST. This one
/// swaps that for `form-action 'self'` and drops `connect-src`, since neither page
/// runs any script at all.
fn html_response(status: StatusCode, body: String) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; \
                 base-uri 'none'; frame-ancestors 'none'",
            ),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            // A consent page's URL carries the client's `state`; keep it out of
            // any onward Referer.
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}
