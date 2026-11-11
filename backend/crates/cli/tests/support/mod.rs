//! Shared fixtures for the management-CLI integration tests.
//!
//! The password service is process-wide so the expensive Argon2 parameters are
//! paid once per test binary.

#![allow(dead_code)]

use rs_admin_cli::CliServices;
use rs_admin_security::{AuditKeyRing, PasswordService};
use secrecy::SecretString;
use sqlx::PgPool;
use std::sync::OnceLock;
use std::time::Duration;
use uuid::Uuid;

pub const AUDIT_KEY_ID: &str = "test-audit-2026-11";
pub const AUDIT_KEY_B64: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
pub const PASSWORD: &str = "correct horse battery staple";

/// Wrap a test password the same way the CLI does.
pub fn secret(password: &str) -> SecretString {
    SecretString::from(password.to_owned())
}

pub fn password_service() -> &'static PasswordService {
    static SERVICE: OnceLock<PasswordService> = OnceLock::new();
    SERVICE.get_or_init(|| {
        PasswordService::with_queue_timeout(4, Duration::from_secs(120)).expect("password service")
    })
}

pub fn audit_keyring() -> AuditKeyRing {
    AuditKeyRing::from_json(
        &format!("{{\"{AUDIT_KEY_ID}\":\"{AUDIT_KEY_B64}\"}}"),
        AUDIT_KEY_ID,
    )
    .expect("audit keyring")
}

pub fn services() -> &'static CliServices {
    static SERVICES: OnceLock<CliServices> = OnceLock::new();
    SERVICES.get_or_init(|| {
        CliServices::from_parts(
            password_service().clone(),
            audit_keyring(),
            Duration::from_secs(72 * 3600),
        )
    })
}

/// Create the code-owned catalog and built-in roles. Bootstrap also does this,
/// but tests that insert users first need the roles to exist.
pub async fn prepare_catalog(pool: &PgPool) {
    rs_admin_store::rbac::sync_catalog(pool)
        .await
        .expect("catalog sync");
}

pub struct NewUser<'a> {
    pub username: &'a str,
    pub email: &'a str,
    pub password: &'a str,
    pub status: &'a str,
    /// Built-in role added in addition to the baseline `user` role.
    pub role: Option<&'a str>,
}

impl<'a> NewUser<'a> {
    pub fn active(username: &'a str, email: &'a str) -> Self {
        Self {
            username,
            email,
            password: PASSWORD,
            status: "active",
            role: None,
        }
    }
}

/// Insert a user and its baseline role (and an optional built-in role).
/// Requires [`prepare_catalog`].
pub async fn insert_user(pool: &PgPool, user: NewUser<'_>) -> Uuid {
    let hash = password_service()
        .hash(user.password)
        .await
        .expect("password hash");
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (
            username, email, display_name, password_hash, status
         ) VALUES ($1, $2, $1, $3, $4)
         RETURNING id",
    )
    .bind(user.username)
    .bind(user.email)
    .bind(hash)
    .bind(user.status)
    .fetch_one(pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id)
         SELECT $1, id FROM roles WHERE name = 'user'",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("baseline role");

    if let Some(role) = user.role.filter(|role| *role != "user") {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id)
             SELECT $1, id FROM roles WHERE name = $2",
        )
        .bind(id)
        .bind(role)
        .execute(pool)
        .await
        .expect("extra role");
    }

    id
}

/// Insert an active session and one refresh-token row. Returns the session id.
pub async fn insert_session(pool: &PgPool, user_id: Uuid) -> Uuid {
    let session_id: Uuid = sqlx::query_scalar(
        "INSERT INTO auth_sessions (
            user_id, idle_expires_at, absolute_expires_at, authenticated_at
         ) VALUES ($1, now() + interval '30 minutes', now() + interval '12 hours', now())
         RETURNING id",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("insert session");

    let token_hash = token_hash(session_id);
    sqlx::query(
        "INSERT INTO auth_refresh_tokens (token_hash, session_id, rotation_counter, expires_at)
         VALUES ($1, $2, 0, now() + interval '12 hours')",
    )
    .bind(token_hash.as_slice())
    .bind(session_id)
    .execute(pool)
    .await
    .expect("insert refresh token");

    session_id
}

/// Deterministic, unique token hash for a session.
pub fn token_hash(session_id: Uuid) -> [u8; 32] {
    let mut hash = [0u8; 32];
    for (slot, byte) in hash.iter_mut().zip(session_id.as_bytes()) {
        *slot = *byte;
    }
    hash
}

pub async fn user_roles(pool: &PgPool, user_id: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT r.name
         FROM user_roles ur
         JOIN roles r ON r.id = ur.role_id
         WHERE ur.user_id = $1
         ORDER BY r.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .expect("user roles")
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AuditRow {
    pub actor_user_id: Option<Uuid>,
    pub actor_label: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub target_label: Option<String>,
    pub result: String,
    pub severity: String,
    pub details: serde_json::Value,
}

pub async fn audit_rows(pool: &PgPool) -> Vec<AuditRow> {
    sqlx::query_as(
        "SELECT actor_user_id, actor_label, action, target_type, target_id,
                target_label, result, severity, details
         FROM audit_logs
         ORDER BY chain_seq",
    )
    .fetch_all(pool)
    .await
    .expect("audit rows")
}

pub async fn verify_audit_chain(pool: &PgPool) {
    let keys = AuditKeyRing::from_json(
        &format!("{{\"{AUDIT_KEY_ID}\":\"{AUDIT_KEY_B64}\"}}"),
        AUDIT_KEY_ID,
    )
    .expect("audit keyring");
    rs_admin_audit::verify_chain(pool, &keys)
        .await
        .expect("audit chain");
}
