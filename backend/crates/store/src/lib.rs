//! Database access, migrations and readiness checks.
//!
//! This is the only crate that writes SQL. Errors never carry the database
//! URL or credentials; callers may log the stable error message only.

pub mod audit;
pub mod identity;
pub mod rbac;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

/// Embedded migrations. The binary embeds them at compile time; it does not
/// run them on startup. Production applies them as a separate deployment step.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Version of the newest embedded migration.
pub fn expected_migration_version() -> i64 {
    MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0)
}

/// Connection-pool configuration.
#[derive(Clone, Debug)]
pub struct PoolSettings {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
}

impl Default for PoolSettings {
    fn default() -> Self {
        Self {
            max_connections: 10,
            min_connections: 1,
            acquire_timeout: Duration::from_secs(3),
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(1800),
        }
    }
}

/// Errors from the store layer. Display text is stable and contains no
/// connection details.
#[derive(thiserror::Error)]
pub enum StoreError {
    #[error("invalid database configuration")]
    InvalidUrl,
    #[error("database connection failed")]
    Connect(#[source] sqlx::Error),
    #[error("database migration failed")]
    Migrate(#[source] sqlx::migrate::MigrateError),
}

// Keep accidental `{:?}` logging from printing the SQLx source, which may
// contain connection details.
impl fmt::Debug for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StoreError")
            .field(&self.to_string())
            .finish()
    }
}

/// Result of a readiness probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Unready(UnreadyReason),
}

impl Readiness {
    /// Readiness requires the applied migration version to equal the newest
    /// embedded migration exactly. A database ahead of the binary is unready.
    pub fn from_migration_versions(applied: Option<i64>, expected: i64) -> Self {
        if applied == Some(expected) {
            Self::Ready
        } else {
            Self::Unready(UnreadyReason::MigrationMismatch { applied, expected })
        }
    }
}

/// Stable internal reasons for an unready probe. These may be logged but must
/// not be returned to clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnreadyReason {
    DatabaseUnavailable,
    MigrationsUnavailable,
    MigrationMismatch { applied: Option<i64>, expected: i64 },
}

/// A PostgreSQL pool plus migration helpers.
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connect eagerly. Startup fails when the database is unreachable.
    pub async fn connect(url: &str, settings: &PoolSettings) -> Result<Self, StoreError> {
        let options = connect_options(url)?;
        let pool = pool_options(settings)
            .connect_with(options)
            .await
            .map_err(StoreError::Connect)?;
        Ok(Self { pool })
    }

    /// Build a pool without connecting. Used by tests and by callers that
    /// want readiness (rather than startup) to report database failures.
    pub fn connect_lazy(url: &str, settings: &PoolSettings) -> Result<Self, StoreError> {
        let options = connect_options(url)?;
        let pool = pool_options(settings).connect_lazy_with(options);
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Apply embedded migrations. Called by the standalone `migrate` binary,
    /// never by the server's startup path.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR.run(&self.pool).await.map_err(StoreError::Migrate)
    }

    /// Check database connectivity and migration-version equality with one
    /// overall timeout. The caller maps this to 200/503 and logs the reason.
    pub async fn check_ready(&self, timeout: Duration) -> Readiness {
        let expected = expected_migration_version();

        let probe = tokio::time::timeout(timeout, async {
            match sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(&self.pool)
                .await
            {
                Ok(1) => {}
                _ => return Probe::DatabaseUnavailable,
            }

            match sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM _sqlx_migrations")
                .fetch_one(&self.pool)
                .await
            {
                Ok(applied) => Probe::Applied(applied),
                Err(_) => Probe::MigrationsUnavailable,
            }
        })
        .await;

        match probe {
            Ok(Probe::Applied(applied)) => Readiness::from_migration_versions(applied, expected),
            Ok(Probe::MigrationsUnavailable) => {
                Readiness::Unready(UnreadyReason::MigrationsUnavailable)
            }
            Ok(Probe::DatabaseUnavailable) | Err(_) => {
                Readiness::Unready(UnreadyReason::DatabaseUnavailable)
            }
        }
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

enum Probe {
    DatabaseUnavailable,
    MigrationsUnavailable,
    Applied(Option<i64>),
}

fn connect_options(url: &str) -> Result<PgConnectOptions, StoreError> {
    PgConnectOptions::from_str(url).map_err(|_| StoreError::InvalidUrl)
}

fn pool_options(settings: &PoolSettings) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .min_connections(settings.min_connections)
        .acquire_timeout(settings.acquire_timeout)
        .idle_timeout(settings.idle_timeout)
        .max_lifetime(settings.max_lifetime)
}
