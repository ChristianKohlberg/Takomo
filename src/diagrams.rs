//! Private, bounded Kroki transport. Diagram source is never logged or persisted here.
use crate::error::{ApiError, ApiResult};
use axum::http::StatusCode;
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, sync::Mutex, time::Duration};
use tokio::sync::Semaphore;

pub const MAX_SOURCE_BYTES: usize = 50_000;
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 128;

#[derive(Default)]
struct Cache {
    entries: VecDeque<([u8; 32], String)>,
    bytes: usize,
}

pub struct DiagramRenderer {
    base: reqwest::Url,
    version: String,
    client: reqwest::Client,
    slots: Semaphore,
    cache: Mutex<Cache>,
}

impl DiagramRenderer {
    pub fn new(base: &str, version: &str) -> Result<Self, String> {
        let mut base =
            reqwest::Url::parse(base).map_err(|_| "TAKOMO_KROKI_URL must be an HTTP(S) URL.")?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(
                "TAKOMO_KROKI_URL must be an HTTP(S) URL without credentials, query or fragment."
                    .into(),
            );
        }
        base.set_path(&format!("{}/", base.path().trim_end_matches('/')));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "Could not initialize the diagram renderer.")?;
        Ok(Self {
            base,
            version: version.to_owned(),
            client,
            slots: Semaphore::new(4),
            cache: Mutex::new(Cache::default()),
        })
    }

    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(url) = std::env::var("TAKOMO_KROKI_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
        else {
            return Ok(None);
        };
        Self::new(
            &url,
            &std::env::var("TAKOMO_KROKI_VERSION").unwrap_or_else(|_| "default".into()),
        )
        .map(Some)
    }

    pub async fn render(&self, engine: &str, source: &str) -> ApiResult<String> {
        // Keep validation here as well as in the HTTP handler: URL path selection
        // must never become an arbitrary endpoint when another caller is added.
        if !matches!(engine, "mermaid" | "plantuml" | "d2") {
            return Err(ApiError::validation(
                "validation.diagram_engine",
                "Choose mermaid, plantuml or d2.",
            ));
        }
        if source.trim().is_empty() || source.len() > MAX_SOURCE_BYTES {
            return Err(ApiError::validation(
                "validation.diagram_source",
                "Diagram source must contain text and be at most 50 KB.",
            ));
        }
        if engine == "d2" && source.contains('@') {
            return Err(ApiError::validation("validation.diagram_source", "D2 imports and at-signs are not supported. Remove every @ character from the source."));
        }
        let mut hash = Sha256::new();
        hash.update(self.version.as_bytes());
        hash.update([0]);
        hash.update(engine.as_bytes());
        hash.update([0]);
        hash.update(source.as_bytes());
        let key: [u8; 32] = hash.finalize().into();
        if let Some(svg) = self
            .cache
            .lock()
            .unwrap()
            .entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, svg)| svg.clone())
        {
            return Ok(svg);
        }
        let _permit = self.slots.try_acquire().map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "diagram.busy",
                "Diagram rendering is busy. Try again shortly.",
            )
        })?;
        let url = self
            .base
            .join(&format!("{engine}/svg"))
            .map_err(|_| upstream_error())?;
        let mut response = self
            .client
            .post(url)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Accept", "image/svg+xml")
            .body(source.to_owned())
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            return Err(if response.status().is_client_error() {
                ApiError::validation("validation.diagram_syntax", "The renderer could not render this diagram. Check the source syntax and selected language.")
            } else {
                upstream_error()
            });
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_OUTPUT_BYTES as u64)
        {
            return Err(output_error());
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if content_type.split(';').next().unwrap_or("").trim() != "image/svg+xml" {
            return Err(upstream_error());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > MAX_OUTPUT_BYTES {
                return Err(output_error());
            }
            bytes.extend_from_slice(&chunk);
        }
        let svg = String::from_utf8(bytes).map_err(|_| upstream_error())?;
        validate_svg(&svg, engine)?;
        let mut cache = self.cache.lock().unwrap();
        // Concurrent renders of identical source may both finish; retain one entry.
        if !cache.entries.iter().any(|(k, _)| *k == key) {
            while cache.entries.len() >= MAX_CACHE_ENTRIES
                || cache.bytes + svg.len() > MAX_CACHE_BYTES
            {
                if let Some((_, old)) = cache.entries.pop_front() {
                    cache.bytes -= old.len();
                }
            }
            cache.bytes += svg.len();
            cache.entries.push_back((key, svg.clone()));
        }
        Ok(svg)
    }
}

// This validates XML and unsupported assets, not a DOM sanitizer. Clients must
// keep SVG in an isolated image. Kroki itself must run with secure settings.
fn validate_svg(svg: &str, engine: &str) -> ApiResult<()> {
    use quick_xml::{events::Event, Reader};
    let mut reader = Reader::from_str(svg);
    let mut depth = 0usize;
    let mut root_seen = false;
    loop {
        let event = reader.read_event().map_err(|_| upstream_error())?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                if depth == 0 {
                    if root_seen || element.name().as_ref() != b"svg" {
                        return Err(upstream_error());
                    }
                    root_seen = true;
                }
                for attr in element.attributes() {
                    let attr = attr.map_err(|_| upstream_error())?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| upstream_error())?;
                    if engine == "d2"
                        && element.local_name().as_ref() == b"image"
                        && attr.key.local_name().as_ref() == b"href"
                        && !value.starts_with('#')
                        && !value.starts_with("data:image/")
                    {
                        return Err(ApiError::validation("validation.diagram_assets", "External images are not supported in D2 diagrams. Remove external image and icon references."));
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(upstream_error)?;
            }
            Event::DocType(_) => return Err(upstream_error()),
            Event::Text(text) if depth == 0 => {
                if !text
                    .decode()
                    .map_err(|_| upstream_error())?
                    .trim()
                    .is_empty()
                {
                    return Err(upstream_error());
                }
            }
            Event::CData(_) | Event::GeneralRef(_) if depth == 0 => return Err(upstream_error()),
            Event::Eof => {
                return if root_seen && depth == 0 {
                    Ok(())
                } else {
                    Err(upstream_error())
                }
            }
            _ => {}
        }
    }
}
fn upstream_error() -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "diagram.unavailable",
        "The diagram renderer is unavailable or returned an invalid result. Try again shortly.",
    )
}
fn output_error() -> ApiError {
    ApiError::validation(
        "validation.diagram_output",
        "The rendered diagram exceeds 2 MiB. Simplify the diagram and try again.",
    )
}
fn transport_error(error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            "diagram.timeout",
            "Diagram rendering took too long. Simplify the diagram or try again.",
        )
    } else {
        upstream_error()
    }
}
