//! Two embedding wire protocols, with validation before vectors enter the index.
use crate::{
    error::{ApiError, ApiResult},
    store::search::EmbeddingConfig,
};
use serde_json::{json, Value};
use std::sync::OnceLock;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn client() -> ApiResult<&'static reqwest::Client> {
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let built = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiError::internal("Cannot initialize embedding client"))?;
    Ok(CLIENT.get_or_init(|| built))
}

pub async fn embed(
    config: &EmbeddingConfig,
    key: &str,
    texts: &[String],
    query: bool,
) -> ApiResult<Vec<Vec<f32>>> {
    if key.is_empty() {
        return Err(ApiError::validation(
            "embeddings.unconfigured",
            "Embedding provider is not configured",
        ));
    }
    let mut body = json!({"model":config.model,"input":texts});
    if config.provider == "voyage" {
        body["input_type"] = json!(if query { "query" } else { "document" });
        body["output_dimension"] = json!(config.dimensions);
        body["truncation"] = json!(false);
    } else {
        body["dimensions"] = json!(config.dimensions);
        body["encoding_format"] = json!("float");
    }
    let mut response = client()?
        .post(&config.endpoint)
        .timeout(std::time::Duration::from_secs(10))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|_| ApiError::internal("Embedding provider unavailable"))?;
    if !response.status().is_success() {
        return Err(ApiError::internal(format!(
            "Embedding provider returned HTTP {}",
            response.status().as_u16()
        )));
    }
    if response.content_length().is_some_and(|n| n > 8_000_000) {
        return Err(ApiError::internal("Embedding response too large"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::internal("Invalid embedding response"))?
    {
        if bytes.len() + chunk.len() > 8_000_000 {
            return Err(ApiError::internal("Embedding response too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::internal("Invalid embedding response"))?;
    let data = body["data"]
        .as_array()
        .ok_or_else(|| ApiError::internal("Embedding response missing data"))?;
    if data.len() != texts.len() {
        return Err(ApiError::internal("Embedding response count mismatch"));
    }
    let mut vectors = vec![None; texts.len()];
    for item in data {
        let index = item["index"]
            .as_u64()
            .ok_or_else(|| ApiError::internal("Embedding response missing index"))?
            as usize;
        if index >= vectors.len() || vectors[index].is_some() {
            return Err(ApiError::internal("Embedding response invalid index"));
        }
        let vector: Vec<f32> = serde_json::from_value(item["embedding"].clone())
            .map_err(|_| ApiError::internal("Invalid embedding vector"))?;
        if vector.len() != config.dimensions
            || vector.iter().any(|v| !v.is_finite())
            || vector.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() == 0.0
        {
            return Err(ApiError::internal(
                "Embedding vector dimensions or values invalid",
            ));
        }
        vectors[index] = Some(vector);
    }
    vectors
        .into_iter()
        .map(|v| v.ok_or_else(|| ApiError::internal("Missing embedding vector")))
        .collect()
}
