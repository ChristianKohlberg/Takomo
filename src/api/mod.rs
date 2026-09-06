//! HTTP handlers. Parsing is done by hand from `serde_json::Value` so that
//! malformed input gets teaching errors, not bare 400s.

pub mod agent_chat;
pub mod bugs;
pub mod checklist;
pub mod claims;
pub mod diagrams;
pub mod docprops;
pub mod docs;
pub mod docsync;
pub mod environments;
pub mod events;
pub mod export;
pub mod initiatives;
pub mod metrics;
pub mod mindmaps;
pub mod oauth;
pub mod projects;
pub mod questions;
pub mod schedules;
pub mod shares;
pub mod spec_history;
pub mod speech;
pub mod tags;
pub mod testruns;
pub mod tickets;
pub mod tokens;
pub mod transition;
pub mod users;
pub mod work_lanes;
pub mod workflows;

use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::Json;
use serde_json::Value;
use std::time::Duration;

/// A request-body JSON extractor that maps axum's built-in rejections (invalid
/// JSON, wrong/absent `Content-Type`, empty body) to the same structured
/// teaching error the rest of the API returns, instead of a bare 400/415.
pub struct ApiJson<T>(pub T);

impl<T, S> axum::extract::FromRequest<S> for ApiJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(ApiJson(value)),
            Err(rej) => Err(ApiError::bad_request(
                "validation.json",
                format!(
                    "Request body must be valid JSON sent with 'Content-Type: application/json'. {}",
                    rej.body_text()
                ),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Work-loop conventions.

/// Attach the project's writing conventions to a work-loop response, so an agent
/// sees them *before* it writes a ticket, a comment, or a question:
///
/// - `language_hint` — the human-facing language questions belong in;
/// - `style_hint` — the project's style guide for text the agent writes.
///
/// They ride as top-level sibling keys on whatever object the response already
/// is (a ticket, a created ticket, a lease), the way `similar` and `lease`
/// already do — so `/v1` stays additive and no existing field moves or changes
/// type. Each key is omitted when the project sets nothing, so a project with no
/// conventions gets no extra payload at all.
///
/// This is the single source of the hint wording for *both* surfaces: `src/mcp.rs`
/// calls it too, so REST and MCP cannot drift into telling agents different
/// things. `out` must be a JSON object; anything else is left untouched.
pub fn attach_conventions(state: &AppState, out: &mut Value, project: &str) {
    if !out.is_object() {
        return;
    }
    // A hint is advisory: a project that vanished under us must not turn a
    // successful claim into an error.
    let Ok(conv) = state.store.project_conventions(project) else {
        return;
    };
    if let Some(lang) = conv.question_language {
        out["language_hint"] = serde_json::json!({
            "question_language": lang,
            "note": format!("This project expects human-facing questions (takomo_ask) and their options written in {lang}. Internal ticket text may be in another language."),
        });
    }
    if let Some(style) = conv.style_guide {
        out["style_hint"] = serde_json::json!({
            "style_guide": style,
            "note": "This project's house style for text you write — ticket titles and bodies, comments, and human-facing questions. Follow it as written.",
        });
    }
}

pub async fn healthz() -> Json<Value> {
    Json(serde_json::json!({ "status": "ok", "version": crate::server::VERSION }))
}

/// Bare hostname hits `/` before the SPA router runs; send readers to the board
/// without requiring a bearer token (same placement as `/healthz` and `/oauth/*`).
pub async fn root_redirect() -> axum::response::Redirect {
    axum::response::Redirect::temporary("/board")
}

/// Serve the app document with defense-in-depth headers. The page holds the
/// viewer's bearer token in `localStorage`, so a strict CSP keeps any future
/// injection from exfiltrating it, and `frame-ancestors`/`X-Frame-Options`
/// block clickjacking of the board.
///
/// `script-src` is now `'self'` with NO `'unsafe-inline'`. That is a real
/// tightening, and it is a side effect of the move to one bundle: the scripts
/// used to be inlined into each document, which forced `'unsafe-inline'` and
/// with it the whole class of injected-`<script>` attacks. They are separate
/// same-origin files now, so the allowance is simply no longer needed.
///
/// `style-src` keeps `'unsafe-inline'`: React writes element `style` attributes
/// (the `style={{…}}` prop), and CSP counts those as inline styles. Dropping it
/// would need every one of those rewritten to a class.
fn secure_html(body: &'static str) -> impl axum::response::IntoResponse {
    use axum::http::header;
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                // `font-src` is named explicitly rather than left to
                // `default-src`. The two are equivalent today — the display face
                // is served from `/assets/` like every other embedded file — but
                // a font arriving from a CDN is exactly the change somebody
                // makes without thinking, and a directive that is already
                // written down is the one that refuses it.
                "default-src 'self'; script-src 'self'; \
                 style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                 font-src 'self'; connect-src 'self'; base-uri 'none'; \
                 form-action 'none'; frame-ancestors 'none'",
            ),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        body,
    )
}

