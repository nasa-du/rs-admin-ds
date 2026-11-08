//! User status and super-administrator invariants
//! (docs/specs/login-system.md §5.9, §6.3 rule 7).
//!
//! These are pure policy decisions so the HTTP layer only orchestrates:
//! parameter parsing, transaction boundaries and audit.

/// Whether an account is allowed to sign in (spec §4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Disabled,
}

impl UserStatus {
    /// Stable wire and storage identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    /// Parse a wire value or a stored status. Surrounding whitespace is
    /// ignored for request bodies; the database always stores the canonical
    /// form.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

/// Disabling this target would leave the system without an enabled super
/// administrator.
pub const fn disables_last_super_admin(
    target_is_enabled_super_admin: bool,
    enabled_super_admin_count: usize,
) -> bool {
    target_is_enabled_super_admin && enabled_super_admin_count <= 1
}

/// A role replacement that drops `super_admin` from this target would leave
/// the system without an enabled super administrator.
pub const fn demotes_last_super_admin(
    target_is_enabled_super_admin: bool,
    keeps_super_admin: bool,
    enabled_super_admin_count: usize,
) -> bool {
    !keeps_super_admin
        && disables_last_super_admin(target_is_enabled_super_admin, enabled_super_admin_count)
}
