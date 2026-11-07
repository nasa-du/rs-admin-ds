//! Request-id, security-header, problem-normalization and observability
//! middleware. The ordering is defined in `router.rs`.

use crate::error::{ApiError, PROBLEM_CONTENT_TYPE, ProblemDetails};
use axum::body::Body;
use axum::extract::{MatchedPath, Request, State};
use axum::http::{
    HeaderValue, StatusCode,
    header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, WWW_AUTHENTICATE},
};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rs_admin_domain::ErrorCode;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Server-generated request id. Client-supplied `X-Request-Id` is ignored.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestId(pub(crate) Uuid);

/// Cancel the handler future when the request exceeds its total budget.
pub(crate) async fn request_timeout(
    State(timeout): State<Duration>,
    request: Request,
    next: Next,
) -> Response {
    match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => ApiError::new(ErrorCode::ServiceUnavailable).into_response(),
    }
}

/// Generate a UUIDv7 per request, expose it to handlers, and write it back on
/// the response. The client header is removed before the handler runs.
pub(crate) async fn request_id(mut request: Request, next: Next) -> Response {
    request.headers_mut().remove("x-request-id");
    let id = RequestId(Uuid::now_v7());
    request.extensions_mut().insert(id);

    let mut response = next.run(request).await;
    let value = id.0.to_string();
    if let Ok(header) = HeaderValue::from_str(&value) {
        response.headers_mut().insert("x-request-id", header);
    }
    response.extensions_mut().insert(id);
    response
}

/// Stable security headers for every API response.
pub(crate) async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'; base-uri 'none'"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    response
}

/// Rewrite non-problem framework rejections into the closed problem contract
/// and fill `instance`/`request_id` for errors created by handlers.
pub(crate) async fn normalize_problem(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let response = next.run(request).await;
    let request_id = response
        .extensions()
        .get::<RequestId>()
        .map(|id| id.0.to_string())
        .unwrap_or_default();

    if let Some(mut details) = response.extensions().get::<ProblemDetails>().cloned() {
        details.instance = path;
        details.request_id = request_id;
        let status =
            StatusCode::from_u16(details.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut rebuilt = with_problem_body(response, status, &details);
        add_unauthorized_challenge(&mut rebuilt);
        rebuilt.extensions_mut().insert(details);
        return rebuilt;
    }

    let Some(code) = code_for_status(response.status()) else {
        return response;
    };
    let details = ProblemDetails::for_code(code, path, request_id);
    let status = StatusCode::from_u16(code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut rebuilt = with_problem_body(response, status, &details);
    add_unauthorized_challenge(&mut rebuilt);
    rebuilt.extensions_mut().insert(details);
    rebuilt.extensions_mut().insert(code);
    rebuilt
}

/// Every 401 carries the Bearer challenge required by the API contract.
fn add_unauthorized_challenge(response: &mut Response) {
    if response.status() == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .entry(WWW_AUTHENTICATE)
            .or_insert(HeaderValue::from_static("Bearer realm=\"rs-admin\""));
    }
}

fn code_for_status(status: StatusCode) -> Option<ErrorCode> {
    match status {
        StatusCode::BAD_REQUEST => Some(ErrorCode::InvalidJson),
        StatusCode::NOT_FOUND => Some(ErrorCode::NotFound),
        StatusCode::METHOD_NOT_ALLOWED => Some(ErrorCode::MethodNotAllowed),
        StatusCode::PAYLOAD_TOO_LARGE => Some(ErrorCode::PayloadTooLarge),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => Some(ErrorCode::UnsupportedMediaType),
        StatusCode::UNPROCESSABLE_ENTITY => Some(ErrorCode::ValidationFailed),
        StatusCode::INTERNAL_SERVER_ERROR => Some(ErrorCode::InternalError),
        _ => None,
    }
}

fn with_problem_body(response: Response, status: StatusCode, details: &ProblemDetails) -> Response {
    let (mut parts, _) = response.into_parts();
    parts.status = status;
    parts.headers.remove(CONTENT_LENGTH);
    parts
        .headers
        .insert(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_CONTENT_TYPE));
    let body = serde_json::to_vec(details).unwrap_or_default();
    Response::from_parts(parts, Body::from(body))
}

/// Record bounded metrics and one structured log line per request. Only
/// whitelisted fields are emitted; headers, query strings and bodies are
/// never read.
pub(crate) async fn observe(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());

    let response = next.run(request).await;
    let status = response.status();
    let elapsed = started.elapsed();
    let request_id = response
        .extensions()
        .get::<RequestId>()
        .map(|id| id.0.to_string())
        .unwrap_or_default();
    let error_code = response
        .extensions()
        .get::<ErrorCode>()
        .map(|code| code.as_str())
        .unwrap_or("");

    let method_label = metric_method(&method);
    metrics::counter!(
        "http_requests_total",
        "method" => method_label,
        "route" => route.clone(),
        "status" => status.as_u16().to_string()
    )
    .increment(1);
    metrics::histogram!(
        "http_request_duration_seconds",
        "method" => method_label,
        "route" => route.clone()
    )
    .record(elapsed.as_secs_f64());

    tracing::info!(
        request_id = %request_id,
        method = %method,
        matched_path = %route,
        status = status.as_u16(),
        latency_ms = elapsed.as_millis() as u64,
        error_code = error_code,
        "request completed"
    );

    response
}

fn metric_method(method: &axum::http::Method) -> &'static str {
    match *method {
        axum::http::Method::GET => "GET",
        axum::http::Method::POST => "POST",
        axum::http::Method::PUT => "PUT",
        axum::http::Method::PATCH => "PATCH",
        axum::http::Method::DELETE => "DELETE",
        axum::http::Method::HEAD => "HEAD",
        axum::http::Method::OPTIONS => "OPTIONS",
        _ => "OTHER",
    }
}