/// Read-only kanban board: a self-contained single-page app that talks to the
/// same-origin `/v1` API with a token the viewer supplies in the browser. The
/// page itself is unauthenticated (all data fetches carry the Bearer token);
/// serving static HTML leaks nothing the API does not already guard.
///
/// ONE document for all four surfaces. It used to be four self-contained
/// documents with every script inlined; the app is client-side routed now, so
/// the same `index.html` is served on `/board`, `/inbox`, `/initiatives` and
/// `/schedules` and the router picks the surface from the path.
///
/// Serving it on each real path rather than on a catch-all is deliberate: a
/// typed URL, a bookmark and an `#a=` answer link all still hit a route the
/// server actually knows, and an unknown `/v1/...` path keeps answering a JSON
/// 404 instead of handing an agent an HTML document.
static INDEX_HTML: &str = include_str!("../../web/dist/index.html");

// The app's JavaScript and CSS, embedded as a compile-time manifest: `ASSETS`,
// generated by build.rs from the generated `web/dist/assets/`.
//
// scripts/build.sh and the deployment pipelines generate assets before Cargo
// embeds them. The deployed binary needs neither Node nor loose frontend files.
//
// This used to be four `include_str!`s naming `app.js`, `vendor.js`,
// `runtime.js` and `app.css` one by one, with `web/vite.config.ts` failing the
// build if the bundle emitted anything else. That contract could not survive code
// splitting — a dynamic `import()` emits a chunk whose name is not known until the
// bundler runs — and the editor route is far too large to live in the single
// `app.js` chunk.
//
// **The property the fixed names bought is unchanged.** It was never the names
// that mattered but that there is no directory to traverse: `ASSETS` is an exact
// lookup table baked in at compile time, so a request either matches an entry or
// 404s, and `../` reaches nothing because no filesystem is consulted.
//
// The names are still STABLE — content hashing stays off in `web/vite.config.ts`
// — so cache correctness comes from the ETag below.
//
// (A plain comment, not a doc comment: rustdoc does not document items produced
// by a macro expansion, and `#[warn(unused_doc_comments)]` correctly says so.)
include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// `GET /board` — the kanban board.
///
/// Serves three audiences from one route: the board itself, a read-only `#s=`
/// share, and a single-use `#a=` answer link for an outside expert. The
/// fragment never reaches the server, so all three are this same document; the
/// client decides which to render.
pub async fn board() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// `GET /inbox` — the ask-a-human inbox (folder rail, question list, reading and
/// answer pane). Unauthenticated like every page route; the data fetches carry
/// the viewer's bearer token.
pub async fn inbox() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// `GET /initiatives` — the ideas a fleet is nurturing.
///
/// One of the two surfaces that WRITE, which is why `/v1/initiatives` grew POST
/// and PATCH handlers: an initiative is fed by people as well as agents, and a
/// browser cannot call an MCP tool.
///
/// Named `initiatives_page` rather than `initiatives`: the sibling module
/// `crate::api::initiatives` holds the JSON handlers, and while Rust would let a
/// function share that name (different namespace), a reader should not have to
/// know that to tell which one a call site means.
pub async fn initiatives_page() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// `GET /documents` — prose humans and agents write at the same time.
///
/// The surface `/initiatives` could not be: an initiative's document is reduced
/// from an append-only entry log, "latest view per pane wins", so revising a
/// paragraph means appending a whole new copy of the pane and whatever somebody
/// else wrote meanwhile loses. This one is a Yjs CRDT over a WebSocket.
///
/// Built BESIDE `/initiatives`, not over it: nothing here writes to an
/// initiative, and a document may name the one it was distilled from.
pub async fn documents_page() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// `GET /mindmaps` — brainstorming, before any of it is an idea. Same document as
/// every other surface; the router picks this one from the path.
pub async fn mindmaps_page() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// `GET /schedules` — the recurrence page.
///
/// Rows, not columns, and that is the whole design decision: the board sorts by
/// state, but a schedule's content is a *history*, so forcing cadences into
/// columns would throw away the axis that carries the meaning.
pub async fn schedules_page() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// `GET /settings` — the admin console: the current token, the database export,
/// tokens and projects.
///
/// Unauthenticated like every other page route, which is worth stating plainly
/// because everything ON it is admin-only. The document is the same shell the
/// other four routes serve — it carries no data — and every request behind it is
/// refused without an `admin` token. Gating the HTML would protect nothing and
/// would break the one thing a page route owes a caller: answering a typed URL.
/// `GET /verification` — whether the tests a feature was agreed on still pass.
///
/// Named `_page` for the same reason `initiatives_page` is: a sibling module in
/// `src/api/` already owns the name.
pub async fn verification_page() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// `GET /environments` — the registry of places a check can be run.
pub async fn environments_page() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// `GET /agent-queues` — read-only inspection of on-demand agent jobs.
pub async fn agent_queues_page() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

