//! Alertmanager -> chat webhook adapter. Runs as its own Compose service from
//! the API image; it never touches the database or the business API.

use rs_admin_api::{alert_webhook, init_logging};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(error) = init_logging() {
        eprintln!("startup failed: {error}");
        return ExitCode::FAILURE;
    }

    match alert_webhook::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "alert webhook adapter failed");
            ExitCode::FAILURE
        }
    }
}
