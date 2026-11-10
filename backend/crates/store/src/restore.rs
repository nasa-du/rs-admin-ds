//! Read-only inspection of a restored cluster for the restore drill
//! (ADR-0008 §5.6, AC-OPS-05).
//!
//! The maintenance/restore CLIs do not write SQL; this module owns the two
//! queries the drill needs before it verifies the audit chain.

use sqlx::PgPool;
use std::fmt;

/// Row counts for the tables the drill treats as key data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestoreRowCounts {
    pub users: i64,
    pub roles: i64,
    pub sessions: i64,
    pub audit_logs: i64,
}

/// Schema state read from `_sqlx_migrations` plus the key row counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestoreSchema {
    pub migration_version: i64,
    pub migrations_ok: bool,
    pub row_counts: RestoreRowCounts,
}

/// Errors from the restore inspection. Display and debug never include SQL or
/// connection details.
#[derive(thiserror::Error)]
pub enum RestoreStoreError {
    #[error("restore inspection query failed")]
    Database(#[source] sqlx::Error),
    #[error("the restored database has no applied migrations")]
    NoMigrations,
    #[error("the restored database has a failed migration")]
    FailedMigration,
}

impl fmt::Debug for RestoreStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RestoreStoreError")
            .field(&self.to_string())
            .finish()
    }
}

/// Read the migration state and key table counts from a restored database.
pub async fn inspect(pool: &PgPool) -> Result<RestoreSchema, RestoreStoreError> {
    let (migration_version, migrations_ok): (Option<i64>, Option<bool>) =
        sqlx::query_as("SELECT MAX(version)::bigint, BOOL_AND(success) FROM _sqlx_migrations")
            .fetch_one(pool)
            .await
            .map_err(RestoreStoreError::Database)?;

    let Some(migration_version) = migration_version else {
        return Err(RestoreStoreError::NoMigrations);
    };
    if migrations_ok != Some(true) {
        return Err(RestoreStoreError::FailedMigration);
    }

    let (users, roles, sessions, audit_logs): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM users),
            (SELECT COUNT(*) FROM roles),
            (SELECT COUNT(*) FROM auth_sessions),
            (SELECT COUNT(*) FROM audit_logs)",
    )
    .fetch_one(pool)
    .await
    .map_err(RestoreStoreError::Database)?;

    Ok(RestoreSchema {
        migration_version,
        migrations_ok: true,
        row_counts: RestoreRowCounts {
            users,
            roles,
            sessions,
            audit_logs,
        },
    })
}