pub async fn settings_page() -> impl axum::response::IntoResponse {
    secure_html(INDEX_HTML)
}

/// A strong ETag over the asset body.
///
/// The assets have stable names, so a client cannot tell one build's `app.js`
/// from the next by URL — the ETag is what does it. `sha2` is already a
/// dependency (token hashing), so this costs nothing new, and 16 hex characters
/// of SHA-256 is far past what a cache validator needs.
fn etag_for(body: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(body);
    format!("\"{}\"", hex16(&digest))
}

fn hex16(bytes: &[u8]) -> String {
    bytes.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Compare two entity-tags the way `If-None-Match` requires: WEAK comparison,
/// which ignores the `W/` prefix on either side (RFC 9110 §13.1.2).
///
/// This is not pedantry. In production a compressing proxy sits in front of this
/// server, and compressing a response changes its bytes — so the proxy correctly
/// downgrades the strong `"abc"` we emit to a weak `W/"abc"`. The browser stores
/// and returns the weak form. A literal `==` therefore never matches, every
/// revalidation answers 200 with a full body, and the ~110 kB vendor bundle is
/// re-downloaded on every load — with the ETag machinery all present and looking
/// correct. It reproduces against any CDN and against nothing locally, which is
/// how it shipped.
fn weak_eq(a: &str, b: &str) -> bool {
    a.strip_prefix("W/").unwrap_or(a) == b.strip_prefix("W/").unwrap_or(b)
}

/// Serve one embedded asset with revalidation caching.
///
/// `must-revalidate` with `max-age=0` rather than a far-future immutable cache:
/// immutable is only safe when the URL changes with the content, and these URLs
/// deliberately do not. So the browser asks every time and almost always gets a
/// 304 with no body — one cheap round trip instead of re-downloading ~110 kB of
/// vendor bundle on every navigation.
fn asset(
    headers: &axum::http::HeaderMap,
    body: &'static [u8],
    content_type: &'static str,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    let etag = etag_for(body);
    let fresh = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|c| weak_eq(c.trim(), &etag)));

    let common = [
        (header::ETAG, etag.clone()),
        (
            header::CACHE_CONTROL,
            "public, max-age=0, must-revalidate".to_string(),
        ),
        // The assets are same-origin and referenced by the app's own document,
        // but they are still attacker-visible surface; keep the sniffing off.
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
    ];

    if fresh {
        return (StatusCode::NOT_MODIFIED, common).into_response();
    }
    (common, [(header::CONTENT_TYPE, content_type)], body).into_response()
}

