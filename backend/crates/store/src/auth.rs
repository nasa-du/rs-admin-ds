//! SQL for authentication: identity lookup, sessions and refresh tokens
//! (docs/specs/login-system.md §4.2–§5.3).
//!
//! Handlers never inspect SQL errors directly; every failure surfaces as
//! [`AuthStoreError`], whose display text is stable and contains no SQL or
//! connection details.

use chrono::{DateTime, Utc};
use sqlx::{Executor, PgPool, Postgres, Transaction};
use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use uuid::Uuid;

/// A user row as needed by the authentication pipeline.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub phone: Option<String>,
    pub password_hash: String,
    pub status: String,
    pub must_change_password: bool,
    pub temporary_password_expires_at: Option<DateTime<Utc>>,
    pub token_version: i64,
    pub failed_login_count: i32,
    pub locked_until: Option<DateTime<Utc>>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AuthUser {
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// A temporary password is expired only after a successful password check;
    /// callers must not reveal this state before verifying the password.
    pub fn temporary_password_expired(&self, now: DateTime<Utc>) -> bool {
        self.must_change_password
            && self
                .temporary_password_expires_at
                .is_some_and(|expires_at| expires_at <= now)
    }
}

/// An active session row.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AuthSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub idle_expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
    pub authenticated_at: DateTime<Utc>,
}

/// A role held by a user plus the permission keys granted by that role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserRole {
    pub id: Uuid,
    pub name: String,
    pub is_builtin: bool,
    pub permission_keys: Vec<String>,
}

/// Parameters for a new session.
#[derive(Clone, Debug)]
pub struct NewSession {
    pub user_id: Uuid,
    pub idle_expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
    pub authenticated_at: DateTime<Utc>,
    pub ip: Option<IpAddr>,
    pub user_agent: Option<String>,
}

/// Parameters for a new refresh-token row. Only the hash is persisted.
#[derive(Clone, Copy, Debug)]
pub struct NewRefreshToken<'a> {
    pub token_hash: &'a [u8; 32],
    pub session_id: Uuid,
    pub rotation_counter: i64,
    pub expires_at: DateTime<Utc>,
}

/// Errors from the authentication SQL layer.
#[derive(thiserror::Error)]
pub enum AuthStoreError {
    #[error("authentication store query failed")]
    Database(#[source] sqlx::Error),
}

impl From<sqlx::Error> for AuthStoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl fmt::Debug for AuthStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AuthStoreError")
            .field(&self.to_string())
            .finish()
    }
}

