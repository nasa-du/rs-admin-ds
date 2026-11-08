//! `recover-super-admin`: break-glass credential recovery for an existing
//! super administrator (ADR-0003 §6).
//!
//! The target must already hold the built-in `super_admin` role. Recovery
//! re-issues a temporary password, enables the account, clears the
//! account-level login lock, revokes every session and increments
//! `token_version`. Everything happens in one transaction with the audit row,
//! so a failure changes nothing.

use crate::credentials::{hash_temporary_password, reset_credentials};
use crate::error::CliError;
use crate::services::CliServices;
use chrono::{DateTime, Utc};
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, TargetType};
use rs_admin_domain::identity::Email;
use rs_admin_domain::rbac::BuiltInRole;
use rs_admin_store::admin;
use secrecy::SecretString;
use sqlx::PgPool;
use uuid::Uuid;

/// Inputs for a break-glass recovery. The password is wrapped so it cannot be
/// logged accidentally.
pub struct RecoverInput<'a> {
    pub email: &'a str,
    pub password: &'a SecretString,
}

/// Result of a successful recovery.
#[derive(Clone, Debug)]
pub struct RecoverOutcome {
    pub user_id: Uuid,
    pub username: String,
    pub email: String,
    pub temporary_password_expires_at: DateTime<Utc>,
    pub sessions_revoked: u64,
    pub token_version: i64,
}

/// Re-issue a temporary password for an existing super administrator, enable
/// the account, clear its login lock, revoke every session and invalidate
/// every access token. Non-super-admin targets are refused.
pub async fn recover_super_admin(
    pool: &PgPool,
    services: &CliServices,
    input: RecoverInput<'_>,
) -> Result<RecoverOutcome, CliError> {
    let email = Email::parse(input.email).map_err(CliError::InvalidEmail)?;

    let mut transaction = pool.begin().await?;
    let Some(user) = admin::lock_user_by_email(&mut transaction, email.as_str()).await? else {
        return Err(CliError::UserNotFound);
    };

    if !admin::user_holds_builtin_role(&mut *transaction, user.id, BuiltInRole::SuperAdmin).await? {
        return Err(CliError::NotSuperAdmin);
    }

    // The username and display name are part of the identifier-equality check,
    // so policy validation happens after the row is loaded. The row lock is
    // held across the Argon2 hash, serializing concurrent resets of this user.
    let password_hash = hash_temporary_password(
        services,
        input.password,
        &[&user.username, &user.email, &user.display_name],
    )
    .await?;
    let reset = reset_credentials(&mut transaction, services, user.id, &password_hash).await?;

    let event = AuditEvent::builder(
        AuditAction::SystemRecoverSuperAdmin,
        AuditResult::Success,
        AuditActor::cli(),
        Uuid::now_v7(),
    )
    .target(TargetType::User, Some(user.id), Some(user.username.clone()))
    .detail("target_user_id", user.id.to_string())
    .detail("recovered", true)
    .detail("sessions_revoked_count", reset.sessions_revoked as i64)
    .build()?;
    services
        .audit()
        .write_in_tx(&mut transaction, &event)
        .await?;

    transaction.commit().await?;

    Ok(RecoverOutcome {
        user_id: user.id,
        username: user.username,
        email: user.email,
        temporary_password_expires_at: reset.expires_at,
        sessions_revoked: reset.sessions_revoked,
        token_version: reset.token_version,
    })
}
