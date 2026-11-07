//! HTTP API surface and runtime.
//!
//! The middleware stack is deliberately ordered so every response (including
//! routing failures) carries a server-generated request id, RFC 7807 problem
//! details where applicable, and the API security headers.

mod auth;
mod error;
mod logging;
mod metrics;
mod middleware;
mod openapi;
mod router;

pub use auth::{AuthServiceError, AuthServices};
pub use error::{ApiError, FieldError, ProblemDetails};
pub use logging::{LoggingError, build_env_filter, init_logging};
pub use metrics::{MetricsError, install_metrics_recorder, metrics_router, spawn_metrics_upkeep};
pub use openapi::openapi_document;
pub use router::{
    AppState, MAX_IN_FLIGHT_REQUESTS, MAX_JSON_BODY_BYTES, REQUEST_TIMEOUT, build_router,
    with_common_middleware, with_common_middleware_and_timeout,
};

/// Map validated configuration into store pool settings.
pub fn store_pool_settings(pool: &rs_admin_config::PoolConfig) -> rs_admin_store::PoolSettings {
    rs_admin_store::PoolSettings {
        max_connections: pool.max_connections,
        min_connections: pool.min_connections,
        acquire_timeout: pool.acquire_timeout,
        idle_timeout: pool.idle_timeout,
        max_lifetime: pool.max_lifetime,
    }
}
