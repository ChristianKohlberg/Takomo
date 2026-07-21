//! Event log endpoints: cursor read with long-poll, and the SSE doorbell.

use super::{clamp_wait, first, long_poll, parse_i64_param, query_pairs};
use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::server::AppState;
use crate::store::EventFilter;
use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::{Extension, Json};
use futures_core::Stream;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;

fn parse_filter(ctx: &AuthCtx, pairs: &[(String, String)]) -> ApiResult<EventFilter> {
    if let Some(p) = first(pairs, "project") {
        ctx.require_project(p)?;
    }
    Ok(EventFilter {
        project: first(pairs, "project").map(str::to_string),
        ticket: first(pairs, "ticket").map(str::to_string),
        kinds: first(pairs, "kind")
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        allowed_projects: ctx.allowed_projects_vec(),
    })
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());
    let since = parse_i64_param(&pairs, "since")?.ok_or_else(|| {
        ApiError::bad_request(
            "validation.query",
            "Query parameter 'since' is required: the last global_seq you processed, or 0 to read from the start. The response's 'cursor' is your next 'since'.",
        )
    })?;
    let filter = parse_filter(&ctx, &pairs)?;
    let wait = clamp_wait(parse_i64_param(&pairs, "wait")?);
    let limit = parse_i64_param(&pairs, "limit")?
        .unwrap_or(200)
        .clamp(1, 1000);

    // Long-poll: return as soon as at least one matching event exists.
    let result = long_poll(&state, wait, || {
        let (events, cursor) = state.store.events_since(since, &filter, limit)?;
        if events.is_empty() {
            Ok(None)
        } else {
            Ok(Some((events, cursor)))
        }
    })
    .await?;

    let (events, cursor) = result.unwrap_or_else(|| (Vec::new(), since));
    Ok(Json(json!({
        "events": events.iter().map(|e| e.to_json()).collect::<Vec<_>>(),
        "cursor": cursor,
    })))
}

/// SSE stream. Each event's `id:` is its global_seq so `Last-Event-ID`
/// resumes; the stream is a doorbell — consumers reconcile via GET /events.
pub async fn stream(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> ApiResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    ctx.require_scope("read")?;
    let pairs = query_pairs(raw.as_deref());
    let filter = parse_filter(&ctx, &pairs)?;
    let last_event_id = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok());
    let mut cursor = last_event_id
        .or(parse_i64_param(&pairs, "since")?)
        .unwrap_or(0);

    let stream = async_stream::stream! {
        loop {
            // Register before checking so no commit slips between poll and wait.
            let notified = state.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            match state.store.events_since(cursor, &filter, 500) {
                Ok((events, new_cursor)) if !events.is_empty() => {
                    cursor = new_cursor;
                    for e in events {
                        yield Ok(SseEvent::default()
                            .id(e.seq.to_string())
                            .event(e.kind.clone())
                            .data(e.to_json().to_string()));
                    }
                }
                Ok(_) => {
                    notified.await;
                }
                Err(err) => {
                    yield Ok(SseEvent::default()
                        .event("error")
                        .data(json!({ "code": err.body.code, "message": err.body.message }).to_string()));
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    ))
}
