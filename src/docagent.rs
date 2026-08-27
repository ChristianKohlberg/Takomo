//! Turning an instruction into a set of proposed ops — the one place Takomo
//! calls a language model.
//!
//! ## This is a deliberate exception, and it should stay one
//!
//! Everywhere else in this repo the division is **"Takomo stores, the agent
//! computes"**: Checklist does not generate cases, the roadmap does not decide
//! what to build, and no store call has ever depended on a model being reachable.
//! That rule is what keeps this server predictable, cheap to run, and testable
//! without a network.
//!
//! The prompt bar breaks it, on purpose. The alternative was for a person typing
//! "tighten this paragraph" to file a request that sat until some fleet agent
//! happened to look at that document — which is not a feature anyone would use.
//! Closing the loop needed the server to make the call.
//!
//! Three things keep the exception contained:
//!
//! - **It is OFF unless configured.** No key, no route: `POST /v1/documents/{id}/run`
//!   answers a teaching 503 and everything else works exactly as before. A
//!   deployment that never sets `TAKOMO_TENSORX_API_KEY` is the server this repo
//!   documents everywhere else.
//! - **Nothing it produces is trusted.** The model's answer goes through the same
//!   `validate_ops` a fleet agent's does — block ids checked against the live
//!   document, scope enforced, unusable ops dropped and reported. It has no
//!   privileged path.
//! - **Nothing it produces is live text.** It writes a proposal, exactly like an
//!   agent over MCP. A person still accepts or rejects it.
//!
//! ## The op schema is the whole design
//!
//! The model is constrained by JSON Schema to answer with **operations against
//! block ids**, never with a document. That constraint is not a formatting
//! preference: a whole-document answer would overwrite whatever was typed during
//! the seconds the model spent thinking, and turn a one-word fix into a diff
//! nobody can review.
//!
//! ## The anti-fabrication rules are load-bearing
//!
//! The prototype measured this: on the same task, one model **invented
//! statistics** that were nowhere in the document. A fabricated number that reads
//! well is the worst failure mode a document curator has, because it is the one a
//! reviewer is least likely to catch. The two rules that fixed it — never invent
//! facts, and each block stands alone — are carried over verbatim, and should not
//! be trimmed for brevity.

use crate::error::{ApiError, ApiResult};
use serde_json::{json, Value};

/// Where TensorX lives, unless overridden.
const DEFAULT_BASE_URL: &str = "https://api.tensorx.ai/v1";

/// The default model.
///
/// Measured in the prototype at ~1–1.8s on a "tighten this paragraph" task, with
/// no fabrication under the hardened prompt, and — the property that actually
/// matters for a curator — it returns **zero ops** when the document already
/// satisfies the request rather than inventing a change to justify itself.
const DEFAULT_MODEL: &str = "deepseek/deepseek-v4-flash-0731";

/// Cap on an instruction. A prompt bar is for a sentence.
pub const MAX_INSTRUCTION: usize = 2000;

/// How long to wait for the model before giving up.
///
/// Generous for a flash model and finite because this is an inbound HTTP request
/// somebody is watching: a run that hangs should say so, not spin.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// The document-agent's configuration. `None` means the feature is off.
#[derive(Debug, Clone)]
pub struct DocAgentConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

/// Read the configuration from the environment, and produce the startup line.
///
/// Follows `resolve_oauth`: an unusable value turns the feature off and says so
/// loudly rather than stopping the server. Nothing else in Takomo depends on a
/// model being reachable, so a broken key must not cost an operator their ticket
/// store.
pub fn resolve(
    key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
) -> (Option<DocAgentConfig>, String) {
    let Some(api_key) = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) else {
        return (
            None,
            "doc agent: off (set TAKOMO_TENSORX_API_KEY to enable the /documents prompt bar)"
                .to_string(),
        );
    };

    let base_url = base_url
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

    if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
        return (
            None,
            format!("doc agent: OFF — TAKOMO_TENSORX_BASE_URL '{base_url}' is not an http(s) URL"),
        );
    }

    let model = model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    let line = format!("doc agent: on ({model} via {base_url})");
    (
        Some(DocAgentConfig {
            api_key,
            base_url,
            model,
        }),
        line,
    )
}

