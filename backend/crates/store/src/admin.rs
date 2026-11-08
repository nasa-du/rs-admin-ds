//! SQL for the offline management CLI (ADR-0003 §6, spec §2.2).
//!
//! These statements back `bootstrap-super-admin` and `recover-super-admin`.
//! Neither command is reachable over HTTP. Errors never carry SQL, connection
//! details or credential material.

use chrono::{DateTime, Utc};
use rs_admin_domain::rbac::BuiltInRole;
use sqlx::{Executor, Postgres, Transaction};
use std::fmt;
use std::time::Duration;
use uuid::Uuid;

/// Advisory-lock key that serializes concurrent bootstrap runs. Derived from
/// the ASCII bytes `RSADMIN` plus a format version byte; distinct from the
/// catalog-sync key.
pub const BOOTSTRAP_SUPER_ADMIN_LOCK_KEY: i64 = 0x5253_4144_4D49_4E02;

/// Errors from the management SQL layer. Display and debug never include SQL
/// or connection details.
#[derive(thiserror::Error)]
pub enum AdminStoreError {
    #[error("management store query failed")]
    Database(#[source] sqlx::Error),
}

impl From<sqlx::Error> for AdminStoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl fmt::Debug for AdminStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AdminStoreError")
            .field(&self.to_string())
            .finish()
    }
}

/// A user row as needed by the CLI commands. The display name is part of the
/// password policy's identifier-equality check (spec §8.2).
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct CliUser {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub display_name: String,
}

/// Serialize bootstrap runs for the rest of the transaction.
pub async fn lock_bootstrap(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<(), AdminStoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(BOOTSTRAP_SUPER_ADMIN_LOCK_KEY)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

/// Count users that currently hold the built-in `super_admin` role and are
/// enabled. Bootstrap refuses while this is non-zero.
pub async fn count_enabled_super_admins<'e, E>(executor: E) -> Result<i64, AdminStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar(
        "SELECT COUNT(DISTINCT u.id)
         FROM users u
         JOIN user_roles ur ON ur.user_id = u.id
         JOIN roles r ON r.id = ur.role_id
         WHERE u.status = 'active'
           AND r.name = $1
           AND r.is_builtin
           AND r.deleted_at IS NULL",
    )
    .bind(BuiltInRole::SuperAdmin.code())
    .fetch_one(executor)
    .await
    .map_err(AdminStoreError::from)
}

/// Lock and load a user by normalized email inside the transaction.
pub async fn lock_user_by_email(
    transaction: &mut Transaction<'_, Postgres>,
    email: &str,
) -> Result<Option<CliUser>, AdminStoreError> {
    sqlx::query_as(
        "SELECT id, username, email, display_name
         FROM users WHERE email = $1
         FOR UPDATE",
    )
    .bind(email)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AdminStoreError::from)
}

/// Find the user that owns a normalized username.
pub async fn find_user_id_by_username<'e, E>(
    executor: E,
    username: &str,
) -> Result<Option<Uuid>, AdminStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_optional(executor)
        .await
        .map_err(AdminStoreError::from)
}

/// Whether the user holds the built-in role (regardless of status).
pub async fn user_holds_builtin_role<'e, E>(
    executor: E,
    user_id: Uuid,
    role: BuiltInRole,
) -> Result<bool, AdminStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1
            FROM user_roles ur
            JOIN roles r ON r.id = ur.role_id
            WHERE ur.user_id = $1
              AND r.name = $2
              AND r.is_builtin
              AND r.deleted_at IS NULL
         )",
    )
    .bind(user_id)
    .bind(role.code())
    .fetch_one(executor)
    .await
    .map_err(AdminStoreError::from)
}

/// Insert a new enabled user holding a temporary password. The database clock
/// defines the expiry. Returns the id and the expiry timestamp.
pub async fn insert_cli_user(
    transaction: &mut Transaction<'_, Postgres>,
    username: &str,
    email: &str,
    password_hash: &str,
    temporary_password_ttl: Duration,
) -> Result<(Uuid, DateTime<Utc>), AdminStoreError> {
    sqlx::query_as(
        "INSERT INTO users (
            username, email, display_name, password_hash, status,
            must_change_password, temporary_password_expires_at
         ) VALUES (
            $1, $2, $1, $3, 'active',
            true, now() + make_interval(secs => $4)
         )
         RETURNING id, temporary_password_expires_at",
    )
    .bind(username)
    .bind(email)
    .bind(password_hash)
    .bind(temporary_password_ttl.as_secs_f64())
    .fetch_one(&mut **transaction)
    .await
    .map_err(AdminStoreError::from)
}

/// Grant a built-in role. Idempotent; no error when the user already holds it.
pub async fn assign_builtin_role<'e, E>(
    executor: E,
    user_id: Uuid,
    role: BuiltInRole,
) -> Result<(), AdminStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id)
         SELECT $1, id
         FROM roles
         WHERE name = $2 AND is_builtin AND deleted_at IS NULL
         ON CONFLICT (user_id, role_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(role.code())
    .execute(executor)
    .await?;
    Ok(())
}

/// Replace the password with a temporary one, enable the account, clear the
/// account-level login lock and return the database-defined expiry.
pub async fn set_temporary_credentials(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    password_hash: &str,
    temporary_password_ttl: Duration,
) -> Result<DateTime<Utc>, AdminStoreError> {
    sqlx::query_scalar(
        "UPDATE users
         SET password_hash = $2,
             status = 'active',
             must_change_password = true,
             temporary_password_expires_at = now() + make_interval(secs => $3),
             failed_login_count = 0,
             locked_until = NULL,
             last_failed_login_at = NULL,
             updated_at = now()
         WHERE id = $1
         RETURNING temporary_password_expires_at",
    )
    .bind(user_id)
    .bind(password_hash)
    .bind(temporary_password_ttl.as_secs_f64())
    .fetch_one(&mut **transaction)
    .await
    .map_err(AdminStoreError::from)
}
