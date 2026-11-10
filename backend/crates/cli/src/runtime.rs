//! Runtime plumbing shared by the management binaries.
//!
//! The commands only need the database and audit-chain key material; they are
//! not HTTP servers and never build JWT or refresh-token services.

use rs_admin_config::PoolConfig;
use rs_admin_store::{Database, PoolSettings, Readiness, UnreadyReason};
use std::time::Duration;

/// Readiness budget for the CLI's schema check.
pub const SCHEMA_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Map validated configuration into store pool settings.
pub fn store_pool_settings(pool: &PoolConfig) -> PoolSettings {
    PoolSettings {
        max_connections: pool.max_connections,
        min_connections: pool.min_connections,
        acquire_timeout: pool.acquire_timeout,
        idle_timeout: pool.idle_timeout,
        max_lifetime: pool.max_lifetime,
    }
}

/// Refuse to run against an unreachable database or an unmigrated/mismatched
/// schema. The CLI never applies migrations itself; `migrate` is a separate
/// deployment step.
pub async fn ensure_schema(database: &Database) -> Result<(), SchemaError> {
    match database.check_ready(SCHEMA_CHECK_TIMEOUT).await {
        Readiness::Ready => Ok(()),
        Readiness::Unready(UnreadyReason::MigrationMismatch { applied, expected }) => {
            Err(SchemaError::Mismatch { applied, expected })
        }
        Readiness::Unready(
            UnreadyReason::DatabaseUnavailable | UnreadyReason::MigrationsUnavailable,
        ) => Err(SchemaError::Unavailable),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    #[error("the database is unavailable or its migration history cannot be read")]
    Unavailable,
    #[error(
        "the database schema is at migration {applied:?} but this binary requires at least \
         {expected}; run the migrate command first"
    )]
    Mismatch { applied: Option<i64>, expected: i64 },
}
