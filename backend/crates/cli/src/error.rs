//! Stable CLI errors.
//!
//! Messages never contain passwords, hashes, key material, SQL or connection
//! details. `Debug` is redacted to the same stable text so an accidental
//! `{:?}` cannot leak a source error.

use rs_admin_audit::AuditError;
use rs_admin_audit::AuditEventError;
use rs_admin_domain::identity::IdentityError;
use rs_admin_security::PasswordError;
use rs_admin_store::admin::AdminStoreError;
use rs_admin_store::auth::AuthStoreError;
use rs_admin_store::maintenance::MaintenanceError as MaintenanceStoreError;
use rs_admin_store::rbac::RbacSyncError;
use std::fmt;

#[derive(thiserror::Error)]
pub enum CliError {
    #[error("an enabled super administrator already exists")]
    SuperAdminExists,
    #[error("no user exists with that email")]
    UserNotFound,
    #[error("the target user is not a super administrator")]
    NotSuperAdmin,
    #[error("the username is already taken")]
    UsernameTaken,
    #[error("invalid username: {0}")]
    InvalidUsername(#[source] IdentityError),
    #[error("invalid email: {0}")]
    InvalidEmail(#[source] IdentityError),
    #[error("the temporary password does not satisfy the password policy ({0})")]
    PasswordPolicyViolation(&'static str),
    #[error("password hashing failed")]
    Hashing(#[source] PasswordError),
    #[error("the permission catalog could not be synchronized")]
    CatalogSync(#[source] RbacSyncError),
    #[error("database operation failed")]
    Database(#[source] AdminStoreError),
    #[error("session database operation failed")]
    SessionStore(#[source] AuthStoreError),
    #[error("audit event is invalid")]
    AuditEvent(#[source] AuditEventError),
    #[error("audit write failed")]
    Audit(#[source] AuditError),
    #[error("maintenance database operation failed")]
    Maintenance(#[source] MaintenanceStoreError),
    #[error("maintenance returned an incomplete purge interval")]
    InvalidMaintenanceOutcome,
}

impl fmt::Debug for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CliError")
            .field(&self.to_string())
            .finish()
    }
}

impl From<AdminStoreError> for CliError {
    fn from(error: AdminStoreError) -> Self {
        Self::Database(error)
    }
}

impl From<sqlx::Error> for CliError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(AdminStoreError::from(error))
    }
}

impl From<AuthStoreError> for CliError {
    fn from(error: AuthStoreError) -> Self {
        Self::SessionStore(error)
    }
}

impl From<MaintenanceStoreError> for CliError {
    fn from(error: MaintenanceStoreError) -> Self {
        Self::Maintenance(error)
    }
}

impl From<AuditEventError> for CliError {
    fn from(error: AuditEventError) -> Self {
        Self::AuditEvent(error)
    }
}

impl From<AuditError> for CliError {
    fn from(error: AuditError) -> Self {
        Self::Audit(error)
    }
}

impl From<PasswordError> for CliError {
    fn from(error: PasswordError) -> Self {
        Self::Hashing(error)
    }
}