/// What the model is told. Kept apart from the transport so that changing
/// provider cannot silently change behaviour.
const SYSTEM: &str = r#"You curate a live collaborative document. A human may be editing it at the same moment you are.

The document is given to you as markdown. Each top-level block is preceded by an HTML comment holding its stable id, for example:

<!-- blk_7f3a -->
## Pricing

Return a set of ops that reference those ids. Rules:

- Touch only the blocks that need to change. Every block you do not name is left byte-for-byte alone, which is what keeps a concurrent human editor's work safe. A whole-document rewrite is never the right answer.
- "markdown" is the full new content for the op's target, written as ordinary markdown. Do not include the id comment in it.
- Use "replace" to rewrite a block, "insert_after" to add a block after it, and "delete" to remove one.
- Never invent facts, figures, names, dates, or quotations. You are editing someone's document, not writing a plausible-sounding one: a fabricated number that reads well is worse than a vague sentence that is true. If the request can only be satisfied with information the document does not contain, do the part you can, and say what is missing in the summary.
- Each block stands alone. Do not pull content from a neighbouring block into the one you are rewriting — the reader still has the other block.
- Deliver the change the user asked for, at the scope they asked for. Do not opportunistically fix, tidy, or restructure prose you were not asked about. If you think the request is mistaken, say so in the summary and still do what was asked.
- If the document already satisfies the request, return an empty ops array and say so in the summary.
- "rationale" is one short clause shown next to the proposed change. Say why, not what.
- The summary is one sentence for the document owner. Lead with what changed, and write it in the language the document is written in.

Some of these documents describe software that is meant to be built. Two terms have a specific meaning in them, and you should honour it rather than smooth it away:

- A "Zusage" (commitment) is a sentence you could check the software against. "Wenn jemand absagt, passiert nichts automatisch" is one; "Proben scheitern an Logistik" is not, however true it is. When asked to turn something into a Zusage, make it checkable — name who acts and what observably happens — and do not invent the behaviour if the document does not say.
- An "offene Frage" (open question) is a decision nobody has taken yet. Leave it open; do not resolve it by picking an answer."#;

/// The JSON Schema the answer is constrained to.
fn plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": {
                "type": "string",
                "description": "One sentence for the document owner, in the document's language."
            },
            "ops": {
                "type": "array",
                "description": "Edits to apply, in document order. Empty if no change is warranted.",
                "items": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["replace", "insert_after", "delete"] },
                        "id": {
                            "type": "string",
                            "description": "The blk_ id this op targets. Always required."
                        },
                        "markdown": {
                            "type": ["string", "null"],
                            "description": "New content as markdown, one or more top-level blocks. Null for delete."
                        },
                        "rationale": {
                            "type": ["string", "null"],
                            "description": "One short clause shown next to the change. Why, not what."
                        }
                    },
                    "required": ["op", "id", "markdown", "rationale"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["summary", "ops"],
        "additionalProperties": false
    })
}

/// What came back from one run.
pub struct Plan {
    pub summary: String,
    /// Raw ops, still to go through `docprops::validate_ops`.
    pub ops: Value,
}