/// Find a user by their normalized username.
pub async fn find_user_by_username(
    pool: &PgPool,
    username: &str,
) -> Result<Option<AuthUser>, AuthStoreError> {
    sqlx::query_as(
        "SELECT id, username, email, display_name, phone, password_hash, status,
                must_change_password, temporary_password_expires_at, token_version,
                failed_login_count, locked_until, last_login_at, created_at, updated_at
         FROM users WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(AuthStoreError::Database)
}

/// Find a user by their normalized email address.
pub async fn find_user_by_email(
    pool: &PgPool,
    email: &str,
) -> Result<Option<AuthUser>, AuthStoreError> {
    sqlx::query_as(
        "SELECT id, username, email, display_name, phone, password_hash, status,
                must_change_password, temporary_password_expires_at, token_version,
                failed_login_count, locked_until, last_login_at, created_at, updated_at
         FROM users WHERE email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
    .map_err(AuthStoreError::Database)
}

/// Find a user by id, used by the per-request authentication pipeline.
pub async fn find_user_by_id(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<AuthUser>, AuthStoreError> {
    sqlx::query_as(
        "SELECT id, username, email, display_name, phone, password_hash, status,
                must_change_password, temporary_password_expires_at, token_version,
                failed_login_count, locked_until, last_login_at, created_at, updated_at
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(AuthStoreError::Database)
}

/// Lock the user row for the rest of the login transaction. Serializing logins
/// per user keeps the token version and session cap checks consistent.
pub async fn lock_user_for_login(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<Option<AuthUser>, AuthStoreError> {
    sqlx::query_as(
        "SELECT id, username, email, display_name, phone, password_hash, status,
                must_change_password, temporary_password_expires_at, token_version,
                failed_login_count, locked_until, last_login_at, created_at, updated_at
         FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)
}

/// Insert a session and return its generated id (`sid`).
pub async fn create_session(
    transaction: &mut Transaction<'_, Postgres>,
    session: NewSession,
) -> Result<Uuid, AuthStoreError> {
    let ip = session.ip.map(|ip| ip.to_string());
    sqlx::query_scalar(
        "INSERT INTO auth_sessions (
            user_id, idle_expires_at, absolute_expires_at, authenticated_at, ip, user_agent
         ) VALUES ($1, $2, $3, $4, $5::inet, $6)
         RETURNING id",
    )
    .bind(session.user_id)
    .bind(session.idle_expires_at)
    .bind(session.absolute_expires_at)
    .bind(session.authenticated_at)
    .bind(ip.as_deref())
    .bind(session.user_agent.as_deref())
    .fetch_one(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)
}

/// Persist a refresh token by hash.
pub async fn insert_refresh_token(
    transaction: &mut Transaction<'_, Postgres>,
    token: NewRefreshToken<'_>,
) -> Result<(), AuthStoreError> {
    sqlx::query(
        "INSERT INTO auth_refresh_tokens (
            token_hash, session_id, rotation_counter, expires_at
         ) VALUES ($1, $2, $3, $4)",
    )
    .bind(token.token_hash.as_slice())
    .bind(token.session_id)
    .bind(token.rotation_counter)
    .bind(token.expires_at)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(())
}

/// Load a session only while it is unrevoked and neither idle- nor
/// absolute-expired. `now()` is the database clock, as required for shared
/// multi-instance state.
pub async fn load_active_session(
    pool: &PgPool,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<Option<AuthSession>, AuthStoreError> {
    sqlx::query_as(
        "SELECT id, user_id, created_at, last_seen_at, idle_expires_at,
                absolute_expires_at, authenticated_at
         FROM auth_sessions
         WHERE id = $1 AND user_id = $2
           AND revoked_at IS NULL
           AND idle_expires_at > now()
           AND absolute_expires_at > now()",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(AuthStoreError::Database)
}

/// Mark the login as successful: reset account-level failure state and stamp
/// `last_login_at`, optionally upgrading the password hash in the same row.
pub async fn update_successful_login(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    new_password_hash: Option<&str>,
) -> Result<(), AuthStoreError> {
    sqlx::query(
        "UPDATE users
         SET last_login_at = now(),
             failed_login_count = 0,
             locked_until = NULL,
             last_failed_login_at = NULL,
             password_hash = COALESCE($2::text, password_hash),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(new_password_hash)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(())
}

/// Increment the account-level failure counter. Lock thresholds are applied by
/// the login-throttling layer.
pub async fn record_login_failure(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<(), AuthStoreError> {
    sqlx::query(
        "UPDATE users
         SET failed_login_count = failed_login_count + 1,
             last_failed_login_at = now(),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(())
}

/// Replace a password hash only when the stored value is still the one that
/// was verified. This prevents a rehash from overwriting a concurrent
/// administrator password reset. Returns whether the row was updated.
pub async fn replace_password_hash(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    expected_hash: &str,
    new_hash: &str,
) -> Result<bool, AuthStoreError> {
    let result = sqlx::query(
        "UPDATE users
         SET password_hash = $3, updated_at = now()
         WHERE id = $1 AND password_hash = $2",
    )
    .bind(user_id)
    .bind(expected_hash)
    .bind(new_hash)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(result.rows_affected() == 1)
}

/// Clear account-level failure state after a successful login or
/// reauthentication.
pub async fn reset_login_failures(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<(), AuthStoreError> {
    sqlx::query(
        "UPDATE users
         SET failed_login_count = 0,
             locked_until = NULL,
             last_failed_login_at = NULL,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(())
}

/// Stamp the session as freshly authenticated and return the database time.
/// Returns `None` when the session was revoked concurrently.
pub async fn touch_authenticated_at(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
) -> Result<Option<DateTime<Utc>>, AuthStoreError> {
    sqlx::query_scalar(
        "UPDATE auth_sessions
         SET authenticated_at = now()
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING authenticated_at",
    )
    .bind(session_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)
}

/// Load the user's active role grants with their permission keys. The built-in
/// role permission sets are re-resolved from code by the caller; this query
/// returns the persisted keys for every role.
pub async fn load_role_grants<'e, E>(
    executor: E,
    user_id: Uuid,
) -> Result<Vec<UserRole>, AuthStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    let rows: Vec<(Uuid, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT r.id, r.name, r.is_builtin, rp.permission_key
         FROM user_roles ur
         JOIN roles r ON r.id = ur.role_id AND r.deleted_at IS NULL
         LEFT JOIN role_permissions rp ON rp.role_id = r.id
         WHERE ur.user_id = $1
         ORDER BY r.is_builtin DESC, r.name, rp.permission_key",
    )
    .bind(user_id)
    .fetch_all(executor)
    .await
    .map_err(AuthStoreError::Database)?;

    let mut grants: BTreeMap<String, UserRole> = BTreeMap::new();
    for (id, name, is_builtin, permission_key) in rows {
        let grant = grants.entry(name.clone()).or_insert_with(|| UserRole {
            id,
            name,
            is_builtin,
            permission_keys: Vec::new(),
        });
        if let Some(permission_key) = permission_key
            && !grant.permission_keys.contains(&permission_key)
        {
            grant.permission_keys.push(permission_key);
        }
    }

    Ok(grants.into_values().collect())
}
