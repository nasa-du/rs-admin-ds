//! The closed audit catalog (spec §8.5.1/§8.5.2).
//!
//! Every event name, severity and target type is fixed here. Callers cannot
//! invent free-form action strings, and a table-driven test pins this module
//! to the specification.

use std::fmt;
use std::str::FromStr;

/// Returned when parsing a string that is not in a closed catalog.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown {kind} value")]
pub struct UnknownCatalogValue {
    pub kind: &'static str,
    pub value: String,
}

macro_rules! closed_string_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $( $variant:ident => $wire:literal ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $variant ),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[ $( Self::$variant ),+ ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire ),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = UnknownCatalogValue;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $( $wire => Ok(Self::$variant), )+
                    _ => Err(UnknownCatalogValue {
                        kind: stringify!($name),
                        value: value.to_owned(),
                    }),
                }
            }
        }
    };
}

closed_string_enum! {
    /// Audit severity (`info` < `warning` < `error` < `critical`).
    Severity {
        Info => "info",
        Warning => "warning",
        Error => "error",
        Critical => "critical",
    }
}

closed_string_enum! {
    /// Outcome recorded on an audit event.
    AuditResult {
        Success => "success",
        Failure => "failure",
        Denied => "denied",
    }
}

closed_string_enum! {
    /// Target type of an audit event. Unknown future values are tolerated when
    /// reading historical rows but cannot be written through this catalog.
    TargetType {
        User => "user",
        Role => "role",
        Session => "session",
        AuditLog => "audit_log",
        System => "system",
    }
}

/// How severity may be chosen for an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeverityRule {
    Fixed(Severity),
    /// Most changes use `default`; changes involving `admin`/`super_admin`
    /// escalate to `elevated`.
    Elevated {
        default: Severity,
        elevated: Severity,
    },
}

impl SeverityRule {
    pub const fn default_severity(self) -> Severity {
        match self {
            Self::Fixed(severity) => severity,
            Self::Elevated { default, .. } => default,
        }
    }

    pub fn allows(self, severity: Severity) -> bool {
        match self {
            Self::Fixed(fixed) => fixed == severity,
            Self::Elevated {
                default, elevated, ..
            } => severity == default || severity == elevated,
        }
    }
}

/// Which results are valid for an action (spec §8.5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultRule {
    Fixed(AuditResult),
    AnyOf(&'static [AuditResult]),
}

impl ResultRule {
    pub fn allows(self, result: AuditResult) -> bool {
        match self {
            Self::Fixed(fixed) => fixed == result,
            Self::AnyOf(allowed) => allowed.contains(&result),
        }
    }
}

/// Which target types an action may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetRule {
    Fixed(TargetType),
    Flexible(&'static [TargetType]),
}

impl TargetRule {
    pub fn allows(self, target: TargetType) -> bool {
        match self {
            Self::Fixed(fixed) => fixed == target,
            Self::Flexible(allowed) => allowed.contains(&target),
        }
    }
}

const fn fixed(severity: Severity) -> SeverityRule {
    SeverityRule::Fixed(severity)
}

const fn elevated(default: Severity, escalated: Severity) -> SeverityRule {
    SeverityRule::Elevated {
        default,
        elevated: escalated,
    }
}

const fn target(target: TargetType) -> TargetRule {
    TargetRule::Fixed(target)
}

const fn flexible(targets: &'static [TargetType]) -> TargetRule {
    TargetRule::Flexible(targets)
}

const fn result(result: AuditResult) -> ResultRule {
    ResultRule::Fixed(result)
}

const fn any_result(results: &'static [AuditResult]) -> ResultRule {
    ResultRule::AnyOf(results)
}

const USER_OR_SYSTEM: &[TargetType] = &[TargetType::User, TargetType::System];
const USER_ROLE_OR_SYSTEM: &[TargetType] =
    &[TargetType::User, TargetType::Role, TargetType::System];
const FAILURE_OR_DENIED: &[AuditResult] = &[AuditResult::Failure, AuditResult::Denied];

macro_rules! audit_actions {
    ($( $variant:ident => ($wire:literal, $severity:expr, $target:expr, $result:expr) ),+ $(,)?) => {
        /// Closed audit action catalog.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum AuditAction {
            $( $variant ),+
        }

        impl AuditAction {
            pub const ALL: &'static [Self] = &[ $( Self::$variant ),+ ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire ),+
                }
            }

            pub const fn severity_rule(self) -> SeverityRule {
                match self {
                    $( Self::$variant => $severity ),+
                }
            }

            pub const fn target_rule(self) -> TargetRule {
                match self {
                    $( Self::$variant => $target ),+
                }
            }

            pub const fn result_rule(self) -> ResultRule {
                match self {
                    $( Self::$variant => $result ),+
                }
            }

            pub const fn default_severity(self) -> Severity {
                self.severity_rule().default_severity()
            }
        }

        impl fmt::Display for AuditAction {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for AuditAction {
            type Err = UnknownCatalogValue;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $( $wire => Ok(Self::$variant), )+
                    _ => Err(UnknownCatalogValue {
                        kind: "AuditAction",
                        value: value.to_owned(),
                    }),
                }
            }
        }
    };
}

