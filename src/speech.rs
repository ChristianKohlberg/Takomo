//! Dictation, and the reason the key never reaches the browser.
//!
//! Speaking is the fastest way to fill a mindmap — a brainstorm arrives faster
//! than it can be typed, and the 280-character node cap exists precisely because
//! a thought at that size is one sentence somebody said. So the map takes
//! dictation.
//!
//! The transcription itself happens in the BROWSER, talking straight to
//! AssemblyAI over a WebSocket: audio does not travel through this server, which
//! keeps a recording out of the request log and off this disk entirely. What the
//! browser cannot have is the account key — a key in a page is a key on every
//! machine that loads it — so this mints a SHORT-LIVED token instead, which is
//! the exchange AssemblyAI provides for exactly this shape.
//!
//! Off unless `TAKOMO_ASSEMBLYAI_API_KEY` is set, and `/v1/whoami` reports
//! `features.voice` so the map can leave the button out rather than offer one
//! that fails. The same shape as the document agent beside it.

use crate::error::{ApiError, ApiResult};

/// How long a minted token is good for.
///
/// Ten minutes: long enough for a dictation session nobody notices, short enough
/// that a token pulled out of a page's network log is worth little. The browser
/// mints another when it needs one.
pub const TOKEN_TTL_SECONDS: u32 = 600;

const TOKEN_URL: &str = "https://streaming.assemblyai.com/v3/token";

#[derive(Clone)]
pub struct SpeechConfig {
    api_key: String,
}

impl SpeechConfig {
    /// `None` turns dictation off, which is the default and not an error.
    pub fn from_env(key: Option<String>) -> Option<Self> {
        let key = key?;
        let key = key.trim().to_string();
        if key.is_empty() {
            return None;
        }
        Some(Self { api_key: key })
    }

    /// Exchange the account key for a token the browser may hold.
    pub async fn mint(&self) -> ApiResult<String> {
        let client = reqwest::Client::new();
        let res = client
            .get(TOKEN_URL)
            .query(&[("expires_in_seconds", TOKEN_TTL_SECONDS.to_string())])
            .header("authorization", &self.api_key)
            .send()
            .await
            .map_err(|e| upstream(&format!("could not reach the speech provider: {e}")))?;

        let status = res.status();
        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|e| upstream(&format!("the speech provider's answer did not parse: {e}")))?;
        if !status.is_success() {
            // The provider's own message is more useful than anything invented
            // here — a rejected key says so precisely.
            let detail = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("no detail given");
            return Err(upstream(&format!(
                "the speech provider refused the key ({status}): {detail}"
            )));
        }
        body.get("token")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| upstream("the speech provider returned no token"))
    }
}

/// Dictation is configured off. Not an error in the operator's sense — a
/// deployment without a key is a normal deployment — so it says what to set.
pub fn not_configured() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "speech.not_configured",
        "Dictation is not configured on this server.".to_string(),
    )
    .remedy(
        "Set TAKOMO_ASSEMBLYAI_API_KEY and restart. `/v1/whoami` reports \
         `features.voice`, so a page can leave the button out rather than offer one \
         that cannot work."
            .to_string(),
    )
}

fn upstream(message: &str) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::BAD_GATEWAY,
        "speech.upstream",
        message.to_string(),
    )
    .remedy("Try again; if it persists the key or the provider is the problem.".to_string())
}
