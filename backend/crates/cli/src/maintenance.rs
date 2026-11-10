//! `maintenance`: the scheduled retention cleanup (spec §4.5, ADR-0008 §7.2).
//!
//! A run takes a fixed advisory lock so duplicate timers cannot overlap; when
//! the lock is held, the command succeeds without doing work (exit 0). The
//! audit purge deletes an expired prefix and, in the same transaction, appends
//! a `system_audit_logs_purged` event whose `last_purged_hash` anchors the
//! surviving chain. A failed event write rolls the deletions back.

use crate::error::CliError;
use chrono::{DateTime, Utc};
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, AuditWriter};
use rs_admin_store::maintenance::{self, MaintenanceOutcome, MaintenancePolicy};
use sqlx::PgPool;
use uuid::Uuid;

/// Whether this invocation did the work or deferred to a concurrent run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaintenanceStatus {
    Completed,
    SkippedLockHeld,
}

/// The result of one maintenance invocation.
#[derive(Clone, Debug)]
pub struct MaintenanceRun {
    pub status: MaintenanceStatus,
    pub outcome: MaintenanceOutcome,
}

impl MaintenanceRun {
    fn skipped() -> Self {
        Self {
            status: MaintenanceStatus::SkippedLockHeld,
            outcome: MaintenanceOutcome::default(),
        }
    }
}

/// Take the maintenance lock, run the cleanup at the database clock, and
/// release the lock. The lock is always released before returning.
pub async fn run_maintenance(
    pool: &PgPool,
    writer: &AuditWriter,
    policy: &MaintenancePolicy,
) -> Result<MaintenanceRun, CliError> {
    let mut transaction = pool.begin().await?;
    let now = maintenance::database_now(&mut transaction).await?;
    transaction.rollback().await?;
    run_maintenance_at(pool, writer, policy, now).await
}

/// Run the cleanup at an explicit instant. Production uses
/// [`run_maintenance`]; tests use this to age data without rewriting hashed
/// audit rows.
pub async fn run_maintenance_at(
    pool: &PgPool,
    writer: &AuditWriter,
    policy: &MaintenancePolicy,
    now: DateTime<Utc>,
) -> Result<MaintenanceRun, CliError> {
    let mut lock_connection = pool.acquire().await?;
    if !maintenance::try_advisory_lock(&mut lock_connection).await? {
        return Ok(MaintenanceRun::skipped());
    }

    let result = run_locked(pool, writer, policy, now).await;
    let unlock = maintenance::advisory_unlock(&mut lock_connection).await;

    match (result, unlock) {
        (Ok(run), Ok(())) => Ok(run),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
    }
}

async fn run_locked(
    pool: &PgPool,
    writer: &AuditWriter,
    policy: &MaintenancePolicy,
    now: DateTime<Utc>,
) -> Result<MaintenanceRun, CliError> {
    let mut transaction = pool.begin().await?;
    let outcome = maintenance::cleanup(&mut transaction, now, policy).await?;
    if outcome.audit.count > 0 {
        write_purge_event(&mut transaction, writer, policy, &outcome).await?;
    }
    transaction.commit().await?;

    Ok(MaintenanceRun {
        status: MaintenanceStatus::Completed,
        outcome,
    })
}

async fn write_purge_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    writer: &AuditWriter,
    policy: &MaintenancePolicy,
    outcome: &MaintenanceOutcome,
) -> Result<(), CliError> {
    let from_seq = outcome
        .audit
        .from_seq
        .ok_or(CliError::InvalidMaintenanceOutcome)?;
    let to_seq = outcome
        .audit
        .to_seq
        .ok_or(CliError::InvalidMaintenanceOutcome)?;
    let last_purged_hash = outcome
        .audit
        .last_purged_hash
        .ok_or(CliError::InvalidMaintenanceOutcome)?;

    let event = AuditEvent::builder(
        AuditAction::SystemAuditLogsPurged,
        AuditResult::Success,
        AuditActor::system(),
        Uuid::now_v7(),
    )
    .detail("from_seq", from_seq)
    .detail("to_seq", to_seq)
    .detail("count", outcome.audit.count)
    .detail("last_purged_hash", hex(&last_purged_hash))
    .detail(
        "retention_days",
        u64::from(policy.audit_info_retention_days),
    )
    .detail(
        "retention_critical_days",
        u64::from(policy.audit_critical_retention_days),
    )
    .build()?;

    writer.write_in_tx(transaction, &event).await?;
    Ok(())
}

fn hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
