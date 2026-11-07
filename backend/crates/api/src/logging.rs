//! Structured JSON logging to stdout with an explicit default filter.

use tracing_subscriber::EnvFilter;

/// Default filter when `RUST_LOG` is unset or empty. SQLx query logging is
/// noisy and can contain parameters, so it is raised to `warn`.
pub const DEFAULT_LOG_FILTER: &str = "info,sqlx=warn";

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("invalid RUST_LOG filter: {0}")]
    InvalidFilter(String),
    #[error("failed to install tracing subscriber: {0}")]
    Install(String),
}

/// Parse `RUST_LOG`. An invalid directive is a startup error, not a silent
/// fallback.
pub fn build_env_filter(spec: Option<&str>) -> Result<EnvFilter, LoggingError> {
    let spec = spec
        .map(str::trim)
        .filter(|spec| !spec.is_empty())
        .unwrap_or(DEFAULT_LOG_FILTER);

    EnvFilter::try_new(spec).map_err(|error| LoggingError::InvalidFilter(error.to_string()))
}

/// Install the global JSON subscriber. Call once at startup.
pub fn init_logging() -> Result<(), LoggingError> {
    let spec = std::env::var("RUST_LOG").ok();
    let filter = build_env_filter(spec.as_deref())?;

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_current_span(true)
        .with_span_list(false)
        .flatten_event(true)
        .with_target(false)
        .with_writer(std::io::stdout)
        .try_init()
        .map_err(|error| LoggingError::Install(error.to_string()))
}
