//! HTTP API surface and runtime.
//!
//! The middleware stack is deliberately ordered so every response (including
//! routing failures) carries a server-generated request id, RFC 7807 problem
//! details where applicable, and the API security headers.

pub mod alert_webhook;
mod audit_logs;
mod auth;
mod authz;
mod browser;
mod client_ip;
mod error;
mod json;
mod logging;
mod metrics;
mod middleware;
mod openapi;
mod query;
mod roles;
mod router;
mod users;
mod users_lifecycle;
mod wire;

pub use auth::{AuthServiceError, AuthServices};
pub use error::{ApiError, FieldError, ProblemDetails};
pub use logging::{LoggingError, build_env_filter, init_logging};
pub use metrics::{
    MetricsError, install_metrics_recorder, metrics_router, record_db_pool_metrics,
    spawn_db_pool_metrics, spawn_metrics_upkeep,
};
pub use openapi::openapi_document;
pub use router::{
    AppState, MAX_IN_FLIGHT_REQUESTS, MAX_JSON_BODY_BYTES, REQUEST_TIMEOUT, build_router,
    with_common_middleware, with_common_middleware_and_timeout,
};

/// Wait for SIGTERM or Ctrl-C. Shared by the API and alert-webhook runtimes.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to listen for ctrl-c");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}

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
