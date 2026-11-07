//! Router assembly, health endpoints and the common middleware stack.

use crate::error::ApiError;
use crate::middleware;
use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rs_admin_domain::ErrorCode;
use rs_admin_store::{Database, Readiness};
use serde_json::json;
use std::time::Duration;
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;

/// Maximum JSON request body for regular endpoints.
pub const MAX_JSON_BODY_BYTES: usize = 64 * 1024;

/// Total request timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum in-flight requests before backpressure is applied.
pub const MAX_IN_FLIGHT_REQUESTS: usize = 100;

/// Readiness probes must answer within this budget.
pub const READINESS_TIMEOUT: Duration = Duration::from_secs(2);

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub database: Database,
}

impl AppState {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
}

/// Build the production router.
pub fn build_router(state: AppState) -> Router {
    with_common_middleware(
        Router::new()
            .route("/health/live", get(health_live))
            .route("/health/ready", get(health_ready))
            .fallback(not_found)
            .with_state(state),
    )
}

/// Apply the common middleware stack. Exposed so integration tests can wrap
/// additional routes and exercise the same contract.
pub fn with_common_middleware(router: Router) -> Router {
    with_common_middleware_and_timeout(router, REQUEST_TIMEOUT)
}

/// Same stack with an explicit request timeout. Exposed for timeout tests.
#[doc(hidden)]
pub fn with_common_middleware_and_timeout(router: Router, timeout: Duration) -> Router {
    router.layer(
        ServiceBuilder::new()
            .layer(from_fn(middleware::observe))
            .layer(from_fn(middleware::security_headers))
            .layer(from_fn(middleware::normalize_problem))
            .layer(DefaultBodyLimit::max(MAX_JSON_BODY_BYTES))
            .layer(ConcurrencyLimitLayer::new(MAX_IN_FLIGHT_REQUESTS))
            .layer(from_fn(middleware::request_id))
            .layer(from_fn_with_state(timeout, middleware::request_timeout)),
    )
}

async fn health_live() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "live" })))
}

async fn health_ready(State(state): State<AppState>) -> Response {
    match state.database.check_ready(READINESS_TIMEOUT).await {
        Readiness::Ready => (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response(),
        Readiness::Unready(reason) => {
            tracing::warn!(reason = ?reason, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "unready" })),
            )
                .into_response()
        }
    }
}

async fn not_found() -> ApiError {
    ApiError::new(ErrorCode::NotFound)
}