/// Ask the model for a plan.
pub async fn run(
    cfg: &DocAgentConfig,
    instruction: &str,
    annotated: &str,
    scope: Option<&[String]>,
    model_override: Option<&str>,
) -> ApiResult<Plan> {
    let model = model_override
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(&cfg.model)
        .to_string();

    // The scope sits next to the instruction rather than in the system prompt:
    // it is part of what is being asked and it changes every run.
    let asked = match scope {
        Some(ids) if !ids.is_empty() => format!(
            "{instruction}\n\nRestrict yourself strictly to these blocks: {}. \
             Ops targeting any other block are discarded.",
            ids.join(", ")
        ),
        _ => instruction.to_string(),
    };
    let user = format!("{asked}\n\n---\n\nThe document:\n\n{annotated}");

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| upstream(format!("could not build an HTTP client: {e}")))?;

    // Structured outputs first. Not every open-weight model accepts a
    // `json_schema` response format, so a refusal falls back to `json_object`
    // with the schema in the prompt — the answer is validated either way, so the
    // fallback costs strictness, not safety.
    let strict = request_body(&model, &user, true);
    let mut text = post(&client, cfg, &strict).await;

    if text.is_err() {
        let loose = request_body(&model, &user, false);
        text = post(&client, cfg, &loose).await;
    }
    let text = text?;

    let parsed = parse_plan(&text).ok_or_else(|| {
        upstream(format!(
            "the model's answer was not a usable plan. First 300 characters: {}",
            text.chars().take(300).collect::<String>()
        ))
    })?;

    Ok(parsed)
}

fn request_body(model: &str, user: &str, strict: bool) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": 8000,
        "messages": [
            { "role": "system", "content": SYSTEM },
            { "role": "user", "content": user },
        ],
    });
    if strict {
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": { "name": "plan", "strict": true, "schema": plan_schema() },
        });
    } else {
        body["response_format"] = json!({ "type": "json_object" });
        body["messages"][0]["content"] = json!(format!(
            "{SYSTEM}\n\nAnswer with JSON matching exactly this schema:\n{}",
            plan_schema()
        ));
    }
    body
}

async fn post(client: &reqwest::Client, cfg: &DocAgentConfig, body: &Value) -> ApiResult<String> {
    let resp = client
        .post(format!("{}/chat/completions", cfg.base_url))
        .bearer_auth(&cfg.api_key)
        .json(body)
        .send()
        .await
        .map_err(|e| upstream(format!("the model provider could not be reached: {e}")))?;

    let status = resp.status();
    let payload = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // The provider's own message is passed through: it is the only thing that
        // says whether this is a bad key, an unknown model or a rate limit, and
        // re-wording it would throw that away.
        return Err(upstream(format!(
            "the model provider answered {status}: {}",
            payload.chars().take(400).collect::<String>()
        )));
    }

    let parsed: Value = serde_json::from_str(&payload)
        .map_err(|e| upstream(format!("the model provider's reply was not JSON: {e}")))?;
    parsed["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| upstream("the model provider's reply carried no content".to_string()))
}

/// Pull a plan out of the answer.
///
/// A model that wrapped its JSON in prose or a fence still produced a usable
/// plan; refusing to parse that would fail a run over a formatting nit.
fn parse_plan(text: &str) -> Option<Plan> {
    let direct = serde_json::from_str::<Value>(text.trim()).ok();
    let value = direct.or_else(|| {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        serde_json::from_str::<Value>(text.get(start..=end)?).ok()
    })?;

    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("No summary provided.")
        .to_string();

    // `markdown: null` on a non-delete op is the model getting the schema right
    // but the content wrong; `validate_ops` drops those and reports them, so
    // they are passed through rather than filtered silently here.
    let ops = value.get("ops").cloned().unwrap_or_else(|| json!([]));
    Some(Plan { summary, ops })
}

fn upstream(message: String) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::BAD_GATEWAY,
        "document.agent_failed",
        message,
    )
    .remedy(
        "Nothing was written to the document. Try again, or edit it by hand — the prompt bar \
         is a convenience, not the only way in."
            .to_string(),
    )
}

/// The error for a server with no model configured.
pub fn not_configured() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "document.agent_not_configured",
        "This server has no document agent configured, so the prompt bar cannot run.",
    )
    .remedy(
        "Set TAKOMO_TENSORX_API_KEY (and optionally TAKOMO_TENSORX_BASE_URL and \
         TAKOMO_DOC_MODEL) and restart. Everything else on /documents works without it — \
         an agent connected over MCP can still propose changes."
            .to_string(),
    )
}
