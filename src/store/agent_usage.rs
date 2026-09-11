//! Shared, bounded telemetry for both worker queues. Call only after lease authorization.
use crate::{
    error::{ApiError, ApiResult},
    ids::now_ms,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
pub fn save(conn: &Connection, id: &str, telemetry: Option<&Value>) -> ApiResult<()> {
    let Some(value) = telemetry else {
        return Ok(());
    };
    let invalid = || ApiError::validation("validation.agent_usage", "Invalid agent telemetry.");
    let object = value.as_object().ok_or_else(invalid)?;
    for (key, val) in object {
        match key.as_str() {
            "usage" => {
                let usage = val.as_object().ok_or_else(invalid)?;
                let keys = [
                    "input_tokens",
                    "cached_input_tokens",
                    "output_tokens",
                    "reasoning_output_tokens",
                    "total_tokens",
                ];
                if usage.len() != keys.len()
                    || keys.iter().any(|k| {
                        !usage
                            .get(*k)
                            .is_some_and(|v| v.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991))
                    })
                {
                    return Err(invalid());
                }
            }
            "thread_id" | "turn_id" | "model" => {
                if !val
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 200)
                {
                    return Err(invalid());
                }
            }
            "phase" => {
                if !val
                    .as_str()
                    .is_some_and(|s| matches!(s, "preparing_source" | "drafting" | "publishing"))
                {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
    }
    let mut merged = value.clone();
    let previous: Option<String> = conn
        .query_row(
            "SELECT telemetry FROM agent_run_usage WHERE job=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(previous) = previous.and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        if let Some(usage) = merged.get_mut("usage").and_then(Value::as_object_mut) {
            for (key, count) in usage.iter_mut() {
                *count = json!(count
                    .as_u64()
                    .unwrap()
                    .max(previous["usage"][key].as_u64().unwrap_or(0)));
            }
        }
    }
    conn.execute("INSERT INTO agent_run_usage(job,telemetry,updated_at) VALUES(?1,?2,?3) ON CONFLICT(job) DO UPDATE SET telemetry=json_patch(telemetry,excluded.telemetry),updated_at=excluded.updated_at", params![id,merged.to_string(),now_ms()])?;
    Ok(())
}
pub fn attach(conn: &Connection, value: &mut Value) -> ApiResult<()> {
    let row: Option<(String, i64, Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT telemetry,updated_at,started_at,finished_at FROM agent_run_usage WHERE job=?1",
            [value["id"].as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    value["telemetry"] = row
        .map(|(s, at, start, end)| {
            value["started_at"] = json!(start);
            if end.is_some() {
                value["finished_at"] = json!(end);
            }
            let mut v: Value = serde_json::from_str(&s).unwrap_or(json!({}));
            v["updated_at"] = json!(at);
            v
        })
        .unwrap_or(Value::Null);
    Ok(())
}
