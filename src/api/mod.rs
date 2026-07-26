//! HTTP handlers. Parsing is done by hand from `serde_json::Value` so that
//! malformed input gets teaching errors, not bare 400s.

pub mod claims;
pub mod events;
pub mod export;
pub mod metrics;
pub mod projects;
pub mod questions;
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
pub async fn board() -> impl axum::response::IntoResponse {
    secure_html(include_str!("../board.html"))
}

/// Ask-a-human inbox: a self-contained email-style page (folder rail, question
/// list, reading/answer pane) served at `/inbox`. Like `/board` it is
/// unauthenticated static HTML; every data fetch carries the viewer's bearer
/// token, so serving it leaks nothing the API does not already guard.
pub async fn inbox() -> impl axum::response::IntoResponse {
    secure_html(include_str!("../inbox.html"))
}

/// The takomo mark ("tako" = octopus) as an SVG favicon, served at both
/// `/favicon.svg` and `/favicon.ico`. Both surfaces link `/favicon.svg`
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

/// Clamp long-poll wait to the contract's 0..=120 seconds.
pub fn clamp_wait(wait: Option<i64>) -> Duration {
    Duration::from_secs(wait.unwrap_or(0).clamp(0, 120) as u64)
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