/// `GET /assets/{file}` — one embedded asset, looked up by exact name.
///
/// The path parameter is matched against [`ASSETS`] and nothing else. It is not
/// joined onto a directory, so there is no traversal to defend against: a name
/// that is not in the table is a 404, and `..` is simply a name that is not in
/// the table.
///
/// Serving JSON rather than an HTML document on a miss is deliberate and matches
/// how `/v1/*` behaves — a mistyped asset URL is a programming error, and an
/// HTML body would be parsed as JavaScript by the `<script>` tag that asked.
pub async fn asset_by_name(
    axum::extract::Path(file): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let key = format!("assets/{file}");
    match ASSETS.iter().find(|(name, _, _)| *name == key) {
        Some((_, mime, body)) => asset(&headers, body, mime),
        None => crate::error::ApiError::not_found("asset", &file)
            .remedy(
                "The app's assets are embedded in the binary by name. This URL is \
                 not one of them — a stale index.html from an older build is the \
                 usual cause; hard-reload the page."
                    .to_string(),
            )
            .into_response(),
    }
}

/// The takomo mark ("tako" = octopus) as an SVG favicon, served at both
/// `/favicon.svg` and `/favicon.ico`. The document links `/favicon.svg`
/// explicitly; the `.ico` route catches the bare request legacy browsers make
/// on their own so it never 404s. Static, unauthenticated, leaks nothing.
pub async fn favicon() -> impl axum::response::IntoResponse {
    (
        [
            (axum::http::header::CONTENT_TYPE, "image/svg+xml"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_str!("../favicon.svg"),
    )
}

// ---------------------------------------------------------------------------
// Body/query parsing helpers

pub fn body_object(body: &Value) -> ApiResult<&serde_json::Map<String, Value>> {
    body.as_object().ok_or_else(|| {
        ApiError::bad_request(
            "validation.body_json",
            "The request body must be a JSON object.",
        )
    })
}

pub fn get_str(obj: &serde_json::Map<String, Value>, key: &str) -> ApiResult<Option<String>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ApiError::bad_request(
            "validation.field_type",
            format!("Field '{key}' must be a string."),
        )),
    }
}

pub fn require_str(obj: &serde_json::Map<String, Value>, key: &str) -> ApiResult<String> {
    get_str(obj, key)?.ok_or_else(|| {
        ApiError::bad_request(
            "validation.field_required",
            format!("Field '{key}' is required and must be a string."),
        )
    })
}

pub fn get_i64(obj: &serde_json::Map<String, Value>, key: &str) -> ApiResult<Option<i64>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_i64().map(Some).ok_or_else(|| {
            ApiError::bad_request(
                "validation.field_type",
                format!("Field '{key}' must be an integer."),
            )
        }),
    }
}

/// A number that may have a fractional part — a canvas coordinate, and so far
/// only that. Accepts an integer too, because `{"x": 40}` is what a caller writes
/// when a node happens to land on a whole pixel.
pub fn get_f64(obj: &serde_json::Map<String, Value>, key: &str) -> ApiResult<Option<f64>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or_else(|| {
            ApiError::bad_request(
                "validation.field_type",
                format!("Field '{key}' must be a number."),
            )
        }),
    }
}

pub fn get_bool(obj: &serde_json::Map<String, Value>, key: &str) -> ApiResult<Option<bool>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(ApiError::bad_request(
            "validation.field_type",
            format!("Field '{key}' must be a boolean."),
        )),
    }
}

pub fn get_string_array(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> ApiResult<Option<Vec<String>>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(s) => out.push(s.clone()),
                    _ => {
                        return Err(ApiError::bad_request(
                            "validation.field_type",
                            format!("Field '{key}' must be an array of strings."),
                        ))
                    }
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err(ApiError::bad_request(
            "validation.field_type",
            format!("Field '{key}' must be an array of strings."),
        )),
    }
}

