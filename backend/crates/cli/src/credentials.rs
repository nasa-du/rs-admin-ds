//! Shared credential operations for the two commands.
//!
//! Both a bootstrap promotion and a break-glass recovery replace the password,
//! enable the account, clear the login lock, revoke every session and bump the
//! token version. Keeping the sequence in one place prevents the two commands
//! from drifting apart.

use crate::error::CliError;
use crate::services::CliServices;
use chrono::{DateTime, Utc};
use rs_admin_store::admin;
use rs_admin_store::auth::{
    REVOKE_REASON_ADMIN_REVOKE, bump_token_version, revoke_user_refresh_tokens,
    revoke_user_sessions,
};
use secrecy::{ExposeSecret, SecretString};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Validate the temporary password against the policy, including every login
/// identifier and the display name (spec §8.2), then hash it.
pub(crate) async fn hash_temporary_password(
    services: &CliServices,
    password: &SecretString,
    identifiers: &[&str],
) -> Result<String, CliError> {
    services
        .password_policy()
        .validate(password.expose_secret(), identifiers)
        .map_err(|violation| CliError::PasswordPolicyViolation(violation.as_reason()))?;
    services
        .password_service()
        .hash(password.expose_secret())
        .await
        .map_err(CliError::Hashing)
}

/// Result of replacing an existing user's credentials.
pub(crate) struct ResetCredentials {
    pub sessions_revoked: u64,
    pub token_version: i64,
    pub expires_at: DateTime<Utc>,
}

/// Revoke every session and refresh token, invalidate every access token and
/// install the new temporary password. Callers must already hold the user row
/// lock when they need to serialize against logins and resets.
pub(crate) async fn reset_credentials(
    transaction: &mut Transaction<'_, Postgres>,
    services: &CliServices,
    user_id: Uuid,
    password_hash: &str,
) -> Result<ResetCredentials, CliError> {
    let sessions_revoked =
        revoke_user_sessions(transaction, user_id, REVOKE_REASON_ADMIN_REVOKE).await?;
    revoke_user_refresh_tokens(transaction, user_id, REVOKE_REASON_ADMIN_REVOKE).await?;
    let token_version = bump_token_version(transaction, user_id).await?;
    let expires_at = admin::set_temporary_credentials(
        transaction,
        user_id,
        password_hash,
        services.temporary_password_ttl(),
    )
    .await?;

    Ok(ResetCredentials {
        sessions_revoked,
        token_version,
        expires_at,
    })
}