audit_actions! {
    AuthnLoginSuccess => (
        "authn_login_success",
        fixed(Severity::Info),
        target(TargetType::Session),
        result(AuditResult::Success)
    ),
    AuthnLoginFail => (
        "authn_login_fail",
        fixed(Severity::Warning),
        flexible(USER_OR_SYSTEM),
        any_result(FAILURE_OR_DENIED)
    ),
    AuthnLoginLock => (
        "authn_login_lock",
        fixed(Severity::Warning),
        flexible(USER_OR_SYSTEM),
        result(AuditResult::Denied)
    ),
    AuthnLoginRateLimited => (
        "authn_login_rate_limited",
        fixed(Severity::Warning),
        target(TargetType::System),
        result(AuditResult::Denied)
    ),
    AuthnLogout => (
        "authn_logout",
        fixed(Severity::Info),
        target(TargetType::Session),
        result(AuditResult::Success)
    ),
    AuthnLogoutAll => (
        "authn_logout_all",
        fixed(Severity::Info),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    AuthnRefreshReuseDetected => (
        "authn_refresh_reuse_detected",
        fixed(Severity::Critical),
        target(TargetType::Session),
        result(AuditResult::Failure)
    ),
    AuthnSessionEvicted => (
        "authn_session_evicted",
        fixed(Severity::Info),
        target(TargetType::Session),
        result(AuditResult::Success)
    ),
    AuthnPasswordChanged => (
        "authn_password_changed",
        fixed(Severity::Info),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    AuthnPasswordChangeFail => (
        "authn_password_change_fail",
        fixed(Severity::Warning),
        target(TargetType::User),
        result(AuditResult::Failure)
    ),
    AuthnReauthSuccess => (
        "authn_reauth_success",
        fixed(Severity::Info),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    AuthnReauthFail => (
        "authn_reauth_fail",
        fixed(Severity::Warning),
        target(TargetType::User),
        result(AuditResult::Failure)
    ),
    AuthzStepUpDenied => (
        "authz_step_up_denied",
        fixed(Severity::Warning),
        flexible(USER_ROLE_OR_SYSTEM),
        result(AuditResult::Denied)
    ),
    AuthzDenied => (
        "authz_denied",
        fixed(Severity::Warning),
        flexible(USER_ROLE_OR_SYSTEM),
        result(AuditResult::Denied)
    ),
    UserCreated => (
        "user_created",
        fixed(Severity::Info),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    UserUpdated => (
        "user_updated",
        fixed(Severity::Info),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    UserStatusChanged => (
        "user_status_changed",
        fixed(Severity::Warning),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    UserPasswordReset => (
        "user_password_reset",
        fixed(Severity::Critical),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    UserUnlocked => (
        "user_unlocked",
        fixed(Severity::Warning),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    UserRolesChanged => (
        "user_roles_changed",
        elevated(Severity::Warning, Severity::Critical),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    UserSessionsRevoked => (
        "user_sessions_revoked",
        fixed(Severity::Warning),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    RoleCreated => (
        "role_created",
        fixed(Severity::Warning),
        target(TargetType::Role),
        result(AuditResult::Success)
    ),
    RoleUpdated => (
        "role_updated",
        fixed(Severity::Warning),
        target(TargetType::Role),
        result(AuditResult::Success)
    ),
    RolePermissionsChanged => (
        "role_permissions_changed",
        fixed(Severity::Critical),
        target(TargetType::Role),
        result(AuditResult::Success)
    ),
    RoleDeleted => (
        "role_deleted",
        fixed(Severity::Warning),
        target(TargetType::Role),
        result(AuditResult::Success)
    ),
    SystemBootstrapSuperAdmin => (
        "system_bootstrap_super_admin",
        fixed(Severity::Critical),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    SystemRecoverSuperAdmin => (
        "system_recover_super_admin",
        fixed(Severity::Critical),
        target(TargetType::User),
        result(AuditResult::Success)
    ),
    CryptoKeyRotated => (
        "crypto_key_rotated",
        fixed(Severity::Critical),
        target(TargetType::System),
        result(AuditResult::Success)
    ),
    SystemAuditLogsPurged => (
        "system_audit_logs_purged",
        fixed(Severity::Warning),
        target(TargetType::System),
        result(AuditResult::Success)
    ),
    AuditLogAccessed => (
        "audit_log_accessed",
        fixed(Severity::Info),
        target(TargetType::AuditLog),
        result(AuditResult::Success)
    ),
}
