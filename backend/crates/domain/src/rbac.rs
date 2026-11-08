//! Pure RBAC policy (docs/specs/login-system.md §6).
//!
//! Roles and permissions are resolved from PostgreSQL on every request; these
//! types receive the already-loaded grants and decide authorization without
//! touching a database.

use crate::permission::PermissionKey;
use std::collections::BTreeSet;

/// The baseline role every user holds and that cannot be removed.
pub const BASELINE_ROLE: BuiltInRole = BuiltInRole::User;

/// The three code-defined roles with fixed permission sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuiltInRole {
    User,
    Admin,
    SuperAdmin,
}

const ADMIN_PERMISSIONS: &[PermissionKey] = &[
    PermissionKey::UsersRead,
    PermissionKey::UsersCreate,
    PermissionKey::UsersUpdate,
    PermissionKey::UsersSetStatus,
    PermissionKey::UsersResetPassword,
    PermissionKey::UsersUnlock,
    PermissionKey::UsersAssignRole,
    PermissionKey::SessionsRevoke,
    PermissionKey::RolesRead,
    PermissionKey::AuditRead,
];

impl BuiltInRole {
    /// Every built-in role, in sync order.
    pub const ALL: &'static [Self] = &[Self::User, Self::Admin, Self::SuperAdmin];

    /// Stable wire identifier; built-in role names are reserved.
    pub const fn code(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
            Self::SuperAdmin => "super_admin",
        }
    }

    /// Parse a stable role code. Custom role names never match.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "user" => Some(Self::User),
            "admin" => Some(Self::Admin),
            "super_admin" => Some(Self::SuperAdmin),
            _ => None,
        }
    }

    pub const fn is_baseline(self) -> bool {
        matches!(self, Self::User)
    }

    /// Fixed permission set. `super_admin` additionally bypasses checks in
    /// [`EffectivePermissions`], so future catalog entries need no migration.
    pub const fn permission_keys(self) -> &'static [PermissionKey] {
        match self {
            Self::User => &[],
            Self::Admin => ADMIN_PERMISSIONS,
            Self::SuperAdmin => PermissionKey::ALL,
        }
    }
}

/// One role held by a principal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleGrant<'a> {
    /// A code-defined role with its fixed permission set.
    BuiltIn(BuiltInRole),
    /// A custom role with the permission keys loaded from the database.
    Custom(&'a [PermissionKey]),
}

/// A role considered as the target of a grant or management action
/// (spec §6.4). Unlike [`RoleGrant`] this describes the target, not the
/// operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleTarget<'a> {
    /// A code-defined role.
    BuiltIn(BuiltInRole),
    /// A custom role and its permission keys.
    Custom(&'a [PermissionKey]),
    /// A persisted role or permission key the current catalog cannot resolve.
    /// It grants nothing, but scope decisions fail closed.
    Unresolved,
}

/// The union of a principal's role permissions, plus the super-admin bypass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectivePermissions {
    is_super_admin: bool,
    permission_keys: BTreeSet<PermissionKey>,
}

impl EffectivePermissions {
    /// Resolve a principal's effective permissions. The union is order- and
    /// duplicate-insensitive; v1 has no deny rules or role priority.
    pub fn from_grants<'a>(grants: impl IntoIterator<Item = RoleGrant<'a>>) -> Self {
        let mut is_super_admin = false;
        let mut permission_keys = BTreeSet::new();

        for grant in grants {
            match grant {
                RoleGrant::BuiltIn(BuiltInRole::SuperAdmin) => {
                    is_super_admin = true;
                    permission_keys.extend(PermissionKey::ALL.iter().copied());
                }
                RoleGrant::BuiltIn(role) => {
                    permission_keys.extend(role.permission_keys().iter().copied());
                }
                RoleGrant::Custom(keys) => permission_keys.extend(keys.iter().copied()),
            }
        }

        Self {
            is_super_admin,
            permission_keys,
        }
    }

    pub fn is_super_admin(&self) -> bool {
        self.is_super_admin
    }

    /// The union of granted permissions. For a super administrator this is the
    /// whole catalog, including permissions added after the role was assigned.
    pub fn permission_keys(&self) -> &BTreeSet<PermissionKey> {
        &self.permission_keys
    }

    /// Authorization check. Super administrators are always allowed.
    pub fn contains(&self, permission: PermissionKey) -> bool {
        self.is_super_admin || self.permission_keys.contains(&permission)
    }

    /// Grant-subset rule: non-super-admins may only grant permissions they
    /// hold themselves. Super administrators are exempt.
    pub fn can_grant(&self, target: &BTreeSet<PermissionKey>) -> bool {
        self.can_grant_keys(target.iter().copied())
    }

    /// Grant-subset rule for an unordered key list, such as a role loaded from
    /// the database.
    pub fn can_grant_keys(&self, target: impl IntoIterator<Item = PermissionKey>) -> bool {
        self.is_super_admin
            || target
                .into_iter()
                .all(|key| self.permission_keys.contains(&key))
    }

    /// `assignable` target eligibility (spec §6.4): whether the operator may
    /// hand this role to a user. This flag deliberately ignores whether the
    /// operator holds `users.assign_role`; clients intersect the two.
    pub fn can_assign(&self, target: RoleTarget<'_>) -> bool {
        match target {
            RoleTarget::BuiltIn(BuiltInRole::User) => true,
            RoleTarget::BuiltIn(_) => self.is_super_admin,
            RoleTarget::Custom(keys) => self.can_grant_keys(keys.iter().copied()),
            RoleTarget::Unresolved => false,
        }
    }

    /// `manageable` target eligibility (spec §6.4): built-in roles are never
    /// manageable; custom roles are manageable when their permission set is a
    /// subset of the operator's. Holding `roles.update` is checked separately.
    pub fn can_manage(&self, target: RoleTarget<'_>) -> bool {
        match target {
            RoleTarget::BuiltIn(_) => false,
            RoleTarget::Custom(keys) => self.can_grant_keys(keys.iter().copied()),
            RoleTarget::Unresolved => false,
        }
    }

    /// Management scope over another user (spec §6.3 rule 4).
    ///
    /// A non-super-admin may read and manage only users whose permissions do
    /// not exceed their own and who hold neither the built-in `admin` nor
    /// `super_admin` role. Unresolvable catalog entries fail closed. The
    /// caller applies the "self is always visible" exception; the operator
    /// themselves is subject to this rule like any other target.
    pub fn can_manage_user<'a>(&self, roles: impl IntoIterator<Item = RoleTarget<'a>>) -> bool {
        if self.is_super_admin {
            return true;
        }
        roles.into_iter().all(|role| match role {
            RoleTarget::BuiltIn(BuiltInRole::Admin | BuiltInRole::SuperAdmin) => false,
            RoleTarget::BuiltIn(builtin) => {
                self.can_grant_keys(builtin.permission_keys().iter().copied())
            }
            RoleTarget::Custom(keys) => self.can_grant_keys(keys.iter().copied()),
            RoleTarget::Unresolved => false,
        })
    }
}

/// Normalize an assignment to built-in roles: the baseline `user` role is
/// always present and duplicates collapse.
pub fn with_baseline_role(
    requested: impl IntoIterator<Item = BuiltInRole>,
) -> BTreeSet<BuiltInRole> {
    let mut roles: BTreeSet<_> = requested.into_iter().collect();
    roles.insert(BASELINE_ROLE);
    roles
}
