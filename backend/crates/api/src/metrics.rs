//! Prometheus recorder, upkeep task and the internal metrics listener.

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Interval for the connection-pool gauges. The spec scrapes every 15 seconds;
/// refreshing on the same cadence keeps the values fresh without extra traffic.
const POOL_METRICS_INTERVAL: Duration = Duration::from_secs(15);

const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("failed to install metrics recorder: {0}")]
    Install(String),
}

/// Install the global Prometheus recorder and describe the metric catalog.
pub fn install_metrics_recorder() -> Result<PrometheusHandle, MetricsError> {
    let builder = PrometheusBuilder::new()
        .set_buckets(&[
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
        ])
        .map_err(|error| MetricsError::Install(error.to_string()))?;
    let handle = builder
        .install_recorder()
        .map_err(|error| MetricsError::Install(error.to_string()))?;

    metrics::describe_counter!("http_requests_total", "Total HTTP requests.");
    metrics::describe_histogram!(
        "http_request_duration_seconds",
        "HTTP request latency in seconds."
    );
    metrics::describe_gauge!(
        "db_pool_connections",
        "Database pool connections grouped by state (active/idle/max)."
    );
    metrics::describe_counter!(
        "auth_login_attempts_total",
        "Login attempts grouped by result and finite reason."
    );
    metrics::describe_counter!(
        "auth_login_lockouts_total",
        "Login lockouts grouped by throttle scope."
    );
    metrics::describe_counter!(
        "auth_refresh_reuse_detected_total",
        "Refresh-token reuse detections."
    );
    metrics::describe_histogram!(
        "auth_password_hash_seconds",
        "Argon2id hash and verification latency in seconds."
    );
    metrics::describe_gauge!(
        "auth_global_circuit_open",
        "Whether the global login circuit is open (1) or closed (0)."
    );
    metrics::describe_counter!(
        "audit_write_failures_total",
        "Audit chain writes that failed and failed the calling action closed."
    );
    metrics::describe_gauge!("build_info", "Build information.");
    metrics::gauge!(
        "build_info",
        "version" => env!("CARGO_PKG_VERSION"),
        "commit" => option_env!("GIT_COMMIT").unwrap_or("unknown")
    )
    .set(1.0);

    Ok(handle)
}

/// Refresh histogram internals periodically so memory does not grow without
/// bound. The exporter does not spawn this task itself.
pub fn spawn_metrics_upkeep(handle: PrometheusHandle) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            handle.run_upkeep();
        }
    })
}

/// Publish the SQLx pool gauges. `max` comes from validated configuration so
/// the saturation alert does not depend on the pool having grown.
pub fn record_db_pool_metrics(pool: &PgPool, max_connections: u32) {
    let idle = pool.num_idle() as u32;
    let size = pool.size();
    let active = size.saturating_sub(idle);

    metrics::gauge!("db_pool_connections", "state" => "active").set(f64::from(active));
    metrics::gauge!("db_pool_connections", "state" => "idle").set(f64::from(idle));
    metrics::gauge!("db_pool_connections", "state" => "max").set(f64::from(max_connections));
}

/// Refresh the pool gauges on the scrape cadence.
pub fn spawn_db_pool_metrics(pool: PgPool, max_connections: u32) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(POOL_METRICS_INTERVAL);
        loop {
            interval.tick().await;
            record_db_pool_metrics(&pool, max_connections);
        }
    })
}

/// Internal-only metrics listener. It intentionally bypasses all business
/// middleware and exposes only `GET /metrics`.
pub fn metrics_router(handle: PrometheusHandle) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(handle)
}

async fn metrics_handler(State(handle): State<PrometheusHandle>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, METRICS_CONTENT_TYPE)],
        handle.render(),
    )
}
