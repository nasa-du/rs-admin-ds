//! SQL for authentication: identity lookup, sessions and refresh tokens
//! (docs/specs/login-system.md §4.2–§5.3).
//!
//! Handlers never inspect SQL errors directly; every failure surfaces as
//! [`AuthStoreError`], whose display text is stable and contains no SQL or
//! connection details.

use chrono::{DateTime, Utc};
use rs_admin_security::account_lock_seconds;
use sqlx::{Executor, PgPool, Postgres, Transaction};
use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::time::Duration;
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

/// A refresh-token row as seen while walking a session's rotation chain.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct RefreshTokenRow {
    pub session_id: Uuid,
    pub rotation_counter: i64,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub replaced_by_hash: Option<Vec<u8>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// A presented refresh token plus the session and user state needed to decide
/// whether it may rotate. The session row is locked, serializing every refresh
/// of one token family across instances.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct RefreshContext {
    #[sqlx(flatten)]
    pub token: RefreshTokenRow,
    pub user_id: Uuid,
    pub username: String,
    pub status: String,
    pub token_version: i64,
    pub last_seen_at: DateTime<Utc>,
    pub idle_expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
    pub authenticated_at: DateTime<Utc>,
    pub session_revoked_at: Option<DateTime<Utc>>,
}

impl RefreshContext {
    /// Mirrors [`AuthUser::is_active`] for the joined user row.
    pub fn user_is_active(&self) -> bool {
        self.status == "active"
    }
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

/// Lock the user row for the rest of the transaction. Serializing logins,
/// reauthentication and logout-all per user keeps the token version and
/// session cap checks consistent.
pub async fn lock_user_row(
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

/// Count a user's usable sessions: unrevoked and not yet idle- or
/// absolute-expired. This is the value the per-user session cap is enforced
/// against, matching the "active" state in the session state machine.
pub async fn count_active_sessions(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<i64, AuthStoreError> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions
         WHERE user_id = $1
           AND revoked_at IS NULL
           AND idle_expires_at > now()
           AND absolute_expires_at > now()",
    )
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)
}

