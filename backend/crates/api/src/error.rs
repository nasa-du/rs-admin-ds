//! RFC 7807 problem details and the API error type.

use axum::body::Body;
use axum::http::{
    HeaderMap, HeaderName, HeaderValue, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
};
use axum::response::{IntoResponse, Response};
use rs_admin_domain::ErrorCode;
use serde::Serialize;
use serde_json::Value;

/// Content type required for every error response.
pub const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";

/// A field-level validation error (JSON Pointer).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct FieldError {
    pub pointer: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// RFC 7807 problem document used for every API error.
#[derive(Clone, Debug, Serialize)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub type_uri: String,
    pub code: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
    pub instance: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<FieldError>>,
}

impl ProblemDetails {
    pub fn for_code(
        code: ErrorCode,
        instance: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            type_uri: code.type_uri(),
            code: code.as_str().to_owned(),
            title: code.title().to_owned(),
            status: code.status(),
            detail: code.detail().to_owned(),
            instance: instance.into(),
            request_id: request_id.into(),
            errors: None,
        }
    }
}

/// An API error. Handlers return this to emit a problem document; the problem
/// normalizer fills in `instance` and `request_id` from the request context.
#[derive(Debug)]
pub struct ApiError {
    code: ErrorCode,
    detail: Option<String>,
    errors: Option<Vec<FieldError>>,
    headers: HeaderMap,
}

impl ApiError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            detail: None,
            errors: None,
            headers: HeaderMap::new(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_errors(mut self, errors: Vec<FieldError>) -> Self {
        self.errors = Some(errors);
        self
    }

    /// Attach a response header (for example `WWW-Authenticate` or
    /// `Retry-After`).
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut details = ProblemDetails::for_code(self.code, "", "");
        details.detail = self.detail.unwrap_or_else(|| self.code.detail().to_owned());
        details.errors = self.errors;

        let status =
            StatusCode::from_u16(details.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_vec(&details).unwrap_or_default();
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_CONTENT_TYPE));
        response.headers_mut().remove(CONTENT_LENGTH);
        response.headers_mut().extend(self.headers);
        response.extensions_mut().insert(details);
        response.extensions_mut().insert(self.code);
        response
    }
}
