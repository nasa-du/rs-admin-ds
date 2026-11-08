//! `bootstrap-super-admin`: create or promote the first super administrator
//! (ADR-0003 §6).
//!
//! The command is refused while any enabled super administrator exists. On an
//! empty database it creates the user; when the email already exists it
//! promotes that user instead. In both cases the operator-supplied password
//! becomes a temporary password that must be changed at first login.

use crate::credentials::{hash_temporary_password, reset_credentials};
use crate::error::CliError;
use crate::services::CliServices;
use chrono::{DateTime, Utc};
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, TargetType};
use rs_admin_domain::identity::{Email, Username};
use rs_admin_domain::rbac::BuiltInRole;
use rs_admin_store::{admin, rbac};
use secrecy::SecretString;
use sqlx::PgPool;
use uuid::Uuid;

/// Inputs for a bootstrap run. The password is wrapped so it cannot be logged
/// accidentally.
pub struct BootstrapInput<'a> {
    pub email: &'a str,
    pub username: &'a str,
    pub password: &'a SecretString,
}

/// Result of a successful bootstrap.
#[derive(Clone, Debug)]
pub struct BootstrapOutcome {
    pub user_id: Uuid,
    pub username: String,
    pub email: String,
    /// `true` when the user was created; `false` when an existing user was
    /// promoted.
    pub created: bool,
    pub temporary_password_expires_at: DateTime<Utc>,
    pub sessions_revoked: u64,
}

/// Create the first super administrator, or promote the user that owns the
/// given email. Refuses while an enabled super administrator exists.
pub async fn bootstrap_super_admin(
    pool: &PgPool,
    services: &CliServices,
    input: BootstrapInput<'_>,
) -> Result<BootstrapOutcome, CliError> {
    let username = Username::parse(input.username).map_err(CliError::InvalidUsername)?;
    let email = Email::parse(input.email).map_err(CliError::InvalidEmail)?;

    // A fresh database has no built-in roles until the API starts, so the CLI
    // mirrors the code-owned catalog before assigning them.
    rbac::sync_catalog(pool)
        .await
        .map_err(CliError::CatalogSync)?;

    let mut transaction = pool.begin().await?;
    admin::lock_bootstrap(&mut transaction).await?;

    if admin::count_enabled_super_admins(&mut *transaction).await? > 0 {
        return Err(CliError::SuperAdminExists);
    }

    let existing = admin::lock_user_by_email(&mut transaction, email.as_str()).await?;

    let (user_id, actual_username, created, sessions_revoked, expires_at) = match existing {
        Some(user) => {
            // The requested username may not belong to a different user. The
            // promoted user keeps its existing username, so the password
            // policy runs against the stored identifiers and display name.
            if let Some(owner) =
                admin::find_user_id_by_username(&mut *transaction, username.as_str()).await?
                && owner != user.id
            {
                return Err(CliError::UsernameTaken);
            }

            let password_hash = hash_temporary_password(
                services,
                input.password,
                &[&user.username, &user.email, &user.display_name],
            )
            .await?;
            let reset =
                reset_credentials(&mut transaction, services, user.id, &password_hash).await?;

            for role in [BuiltInRole::User, BuiltInRole::SuperAdmin] {
                admin::assign_builtin_role(&mut *transaction, user.id, role).await?;
            }

            (
                user.id,
                user.username,
                false,
                reset.sessions_revoked,
                reset.expires_at,
            )
        }
        None => {
            if admin::find_user_id_by_username(&mut *transaction, username.as_str())
                .await?
                .is_some()
            {
                return Err(CliError::UsernameTaken);
            }

            // The new user's display name defaults to the username.
            let password_hash = hash_temporary_password(
                services,
                input.password,
                &[username.as_str(), email.as_str(), username.as_str()],
            )
            .await?;
            let (user_id, expires_at) = admin::insert_cli_user(
                &mut transaction,
                username.as_str(),
                email.as_str(),
                &password_hash,
                services.temporary_password_ttl(),
            )
            .await?;

            for role in [BuiltInRole::User, BuiltInRole::SuperAdmin] {
                admin::assign_builtin_role(&mut *transaction, user_id, role).await?;
            }

            (user_id, username.as_str().to_owned(), true, 0, expires_at)
        }
    };

    let event = AuditEvent::builder(
        AuditAction::SystemBootstrapSuperAdmin,
        AuditResult::Success,
        AuditActor::cli(),
        Uuid::now_v7(),
    )
    .target(
        TargetType::User,
        Some(user_id),
        Some(actual_username.clone()),
    )
    .detail("target_user_id", user_id.to_string())
    .detail("created", created)
    .detail("sessions_revoked_count", sessions_revoked as i64)
    .build()?;
    services
        .audit()
        .write_in_tx(&mut transaction, &event)
        .await?;

    transaction.commit().await?;

    Ok(BootstrapOutcome {
        user_id,
        username: actual_username,
        email: email.as_str().to_owned(),
        created,
        temporary_password_expires_at: expires_at,
        sessions_revoked,
    })
}
