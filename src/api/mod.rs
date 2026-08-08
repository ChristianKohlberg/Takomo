//! HTTP handlers. Parsing is done by hand from `serde_json::Value` so that
//! malformed input gets teaching errors, not bare 400s.

pub mod checklist;
pub mod claims;
pub mod events;
pub mod export;
pub mod initiatives;
pub mod metrics;
pub mod oauth;
pub mod projects;
pub mod questions;
pub mod schedules;
pub mod shares;
pub mod tags;
pub mod tickets;
pub mod tokens;
pub mod transition;

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

/// Serve a self-contained HTML app with defense-in-depth headers. These pages
/// hold the viewer's bearer token in `localStorage`, so a strict CSP (no
/// external origins; inline JS/CSS are bundled, hence `'unsafe-inline'`) keeps
/// any future injection from exfiltrating it, and `frame-ancestors`/
/// `X-Frame-Options` block clickjacking of the board.
fn secure_html(body: &'static str) -> impl axum::response::IntoResponse {
    use axum::http::header;
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'self'; script-src 'self' 'unsafe-inline'; \
                 style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                 connect-src 'self'; base-uri 'none'; form-action 'none'; \
                 frame-ancestors 'none'",
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
/// PORTED (phase 4 of 4 — the last). `/board` serves three audiences from one
/// route: the board itself, a read-only `#s=` share, and a single-use `#a=`
/// answer link for an outside expert. All three come out of the `web/` build.
static BOARD_HTML: &str = include_str!("../../web/dist/board.html");
/// PORTED (phase 3 of 4). The ask-a-human inbox, from the `web/` build.
static INBOX_HTML: &str = include_str!("../../web/dist/inbox.html");
/// PORTED (phase 1 of 4). `/initiatives` is served from the `web/` build, not
/// from a hand-written page in `src/`.
///
/// It is still ONE self-contained document — `vite-plugin-singlefile` inlines
/// every script, style and asset — so this stays an `include_str!` and the
/// binary needs no static-file handler, no second request, and no change to the
/// CSP the page is already served under.
///
/// `web/dist/` is committed for exactly this reason: `cargo build --release`
/// must not require node. The CI gate rebuilds it and diffs, so the committed
/// document cannot drift from `web/src/`.
///
/// No `with_spa_common` here: the shared markdown renderer is an ordinary module
/// in the web build (`web/src/lib/markdown.ts`), so there is no marker to splice.
static INITIATIVES_HTML: &str = include_str!("../../web/dist/initiatives.html");
/// PORTED (phase 2 of 4). Same arrangement as `/initiatives` above: one
/// self-contained document out of the `web/` build, `include_str!`'d from
/// committed output so `cargo build --release` stays node-free.
static SCHEDULES_HTML: &str = include_str!("../../web/dist/schedules.html");

pub async fn board() -> impl axum::response::IntoResponse {
    secure_html(BOARD_HTML)
}

/// Ask-a-human inbox: a self-contained email-style page (folder rail, question
/// list, reading/answer pane) served at `/inbox`. Like `/board` it is
/// unauthenticated static HTML; every data fetch carries the viewer's bearer
/// token, so serving it leaks nothing the API does not already guard.
pub async fn inbox() -> impl axum::response::IntoResponse {
    secure_html(INBOX_HTML)
}

/// Initiatives: a self-contained page for the ideas a fleet is nurturing — a
/// list with each collection's rollup, one initiative's entries in full, and the
/// composer that appends to it. Like `/board` and `/inbox` it is unauthenticated
/// static HTML; every data fetch carries the viewer's bearer token.
///
/// It is the one SPA that WRITES, which is why `/v1/initiatives` grew POST and
/// PATCH handlers: an initiative is fed by people as well as agents, and a browser
/// cannot call an MCP tool.
/// Named `initiatives_page` rather than `initiatives`: the sibling module
/// `crate::api::initiatives` holds the JSON handlers, and while Rust would let a
/// function share that name (different namespace), a reader should not have to
/// know that to tell which one a call site means.
pub async fn initiatives_page() -> impl axum::response::IntoResponse {
    secure_html(INITIATIVES_HTML)
}

/// The takomo mark ("tako" = octopus) as an SVG favicon, served at both
/// `/favicon.svg` and `/favicon.ico`. Both surfaces link `/favicon.svg`
/// explicitly; the `.ico` route catches the bare request legacy browsers make
/// on their own so it never 404s. Static, unauthenticated, leaks nothing.
/// GET /schedules — the recurrence page.
///
/// Rows, not columns, and that is the whole design decision: the board sorts by
/// state, but a schedule's content is a *history*, so forcing cadences into
/// columns would throw away the axis that carries the meaning. It shares the
/// board's header, palette, mono identifiers and DE/EN tables, and none of its
/// grid.
pub async fn schedules_page() -> impl axum::response::IntoResponse {
    secure_html(SCHEDULES_HTML)
}

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
