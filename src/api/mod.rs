//! HTTP handlers. Parsing is done by hand from `serde_json::Value` so that
//! malformed input gets teaching errors, not bare 400s.

pub mod claims;
pub mod events;
pub mod export;
pub mod metrics;
pub mod projects;
pub mod questions;
pub mod shares;
pub mod tickets;
pub mod tokens;
pub mod transition;

use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::Json;
use serde_json::Value;
use std::time::Duration;

pub async fn healthz() -> Json<Value> {
    Json(serde_json::json!({ "status": "ok", "version": crate::server::VERSION }))
}

/// Read-only kanban board: a self-contained single-page app that talks to the
/// same-origin `/v1` API with a token the viewer supplies in the browser. The
/// page itself is unauthenticated (all data fetches carry the Bearer token);
/// serving static HTML leaks nothing the API does not already guard.
pub async fn board() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../board.html"))
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
