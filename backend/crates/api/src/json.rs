//! JSON request extractor with the API's error mapping.
//!
//! Axum's built-in `Json` maps every serde data error to 422. The API contract
//! distinguishes syntax errors (`invalid_json`), unknown fields
//! (`unknown_field`) and field validation (`validation_failed` with JSON
//! pointers), so authentication requests use this extractor instead.

use crate::error::{ApiError, FieldError};
use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use rs_admin_domain::ErrorCode;
use serde::de::DeserializeOwned;
use serde_json::error::Category;

/// A JSON body parsed under the API's problem-details contract.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        if !has_json_content_type(request.headers()) {
            return Err(ApiError::new(ErrorCode::UnsupportedMediaType));
        }

        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| {
                if rejection.into_response().status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::new(ErrorCode::PayloadTooLarge)
                } else {
                    ApiError::new(ErrorCode::InvalidJson)
                }
            })?;

        serde_json::from_slice(&bytes).map(Self).map_err(json_error)
    }
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let media_type = value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            media_type == "application/json"
                || (media_type.starts_with("application/") && media_type.ends_with("+json"))
        })
}

fn json_error(error: serde_json::Error) -> ApiError {
    match error.classify() {
        Category::Syntax | Category::Eof | Category::Io => ApiError::new(ErrorCode::InvalidJson),
        Category::Data => {
            let message = error.to_string();
            if quoted_field(&message, "unknown field").is_some() {
                return ApiError::new(ErrorCode::UnknownField);
            }
            if let Some(field) = quoted_field(&message, "missing field") {
                return ApiError::new(ErrorCode::ValidationFailed).with_errors(vec![FieldError {
                    pointer: format!("/{field}"),
                    code: "required".to_owned(),
                    params: None,
                }]);
            }
            ApiError::new(ErrorCode::ValidationFailed).with_errors(Vec::new())
        }
    }
}

/// Extract the field name from serde messages such as
/// ``unknown field `identifier` ``.
fn quoted_field<'a>(message: &'a str, prefix: &str) -> Option<&'a str> {
    message
        .strip_prefix(prefix)?
        .split('`')
        .nth(1)
        .filter(|field| !field.is_empty())
}
