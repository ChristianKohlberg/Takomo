//! API error type. Every rejection teaches: stable machine `code`, a `message`
//! written for LLM readers (what is wrong AND what to do), and where one
//! exists, a `remedy` naming the exact next call.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// One allowed transition edge, echoed in transition errors so the caller can
/// see the legal moves without a second request.
#[derive(Debug, Clone, Serialize)]
pub struct AllowedTransition {
    pub to: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
}

/// The wire error body (Error / TransitionError schemas from openapi.yaml).
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_transitions: Option<Vec<AllowedTransition>>,
}

#[derive(Debug)]
pub struct ApiErrorData {
    pub status: StatusCode,
    pub body: ErrorBody,
    /// Extra response headers (e.g. Retry-After on 429).
    pub headers: Vec<(&'static str, String)>,
}

/// Boxed so `Result<T, ApiError>` stays a pointer wide (clippy: result_large_err).
#[derive(Debug)]
pub struct ApiError(Box<ApiErrorData>);

impl std::ops::Deref for ApiError {
    type Target = ApiErrorData;
    fn deref(&self) -> &ApiErrorData {
        &self.0
    }
}

impl std::ops::DerefMut for ApiError {
    fn deref_mut(&mut self) -> &mut ApiErrorData {
        &mut self.0
    }
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        ApiError(Box::new(ApiErrorData {
            status,
            body: ErrorBody {
                code: code.to_string(),
                message: message.into(),
                remedy: None,
                current_version: None,
                details: None,
                current_state: None,
                allowed_transitions: None,
            },
            headers: Vec::new(),
        }))
    }

    pub fn remedy(mut self, remedy: impl Into<String>) -> Self {
        self.body.remedy = Some(remedy.into());
        self
    }

    pub fn current_version(mut self, v: i64) -> Self {
        self.body.current_version = Some(v);
        self
    }

    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.body.details = Some(details);
        self
    }

    pub fn current_state(mut self, state: impl Into<String>) -> Self {
        self.body.current_state = Some(state.into());
        self
    }

    pub fn allowed_transitions(mut self, allowed: Vec<AllowedTransition>) -> Self {
        self.body.allowed_transitions = Some(allowed);
        self
    }

    pub fn header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }

    /// Consume the error, yielding just the message (CLI-facing paths).
    pub fn into_message(self) -> String {
        self.0.body.message
    }

    // Common constructors ---------------------------------------------------

    pub fn not_found(kind: &str, id: &str) -> Self {
        ApiError::new(
            StatusCode::NOT_FOUND,
            &format!("notfound.{kind}"),
            format!("No {kind} with id '{id}' exists. Check the id; list endpoints support search and filters."),
        )
    }

    pub fn validation(code: &str, message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    pub fn conflict(code: &str, message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::CONFLICT, code, message)
    }

    pub fn bad_request(code: &str, message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let data = *self.0;
        let mut resp = (data.status, Json(&data.body)).into_response();
        for (name, value) in &data.headers {
            if let Ok(v) = axum::http::HeaderValue::from_str(value) {
                resp.headers_mut().insert(*name, v);
            }
        }
        resp
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::internal(format!("database error: {e}"))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
