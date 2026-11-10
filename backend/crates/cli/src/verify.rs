//! Restore-drill verification: schema version, key table row counts and the
//! audit HMAC chain of a restored database (ADR-0008 §5.6, AC-OPS-05).
//!
//! The command only reads. It is run against a temporary restored cluster and
//! never against production. The SQL lives in `rs_admin_store::restore`.

use rs_admin_audit::{VerifyError, verify_chain};
use rs_admin_security::AuditKeyRing;
use rs_admin_store::restore::{self, RestoreStoreError};
use serde::Serialize;
use sqlx::PgPool;
use std::fmt;

/// The minimum number of built-in roles in a bootstrapped system
/// (`user`, `admin`, `super_admin`).
const MIN_ROLES: i64 = 3;

/// Everything the restore drill records for one restored database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RestoreVerification {
    pub migration_version: i64,
    pub migrations_ok: bool,
    pub row_counts: RowCounts,
    pub audit_chain: ChainVerificationReport,
}

/// Row counts for the tables the drill treats as key data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RowCounts {
    pub users: i64,
    pub roles: i64,
    pub sessions: i64,
    pub audit_logs: i64,
}

/// Result of recomputing the audit chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ChainVerificationReport {
    pub verified_rows: u64,
    pub head_seq: i64,
}

/// Errors from restore verification. Display and debug never include SQL or
/// connection details.
#[derive(thiserror::Error)]
pub enum RestoreVerifyError {
    #[error("database operation failed")]
    Inspect(#[source] RestoreStoreError),
    #[error("the restored database has no applied migrations")]
    NoMigrations,
    #[error("the restored database has a failed migration")]
    FailedMigration,
    #[error("the restored database is at migration {found}, expected {expected}")]
    UnexpectedMigrationVersion { found: i64, expected: i64 },
    #[error("the restored database is missing key rows (users: {users}, roles: {roles})")]
    MissingKeyRows { users: i64, roles: i64 },
    #[error("audit chain verification failed: {0}")]
    Audit(#[from] VerifyError),
}

impl fmt::Debug for RestoreVerifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RestoreVerifyError")
            .field(&self.to_string())
            .finish()
    }
}

/// Verify a restored cluster. `expected_migration_version` is optional so the
/// runbook can compare against the release being drilled.
pub async fn verify_restored_database(
    pool: &PgPool,
    keys: &AuditKeyRing,
    expected_migration_version: Option<i64>,
) -> Result<RestoreVerification, RestoreVerifyError> {
    let schema = restore::inspect(pool).await.map_err(|error| match error {
        RestoreStoreError::NoMigrations => RestoreVerifyError::NoMigrations,
        RestoreStoreError::FailedMigration => RestoreVerifyError::FailedMigration,
        other => RestoreVerifyError::Inspect(other),
    })?;

    if let Some(expected) = expected_migration_version
        && schema.migration_version != expected
    {
        return Err(RestoreVerifyError::UnexpectedMigrationVersion {
            found: schema.migration_version,
            expected,
        });
    }

    if schema.row_counts.users < 1 || schema.row_counts.roles < MIN_ROLES {
        return Err(RestoreVerifyError::MissingKeyRows {
            users: schema.row_counts.users,
            roles: schema.row_counts.roles,
        });
    }

    let chain = verify_chain(pool, keys).await?;

    Ok(RestoreVerification {
        migration_version: schema.migration_version,
        migrations_ok: schema.migrations_ok,
        row_counts: RowCounts {
            users: schema.row_counts.users,
            roles: schema.row_counts.roles,
            sessions: schema.row_counts.sessions,
            audit_logs: schema.row_counts.audit_logs,
        },
        audit_chain: ChainVerificationReport {
            verified_rows: chain.verified_rows,
            head_seq: chain.head_seq,
        },
    })
}