/// Reject any body key not in `known`, so a typo'd field (e.g. `expires_second`
/// or `fenc`) is a loud 400 rather than a silently-ignored — and dangerous —
/// no-op. Every mutating handler that parses a JSON object body should call it.
pub fn reject_unknown(obj: &serde_json::Map<String, Value>, known: &[&str]) -> ApiResult<()> {
    let unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !known.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "validation.unknown_field",
        format!(
            "Unknown field(s): {}. Accepted: {}.",
            unknown.join(", "),
            known.join(", ")
        ),
    ))
}

/// Parse a raw query string into (key, value) pairs (percent-decoded), keeping
/// repeats — needed for repeatable `label` params.
pub fn query_pairs(raw: Option<&str>) -> Vec<(String, String)> {
    let Some(raw) = raw else { return Vec::new() };
    raw.split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push(h * 16 + l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn first<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub fn all(pairs: &[(String, String)], key: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .collect()
}

pub fn parse_i64_param(pairs: &[(String, String)], key: &str) -> ApiResult<Option<i64>> {
    match first(pairs, key) {
        None => Ok(None),
        Some(raw) => raw.parse::<i64>().map(Some).map_err(|_| {
            ApiError::bad_request(
                "validation.query",
                format!("Query parameter '{key}' must be an integer, got '{raw}'."),
            )
        }),
    }
}

/// A bounded list, shaped so a short page cannot be mistaken for a whole one.
///
/// Always carries `total` (how many matched, ignoring the page size) alongside
/// `items` and the `limit` actually applied. When the page left something out it
/// also carries a `note` saying so in the terms of the surface being read —
/// `more` is that surface's advice, e.g. which parameter to raise.
///
/// The `note` is prose rather than a flag on purpose, and it is the same bet the
/// error contract makes: the reader is usually an LLM, and a sentence telling it
/// what to do next is acted on where a `truncated: true` it did not think to
/// check is not. `total` remains the machine answer for anything that wants one.
pub fn paged(items: Vec<Value>, total: i64, limit: i64, more: &str) -> Value {
    let shown = items.len() as i64;
    let mut out = serde_json::json!({ "items": items, "total": total, "limit": limit });
    if total > shown {
        out["note"] = serde_json::json!(format!("Showing {shown} of {total}. {more}"));
    }
    out
}

/// Clamp long-poll wait to the contract's 0..=120 seconds.
pub fn clamp_wait(wait: Option<i64>) -> Duration {
    Duration::from_secs(wait.unwrap_or(0).clamp(0, 120) as u64)
}

// ---------------------------------------------------------------------------
// Scan-shaped reads: off the async runtime, onto the blocking pool.

/// Run a scan-shaped store read on tokio's blocking pool.
///
/// Store calls are synchronous and hold a connection for their duration. For a
/// point read that is a few microseconds and not worth a thread hop, but the
/// endpoints that walk whole tables — `/v1/export`, `/v1/metrics`, a project
/// roadmap, a dep graph — run for as long as the database is big, and on an
/// async worker thread each one occupies a thread the runtime has only
/// `num_cpus` of.
///
/// The read-only connections behind `Store::with_conn` are what keep such a scan
/// from stalling *writers*; this is what keeps it from stalling the *runtime*.
pub async fn blocking_read<T, F>(f: F) -> ApiResult<T>
where
    F: FnOnce() -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        ApiError::internal(format!(
            "read task failed before it could answer: {e}. Retry the request."
        ))
    })?
}

// ---------------------------------------------------------------------------
// Long-poll: re-check `check` after every store mutation until the deadline.

pub async fn long_poll<T>(
    state: &AppState,
    wait: Duration,
    mut check: impl FnMut() -> ApiResult<Option<T>>,
) -> ApiResult<Option<T>> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        // Register interest before checking so a mutation committed between
        // check and await still wakes us.
        let notified = state.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        if let Some(v) = check()? {
            return Ok(Some(v));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::select! {
            _ = &mut notified => {}
            _ = tokio::time::sleep_until(deadline) => {
                return check();
            }
        }
    }
}
