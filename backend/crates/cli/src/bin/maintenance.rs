//! Scheduled retention cleanup (spec §4.5, ADR-0008 §7.2).
//!
//! ```sh
//! maintenance
//! ```
//!
//! Reads `DATABASE_URL`/`DATABASE_URL_FILE` (the `rs_admin_cleanup` role), the
//! audit chain keyring and `AUDIT_RETENTION_*` from the environment. Exits 0
//! when another run holds the advisory lock. When `MAINTENANCE_METRICS_FILE`
//! is set, writes a Prometheus textfile collected by node_exporter.

use rs_admin_audit::AuditWriter;
use rs_admin_cli::runtime::{ensure_schema, store_pool_settings};
use rs_admin_cli::{MaintenanceStatus, run_maintenance};
use rs_admin_config::{AuditChainConfig, AuditSettings, DatabaseConfig};
use rs_admin_store::Database;
use rs_admin_store::maintenance::{MaintenanceOutcome, MaintenancePolicy};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
maintenance - apply retention cleanup to the rs-admin database

Usage:
  maintenance

Takes a fixed advisory lock; when another run holds it, prints a notice and
exits 0. Deletes expired refresh tokens, sessions, login failures/throttle
rows and an expired audit prefix, writing system_audit_logs_purged in the same
transaction as the audit deletion.

When MAINTENANCE_METRICS_FILE is set, writes maintenance_last_success_timestamp,
maintenance_last_attempt_success, maintenance_duration_seconds and per-table
row counts as a Prometheus textfile.";

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("maintenance: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(argument) = std::env::args().nth(1) {
        match argument.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => return Err("unknown argument".into()),
        }
    }

    let database_config = DatabaseConfig::from_env()?;
    let audit_settings = AuditSettings::from_env()?;
    let audit_chain = AuditChainConfig::from_env()?;
    let policy = MaintenancePolicy {
        audit_info_retention_days: audit_settings.retention_info_days,
        audit_critical_retention_days: audit_settings.retention_critical_days,
        ..MaintenancePolicy::default()
    };
    let metrics_path = std::env::var("MAINTENANCE_METRICS_FILE")
        .ok()
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);

    let database = Database::connect(
        database_config.url(),
        &store_pool_settings(&database_config.pool),
    )
    .await?;
    ensure_schema(&database).await?;

    let writer = AuditWriter::new(audit_chain.keys);
    let started = Instant::now();
    let run = run_maintenance(database.pool(), &writer, &policy).await;
    let duration = started.elapsed();
    database.close().await;

    let run = match run {
        Ok(run) => run,
        Err(error) => {
            write_metrics(
                metrics_path.as_deref(),
                false,
                duration,
                None,
                previous_success(metrics_path.as_deref()),
            );
            return Err(error.into());
        }
    };

    match run.status {
        MaintenanceStatus::SkippedLockHeld => {
            println!("maintenance: another instance holds the lock; skipped");
        }
        MaintenanceStatus::Completed => {
            println!("maintenance: completed");
            println!(
                "  refresh tokens deleted: {}",
                run.outcome.refresh_tokens_deleted
            );
            println!("  sessions deleted:       {}", run.outcome.sessions_deleted);
            println!(
                "  login failures deleted: {}",
                run.outcome.login_failures_deleted
            );
            println!(
                "  throttle rows deleted:  {}",
                run.outcome.login_throttle_deleted
            );
            match (run.outcome.audit.from_seq, run.outcome.audit.to_seq) {
                (Some(from_seq), Some(to_seq)) => println!(
                    "  audit rows purged:      {} (chain seq {from_seq}..{to_seq})",
                    run.outcome.audit.count
                ),
                _ => println!("  audit rows purged:      0"),
            }
            println!("  duration:               {:.3}s", duration.as_secs_f64());

            write_metrics(
                metrics_path.as_deref(),
                true,
                duration,
                Some(&run.outcome),
                previous_success(metrics_path.as_deref()),
            );
        }
    }

    Ok(())
}

fn previous_success(path: Option<&Path>) -> u64 {
    path.and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("maintenance_last_success_timestamp ")
                    .and_then(|value| value.trim().parse().ok())
            })
        })
        .unwrap_or(0)
}

/// Write the Prometheus textfile atomically. Metrics failures are logged but do
/// not make a completed cleanup fail.
fn write_metrics(
    path: Option<&Path>,
    success: bool,
    duration: Duration,
    outcome: Option<&MaintenanceOutcome>,
    last_success: u64,
) {
    let Some(path) = path else {
        return;
    };

    let attempted_at = unix_now();
    let last_success = if success { attempted_at } else { last_success };
    let mut text = String::new();
    text.push_str(
        "# HELP maintenance_last_success_timestamp Unix time of the last successful cleanup.\n",
    );
    text.push_str("# TYPE maintenance_last_success_timestamp gauge\n");
    text.push_str(&format!(
        "maintenance_last_success_timestamp {last_success}\n"
    ));
    text.push_str("# HELP maintenance_last_attempt_success Whether the most recent cleanup attempt succeeded (1/0).\n");
    text.push_str("# TYPE maintenance_last_attempt_success gauge\n");
    text.push_str(&format!(
        "maintenance_last_attempt_success {}\n",
        u8::from(success)
    ));
    text.push_str(
        "# HELP maintenance_duration_seconds Wall-clock duration of the cleanup attempt.\n",
    );
    text.push_str("# TYPE maintenance_duration_seconds gauge\n");
    text.push_str(&format!(
        "maintenance_duration_seconds {:.3}\n",
        duration.as_secs_f64()
    ));
    if let Some(outcome) = outcome {
        text.push_str(
            "# HELP maintenance_rows_deleted Rows deleted by the most recent successful cleanup.\n",
        );
        text.push_str("# TYPE maintenance_rows_deleted gauge\n");
        for (kind, count) in [
            ("refresh_tokens", outcome.refresh_tokens_deleted),
            ("sessions", outcome.sessions_deleted),
            ("login_failures", outcome.login_failures_deleted),
            ("login_throttle", outcome.login_throttle_deleted),
            ("audit", outcome.audit.count),
        ] {
            text.push_str(&format!(
                "maintenance_rows_deleted{{kind=\"{kind}\"}} {count}\n"
            ));
        }
    }

    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let result = fs::write(&temporary, text).and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = result {
        eprintln!(
            "maintenance: could not write metrics to {}: {error}",
            path.display()
        );
        let _ = fs::remove_file(&temporary);
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