/// Ids of the user's least recently active usable sessions, oldest first.
/// Callers hold the user row lock, so the selection cannot race a login.
/// Expired sessions are not candidates: the state machine already treats them
/// as terminal. UUIDv7 ids break ties so the order is stable.
pub async fn find_oldest_active_sessions(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<Uuid>, AuthStoreError> {
    sqlx::query_scalar(
        "SELECT id FROM auth_sessions
         WHERE user_id = $1
           AND revoked_at IS NULL
           AND idle_expires_at > now()
           AND absolute_expires_at > now()
         ORDER BY last_seen_at, id
         LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(&mut **transaction)
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

/// Next rotation counter for a session's token family, used when the server
/// must mint a fresh root instead of deriving from a presented token.
pub async fn next_rotation_counter(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
) -> Result<i64, AuthStoreError> {
    sqlx::query_scalar(
        "SELECT COALESCE(MAX(rotation_counter) + 1, 0)
         FROM auth_refresh_tokens WHERE session_id = $1",
    )
    .bind(session_id)
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

/// Lock a refresh token together with its session and user for the refresh
/// transaction. The session row is the lock: every refresh of one token family
/// serializes on it, so concurrent rotations cannot branch the chain.
///
/// The lock is acquired first and the joined state is read in a subsequent
/// statement. Reading the join while waiting would return the snapshot from
/// before the wait and miss a rotation that committed in the meantime.
pub async fn lock_refresh_context(
    transaction: &mut Transaction<'_, Postgres>,
    token_hash: &[u8; 32],
) -> Result<Option<RefreshContext>, AuthStoreError> {
    let session_id: Option<Uuid> =
        sqlx::query_scalar("SELECT session_id FROM auth_refresh_tokens WHERE token_hash = $1")
            .bind(token_hash.as_slice())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(AuthStoreError::Database)?;
    let Some(session_id) = session_id else {
        return Ok(None);
    };

    let locked: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM auth_sessions WHERE id = $1 FOR UPDATE")
            .bind(session_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(AuthStoreError::Database)?;
    if locked.is_none() {
        return Ok(None);
    }

    sqlx::query_as(
        "SELECT t.session_id, t.rotation_counter, t.expires_at, t.used_at,
                t.replaced_by_hash, t.revoked_at,
                s.user_id, s.last_seen_at, s.idle_expires_at,
                s.absolute_expires_at, s.authenticated_at,
                s.revoked_at AS session_revoked_at,
                u.username, u.status, u.token_version
         FROM auth_refresh_tokens t
         JOIN auth_sessions s ON s.id = t.session_id
         JOIN users u ON u.id = s.user_id
         WHERE t.token_hash = $1",
    )
    .bind(token_hash.as_slice())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)
}

/// Wall-clock time from the database (`clock_timestamp()`), not the
/// transaction-start time returned by `now()`. Refresh uses this after winning
/// the session lock so grace and expiry comparisons describe the moment the
/// token was actually handled.
pub async fn database_clock_now(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<DateTime<Utc>, AuthStoreError> {
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **transaction)
        .await
        .map_err(AuthStoreError::Database)
}

/// Load a refresh-token row by hash while walking the derivation chain. Callers
/// already hold the session lock from [`lock_refresh_context`].
pub async fn find_refresh_token(
    transaction: &mut Transaction<'_, Postgres>,
    token_hash: &[u8; 32],
) -> Result<Option<RefreshTokenRow>, AuthStoreError> {
    sqlx::query_as(
        "SELECT session_id, rotation_counter, expires_at, used_at,
                replaced_by_hash, revoked_at
         FROM auth_refresh_tokens WHERE token_hash = $1",
    )
    .bind(token_hash.as_slice())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)
}

/// Mark a token as rotated and point it at its successor. Returns `false` when
/// the row was already used or revoked, which means another writer won the
/// rotation.
pub async fn mark_refresh_token_rotated(
    transaction: &mut Transaction<'_, Postgres>,
    token_hash: &[u8; 32],
    replaced_by_hash: &[u8; 32],
    used_at: DateTime<Utc>,
) -> Result<bool, AuthStoreError> {
    let result = sqlx::query(
        "UPDATE auth_refresh_tokens
         SET used_at = $2, replaced_by_hash = $3
         WHERE token_hash = $1 AND used_at IS NULL AND revoked_at IS NULL",
    )
    .bind(token_hash.as_slice())
    .bind(used_at)
    .bind(replaced_by_hash.as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(result.rows_affected() == 1)
}

/// Session and token revocation reason when refresh reuse is detected
/// (spec §5.3).
pub const REVOKE_REASON_TOKEN_REUSE: &str = "token_reuse";

/// Revocation reason for a user ending their current session (spec §5.9).
pub const REVOKE_REASON_LOGOUT: &str = "logout";

/// Revocation reason for a user ending every session they own (spec §5.9).
pub const REVOKE_REASON_LOGOUT_ALL: &str = "logout_all";

/// Revocation reason for a session pushed out by the per-user session cap
/// (spec §5.3).
pub const REVOKE_REASON_EXCESS_SESSIONS: &str = "excess_sessions";

/// Revocation reason for the other sessions of a user who changed their own
/// password (spec §5.3, §5.9).
pub const REVOKE_REASON_PASSWORD_CHANGE: &str = "password_change";

/// Revocation reason for an administrative sweep: disabling, credential reset
/// and the CLI commands (spec §5.9).
pub const REVOKE_REASON_ADMIN_REVOKE: &str = "admin_revoke";

/// Revoke every token in a session's family with the given reason.
pub async fn revoke_refresh_family(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    reason: &str,
) -> Result<u64, AuthStoreError> {
    let result = sqlx::query(
        "UPDATE auth_refresh_tokens
         SET revoked_at = now(), revoked_reason = $2
         WHERE session_id = $1 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(reason)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(result.rows_affected())
}

/// Revoke a session with an explicit reason. Returns whether this call was the
/// one that revoked it.
pub async fn revoke_session(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    reason: &str,
) -> Result<bool, AuthStoreError> {
    let result = sqlx::query(
        "UPDATE auth_sessions
         SET revoked_at = now(), revoke_reason = $2
         WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(reason)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(result.rows_affected() == 1)
}

/// Revoke a session together with every refresh token in its family. Returns
/// whether this call was the one that revoked the session.
pub async fn revoke_session_family(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    reason: &str,
) -> Result<bool, AuthStoreError> {
    revoke_refresh_family(transaction, session_id, reason).await?;
    revoke_session(transaction, session_id, reason).await
}

/// Revoke every unrevoked session a user owns and return how many changed
/// state. Callers must hold the user row lock so a concurrent login cannot
/// create a session between this sweep and the token-version bump.
pub async fn revoke_user_sessions(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    reason: &str,
) -> Result<u64, AuthStoreError> {
    let result = sqlx::query(
        "UPDATE auth_sessions
         SET revoked_at = now(), revoke_reason = $2
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(reason)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(result.rows_affected())
}

/// Revoke every unrevoked refresh token belonging to a user's sessions.
pub async fn revoke_user_refresh_tokens(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    reason: &str,
) -> Result<u64, AuthStoreError> {
    let result = sqlx::query(
        "UPDATE auth_refresh_tokens t
         SET revoked_at = now(), revoked_reason = $2
         FROM auth_sessions s
         WHERE t.session_id = s.id AND s.user_id = $1 AND t.revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(reason)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(result.rows_affected())
}

/// Revoke every unrevoked session a user owns except the one kept by a
/// self-service password change. Callers must hold the user row lock so a
/// concurrent login cannot slip a new session past the sweep.
pub async fn revoke_other_user_sessions(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    keep_session_id: Uuid,
    reason: &str,
) -> Result<u64, AuthStoreError> {
    let result = sqlx::query(
        "UPDATE auth_sessions
         SET revoked_at = now(), revoke_reason = $3
         WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(keep_session_id)
    .bind(reason)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(result.rows_affected())
}

/// Revoke every unrevoked refresh token belonging to a user's other sessions.
pub async fn revoke_other_user_refresh_tokens(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    keep_session_id: Uuid,
    reason: &str,
) -> Result<u64, AuthStoreError> {
    let result = sqlx::query(
        "UPDATE auth_refresh_tokens t
         SET revoked_at = now(), revoked_reason = $3
         FROM auth_sessions s
         WHERE t.session_id = s.id
           AND s.user_id = $1
           AND s.id <> $2
           AND t.revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(keep_session_id)
    .bind(reason)
    .execute(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(result.rows_affected())
}

/// Increment `users.token_version`, invalidating every access token issued so
/// far for the user. Returns the new version.
pub async fn bump_token_version(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<i64, AuthStoreError> {
    sqlx::query_scalar(
        "UPDATE users SET token_version = token_version + 1, updated_at = now()
         WHERE id = $1
         RETURNING token_version",
    )
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)
}

/// Stamp a session's activity and slide its idle deadline. Callers apply the
/// 60-second throttle before invoking this.
pub async fn touch_session_activity(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    last_seen_at: DateTime<Utc>,
    idle_expires_at: DateTime<Utc>,
) -> Result<(), AuthStoreError> {
    sqlx::query(
        "UPDATE auth_sessions
         SET last_seen_at = $2, idle_expires_at = $3
         WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(last_seen_at)
    .bind(idle_expires_at)
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

/// Lock a session row for the rest of the transaction. Returns `None` when the
/// session is already revoked or idle- or absolute-expired. The caller then
/// holds the same lock a refresh transaction uses, so a password change and a
/// token rotation cannot interleave.
pub async fn lock_active_session(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<Option<AuthSession>, AuthStoreError> {
    sqlx::query_as(
        "SELECT id, user_id, created_at, last_seen_at, idle_expires_at,
                absolute_expires_at, authenticated_at
         FROM auth_sessions
         WHERE id = $1 AND user_id = $2
           AND revoked_at IS NULL
           AND idle_expires_at > clock_timestamp()
           AND absolute_expires_at > clock_timestamp()
         FOR UPDATE",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(&mut **transaction)
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

/// Finish a self-service password change in one write: replace the hash, clear
/// the temporary-password markers and bump the token version. Account-level
/// failure state is left untouched; only login and reauthentication reset it
/// (spec §8.3). The expected hash guards against a credential reset that
/// committed after the caller verified the password. Returns the new token
/// version, or `None` when the stored hash changed.
pub async fn complete_password_change(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    expected_hash: &str,
    new_hash: &str,
) -> Result<Option<i64>, AuthStoreError> {
    sqlx::query_scalar(
        "UPDATE users
         SET password_hash = $3,
             must_change_password = false,
             temporary_password_expires_at = NULL,
             token_version = token_version + 1,
             updated_at = now()
         WHERE id = $1 AND password_hash = $2
         RETURNING token_version",
    )
    .bind(user_id)
    .bind(expected_hash)
    .bind(new_hash)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)
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

/// The two shared throttle rows in `auth_login_throttle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrottleScope {
    Ip,
    Global,
}

impl ThrottleScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ip => "ip",
            Self::Global => "global",
        }
    }
}

/// Outcome of an atomic account- or throttle-layer failure update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureRecord {
    /// The failure was counted. `retry_after_seconds` is present when this
    /// failure crossed a lock threshold.
    Counted {
        failed_count: i32,
        retry_after_seconds: Option<i64>,
    },
    /// Another writer locked the scope first; the count was left unchanged.
    AlreadyLocked { retry_after_seconds: i64 },
}

/// IP- and global-layer lock state for a login attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoginThrottleLocks {
    pub ip_retry_after_seconds: Option<i64>,
    pub global_retry_after_seconds: Option<i64>,
}

impl LoginThrottleLocks {
    /// Seconds the attempt must wait, taking the stricter layer.
    pub fn retry_after_seconds(&self) -> Option<i64> {
        self.ip_retry_after_seconds
            .into_iter()
            .chain(self.global_retry_after_seconds)
            .max()
    }
}

/// Seconds remaining on a known user's account lock, or `None` when unlocked.
pub async fn known_account_lock_seconds<'e, E>(
    executor: E,
    user_id: Uuid,
) -> Result<Option<i64>, AuthStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar(
        "SELECT CEIL(EXTRACT(EPOCH FROM (locked_until - now())))::bigint
         FROM users
         WHERE id = $1 AND locked_until > now()",
    )
    .bind(user_id)
    .fetch_optional(executor)
    .await
    .map_err(AuthStoreError::Database)
}

/// Seconds remaining on an unknown identifier's account lock.
pub async fn unknown_account_lock_seconds<'e, E>(
    executor: E,
    key_hash: &[u8; 32],
) -> Result<Option<i64>, AuthStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar(
        "SELECT CEIL(EXTRACT(EPOCH FROM (locked_until - now())))::bigint
         FROM auth_login_failures
         WHERE key_hash = $1 AND locked_until > now()",
    )
    .bind(key_hash.as_slice())
    .fetch_optional(executor)
    .await
    .map_err(AuthStoreError::Database)
}

/// Atomically count a failure for a known user and apply the account-layer
/// lock thresholds (spec §8.3). A count older than `decay` restarts at one; a
/// currently locked account is left untouched.
pub async fn record_known_login_failure(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    decay: Duration,
) -> Result<FailureRecord, AuthStoreError> {
    let failed_count: Option<i32> = sqlx::query_scalar(
        "UPDATE users
         SET failed_login_count = CASE
                 WHEN last_failed_login_at IS NULL
                      OR last_failed_login_at <= now() - make_interval(secs => $2)
                 THEN 1
                 ELSE failed_login_count + 1
             END,
             last_failed_login_at = now(),
             updated_at = now()
         WHERE id = $1
           AND (locked_until IS NULL OR locked_until <= now())
         RETURNING failed_login_count",
    )
    .bind(user_id)
    .bind(decay.as_secs_f64())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;

    let Some(failed_count) = failed_count else {
        let retry_after_seconds = known_account_lock_seconds(&mut **transaction, user_id)
            .await?
            .unwrap_or(1);
        return Ok(FailureRecord::AlreadyLocked {
            retry_after_seconds,
        });
    };

    let retry_after_seconds = match account_lock_seconds(failed_count) {
        Some(seconds) => {
            sqlx::query(
                "UPDATE users
                 SET locked_until = now() + make_interval(secs => $2),
                     updated_at = now()
                 WHERE id = $1",
            )
            .bind(user_id)
            .bind(seconds as f64)
            .execute(&mut **transaction)
            .await
            .map_err(AuthStoreError::Database)?;
            Some(seconds)
        }
        None => None,
    };

    Ok(FailureRecord::Counted {
        failed_count,
        retry_after_seconds,
    })
}

/// Atomically count a failure for an unknown identifier. Unknown identifiers
/// use `auth_login_failures` with the same thresholds and decay as known users.
pub async fn record_unknown_login_failure(
    transaction: &mut Transaction<'_, Postgres>,
    key_hash: &[u8; 32],
    decay: Duration,
) -> Result<FailureRecord, AuthStoreError> {
    let failed_count: Option<i32> = sqlx::query_scalar(
        "INSERT INTO auth_login_failures AS f (key_hash, failed_count, locked_until, updated_at)
         VALUES ($1, 1, NULL, now())
         ON CONFLICT (key_hash) DO UPDATE SET
             failed_count = CASE
                 WHEN f.updated_at <= now() - make_interval(secs => $2)
                 THEN 1
                 ELSE f.failed_count + 1
             END,
             updated_at = now(),
             locked_until = CASE
                 WHEN f.locked_until > now() THEN f.locked_until ELSE NULL
             END
         WHERE f.locked_until IS NULL OR f.locked_until <= now()
         RETURNING failed_count",
    )
    .bind(key_hash.as_slice())
    .bind(decay.as_secs_f64())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;

    let Some(failed_count) = failed_count else {
        let retry_after_seconds = unknown_account_lock_seconds(&mut **transaction, key_hash)
            .await?
            .unwrap_or(1);
        return Ok(FailureRecord::AlreadyLocked {
            retry_after_seconds,
        });
    };

    let retry_after_seconds = match account_lock_seconds(failed_count) {
        Some(seconds) => {
            sqlx::query(
                "UPDATE auth_login_failures
                 SET locked_until = now() + make_interval(secs => $2),
                     updated_at = now()
                 WHERE key_hash = $1",
            )
            .bind(key_hash.as_slice())
            .bind(seconds as f64)
            .execute(&mut **transaction)
            .await
            .map_err(AuthStoreError::Database)?;
            Some(seconds)
        }
        None => None,
    };

    Ok(FailureRecord::Counted {
        failed_count,
        retry_after_seconds,
    })
}

/// Seconds remaining on one throttle scope's lock, or `None` when it is not
/// locked.
pub async fn load_throttle_lock(
    pool: &PgPool,
    scope: ThrottleScope,
    key_hash: &[u8; 32],
) -> Result<Option<i64>, AuthStoreError> {
    sqlx::query_scalar(
        "SELECT CEIL(EXTRACT(EPOCH FROM (locked_until - now())))::bigint
         FROM auth_login_throttle
         WHERE scope = $1 AND key_hash = $2 AND locked_until > now()",
    )
    .bind(scope.as_str())
    .bind(key_hash.as_slice())
    .fetch_optional(pool)
    .await
    .map_err(AuthStoreError::Database)
}

/// Load the lock state of the IP and global layers.
pub async fn load_login_throttle_locks(
    pool: &PgPool,
    ip_key: &[u8; 32],
    global_key: &[u8; 32],
) -> Result<LoginThrottleLocks, AuthStoreError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT scope, CEIL(EXTRACT(EPOCH FROM (locked_until - now())))::bigint
         FROM auth_login_throttle
         WHERE locked_until > now()
           AND ((scope = 'ip' AND key_hash = $1)
                OR (scope = 'global' AND key_hash = $2))",
    )
    .bind(ip_key.as_slice())
    .bind(global_key.as_slice())
    .fetch_all(pool)
    .await
    .map_err(AuthStoreError::Database)?;

    let mut locks = LoginThrottleLocks::default();
    for (scope, seconds) in rows {
        match scope.as_str() {
            "ip" => locks.ip_retry_after_seconds = Some(seconds),
            "global" => locks.global_retry_after_seconds = Some(seconds),
            _ => {}
        }
    }
    Ok(locks)
}

/// Atomically count one failure in a fixed window for the IP or global layer.
///
/// The `ON CONFLICT ... DO UPDATE ... RETURNING` statement is the unit of
/// serialization; no `SELECT count(*)` precedes the write. A locked scope
/// returns no row, which maps to [`FailureRecord::AlreadyLocked`] without
/// touching the counter.
pub async fn record_throttle_failure(
    transaction: &mut Transaction<'_, Postgres>,
    scope: ThrottleScope,
    key_hash: &[u8; 32],
    window: Duration,
    lock: Duration,
    limit: i64,
) -> Result<FailureRecord, AuthStoreError> {
    let failures: Option<i32> = sqlx::query_scalar(
        "INSERT INTO auth_login_throttle AS t
             (scope, key_hash, window_start, failures, locked_until, last_failure_at)
         VALUES ($1, $2, now(), 1, NULL, now())
         ON CONFLICT (scope, key_hash) DO UPDATE SET
             window_start = CASE
                 WHEN t.window_start <= now() - make_interval(secs => $3)
                 THEN now()
                 ELSE t.window_start
             END,
             failures = CASE
                 WHEN t.window_start <= now() - make_interval(secs => $3)
                 THEN 1
                 ELSE t.failures + 1
             END,
             locked_until = CASE
                 WHEN t.locked_until > now() THEN t.locked_until ELSE NULL
             END,
             last_failure_at = now()
         WHERE t.locked_until IS NULL OR t.locked_until <= now()
         RETURNING failures",
    )
    .bind(scope.as_str())
    .bind(key_hash.as_slice())
    .bind(window.as_secs_f64())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(AuthStoreError::Database)?;

    let Some(failures) = failures else {
        let retry_after_seconds = throttle_lock_seconds(&mut **transaction, scope, key_hash)
            .await?
            .unwrap_or(1);
        return Ok(FailureRecord::AlreadyLocked {
            retry_after_seconds,
        });
    };

    let retry_after_seconds = if i64::from(failures) >= limit {
        let seconds = sqlx::query_scalar(
            "UPDATE auth_login_throttle
             SET locked_until = now() + make_interval(secs => $3)
             WHERE scope = $1 AND key_hash = $2
             RETURNING CEIL(EXTRACT(EPOCH FROM (locked_until - now())))::bigint",
        )
        .bind(scope.as_str())
        .bind(key_hash.as_slice())
        .bind(lock.as_secs_f64())
        .fetch_one(&mut **transaction)
        .await
        .map_err(AuthStoreError::Database)?;
        Some(seconds)
    } else {
        None
    };

    Ok(FailureRecord::Counted {
        failed_count: failures,
        retry_after_seconds,
    })
}

async fn throttle_lock_seconds<'e, E>(
    executor: E,
    scope: ThrottleScope,
    key_hash: &[u8; 32],
) -> Result<Option<i64>, AuthStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar(
        "SELECT CEIL(EXTRACT(EPOCH FROM (locked_until - now())))::bigint
         FROM auth_login_throttle
         WHERE scope = $1 AND key_hash = $2 AND locked_until > now()",
    )
    .bind(scope.as_str())
    .bind(key_hash.as_slice())
    .fetch_optional(executor)
    .await
    .map_err(AuthStoreError::Database)
}

/// Clear the global circuit once its lock has elapsed and report whether this
/// caller observed the release. The row keeps its window count, implementing a
/// half-open probe: a later failure inside the same window is a fresh
/// threshold crossing and re-opens the circuit for another lock period.
/// Exactly one instance wins the update, so each release is announced once.
pub async fn release_expired_global_throttle(pool: &PgPool) -> Result<bool, AuthStoreError> {
    let released: Option<i32> = sqlx::query_scalar(
        "UPDATE auth_login_throttle
         SET locked_until = NULL
         WHERE scope = $1 AND locked_until IS NOT NULL AND locked_until <= now()
         RETURNING failures",
    )
    .bind(ThrottleScope::Global.as_str())
    .fetch_optional(pool)
    .await
    .map_err(AuthStoreError::Database)?;
    Ok(released.is_some())
}

/// Clear the unknown-identifier account row after a successful login. A user
/// created after earlier failures must not inherit the stale lock state.
pub async fn clear_unknown_login_failures(
    transaction: &mut Transaction<'_, Postgres>,
    key_hash: &[u8; 32],
) -> Result<(), AuthStoreError> {
    sqlx::query("DELETE FROM auth_login_failures WHERE key_hash = $1")
        .bind(key_hash.as_slice())
        .execute(&mut **transaction)
        .await
        .map_err(AuthStoreError::Database)?;
    Ok(())
}
