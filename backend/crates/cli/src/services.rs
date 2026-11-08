//! Services the management commands need: password hashing/policy and the
//! audit writer (ADR-0003 §6).

use rs_admin_audit::AuditWriter;
use rs_admin_config::SecurityConfig;
use rs_admin_security::{AuditKeyRing, PasswordPolicy, PasswordService};
use std::time::Duration;

/// Password and audit services shared by both commands.
#[derive(Clone)]
pub struct CliServices {
    password_service: PasswordService,
    password_policy: PasswordPolicy,
    audit: AuditWriter,
    temporary_password_ttl: Duration,
}

impl CliServices {
    /// Build around an existing password service, so callers can share one
    /// bounded hashing pool.
    pub fn with_password_service(
        security: &SecurityConfig,
        password_service: PasswordService,
        temporary_password_ttl: Duration,
    ) -> Self {
        Self::from_parts(
            password_service,
            security.audit_chain.keys.clone(),
            temporary_password_ttl,
        )
    }

    /// Build from the only key material these commands use: the audit chain
    /// keyring. The CLI never signs tokens or derives refresh tokens.
    pub fn from_parts(
        password_service: PasswordService,
        audit_keys: AuditKeyRing,
        temporary_password_ttl: Duration,
    ) -> Self {
        Self {
            password_service,
            password_policy: PasswordPolicy::new(),
            audit: AuditWriter::new(audit_keys),
            temporary_password_ttl,
        }
    }

    pub fn password_service(&self) -> &PasswordService {
        &self.password_service
    }

    pub fn password_policy(&self) -> &PasswordPolicy {
        &self.password_policy
    }

    pub fn audit(&self) -> &AuditWriter {
        &self.audit
    }

    pub fn temporary_password_ttl(&self) -> Duration {
        self.temporary_password_ttl
    }
}
