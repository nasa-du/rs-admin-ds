//! The code-owned permission catalog (docs/specs/login-system.md §6.1).
//!
//! The database `permissions` table is only a mirror of this catalog. Nothing
//! outside this module may add, remove or rename a permission: new permissions
//! are additive code changes that are synced to the database at startup.

use std::fmt;
use std::str::FromStr;

/// Functional area a permission belongs to. Groups are emitted in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionGroup {
    Users,
    Sessions,
    Roles,
    Audit,
}

impl PermissionGroup {
    /// Every group, in catalog order.
    pub const ALL: &'static [Self] = &[Self::Users, Self::Sessions, Self::Roles, Self::Audit];

    /// Stable wire identifier stored in `permissions.group_key`.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Users => "users",
            Self::Sessions => "sessions",
            Self::Roles => "roles",
            Self::Audit => "audit",
        }
    }

    pub const fn display_name_zh(self) -> &'static str {
        match self {
            Self::Users => "用户",
            Self::Sessions => "会话",
            Self::Roles => "角色",
            Self::Audit => "审计",
        }
    }

    pub const fn display_name_en(self) -> &'static str {
        match self {
            Self::Users => "Users",
            Self::Sessions => "Sessions",
            Self::Roles => "Roles",
            Self::Audit => "Audit",
        }
    }
}

/// The closed set of permissions. Keys are `resource.action` with a plural
/// resource and a snake_case action.
macro_rules! permission_catalog {
    ($( $variant:ident => ($key:literal, $group:ident, $zh:literal, $en:literal) ),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum PermissionKey {
            $( $variant ),+
        }

        impl PermissionKey {
            /// Every permission, in catalog order.
            pub const ALL: &'static [Self] = &[ $( Self::$variant ),+ ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $key ),+
                }
            }

            pub const fn group(self) -> PermissionGroup {
                match self {
                    $( Self::$variant => PermissionGroup::$group ),+
                }
            }

            pub const fn display_name_zh(self) -> &'static str {
                match self {
                    $( Self::$variant => $zh ),+
                }
            }

            pub const fn display_name_en(self) -> &'static str {
                match self {
                    $( Self::$variant => $en ),+
                }
            }
        }

        /// Catalog metadata in the order clients must render it.
        pub const PERMISSION_CATALOG: &[PermissionDefinition] = &[
            $(
                PermissionDefinition {
                    key: PermissionKey::$variant,
                    group: PermissionGroup::$group,
                    display_name_zh: $zh,
                    display_name_en: $en,
                }
            ),+
        ];

        impl FromStr for PermissionKey {
            type Err = UnknownPermission;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $( $key => Ok(Self::$variant), )+
                    _ => Err(UnknownPermission(value.to_owned())),
                }
            }
        }

        impl fmt::Display for PermissionKey {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

permission_catalog! {
    UsersRead => ("users.read", Users, "查看用户", "View users"),
    UsersCreate => ("users.create", Users, "创建用户", "Create users"),
    UsersUpdate => ("users.update", Users, "编辑用户", "Edit users"),
    UsersSetStatus => ("users.set_status", Users, "启用/停用用户", "Enable/disable users"),
    UsersResetPassword => (
        "users.reset_password",
        Users,
        "重置用户密码",
        "Reset user passwords"
    ),
    UsersUnlock => ("users.unlock", Users, "解锁用户", "Unlock users"),
    UsersAssignRole => ("users.assign_role", Users, "分配用户角色", "Assign user roles"),
    SessionsRevoke => ("sessions.revoke", Sessions, "撤销用户会话", "Revoke user sessions"),
    RolesRead => ("roles.read", Roles, "查看角色", "View roles"),
    RolesCreate => ("roles.create", Roles, "创建角色", "Create roles"),
    RolesUpdate => ("roles.update", Roles, "编辑角色", "Edit roles"),
    RolesDelete => ("roles.delete", Roles, "删除角色", "Delete roles"),
    AuditRead => ("audit.read", Audit, "查看审计日志", "View audit logs"),
}

/// One entry of the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PermissionDefinition {
    pub key: PermissionKey,
    pub group: PermissionGroup,
    pub display_name_zh: &'static str,
    pub display_name_en: &'static str,
}

/// Returned when a stored permission key is not part of the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPermission(pub String);

impl fmt::Display for UnknownPermission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown permission: {}", self.0)
    }
}

impl std::error::Error for UnknownPermission {}
