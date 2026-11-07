//! The permission catalog is the code-owned source of truth. Its expectations
//! below are transcribed from docs/specs/login-system.md §6.1, independently
//! of the implementation.

use rs_admin_domain::permission::{PERMISSION_CATALOG, PermissionGroup, PermissionKey};
use std::collections::HashSet;
use std::str::FromStr;

/// (key, group, Chinese display name, English display name), in spec order.
const EXPECTED: &[(&str, &str, &str, &str)] = &[
    ("users.read", "users", "查看用户", "View users"),
    ("users.create", "users", "创建用户", "Create users"),
    ("users.update", "users", "编辑用户", "Edit users"),
    (
        "users.set_status",
        "users",
        "启用/停用用户",
        "Enable/disable users",
    ),
    (
        "users.reset_password",
        "users",
        "重置用户密码",
        "Reset user passwords",
    ),
    ("users.unlock", "users", "解锁用户", "Unlock users"),
    (
        "users.assign_role",
        "users",
        "分配用户角色",
        "Assign user roles",
    ),
    (
        "sessions.revoke",
        "sessions",
        "撤销用户会话",
        "Revoke user sessions",
    ),
    ("roles.read", "roles", "查看角色", "View roles"),
    ("roles.create", "roles", "创建角色", "Create roles"),
    ("roles.update", "roles", "编辑角色", "Edit roles"),
    ("roles.delete", "roles", "删除角色", "Delete roles"),
    ("audit.read", "audit", "查看审计日志", "View audit logs"),
];

#[test]
fn catalog_matches_the_spec_exactly_and_in_order() {
    assert_eq!(PERMISSION_CATALOG.len(), EXPECTED.len());

    for (definition, (key, group, zh, en)) in PERMISSION_CATALOG.iter().zip(EXPECTED) {
        assert_eq!(definition.key.as_str(), *key, "key mismatch");
        assert_eq!(definition.group.key(), *group, "group mismatch for {key}");
        assert_eq!(definition.display_name_zh, *zh, "zh mismatch for {key}");
        assert_eq!(definition.display_name_en, *en, "en mismatch for {key}");
    }
}

#[test]
fn catalog_has_thirteen_unique_keys_and_round_trips() {
    assert_eq!(PERMISSION_CATALOG.len(), 13);
    assert_eq!(PermissionKey::ALL.len(), 13);

    let unique: HashSet<_> = PERMISSION_CATALOG.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(unique.len(), 13);

    for definition in PERMISSION_CATALOG {
        assert_eq!(
            PermissionKey::from_str(definition.key.as_str()),
            Ok(definition.key)
        );
    }
    assert!(PermissionKey::from_str("users.delete").is_err());
    assert!(PermissionKey::from_str("").is_err());
}

#[test]
fn groups_are_ordered_users_sessions_roles_audit() {
    let groups: Vec<_> = PERMISSION_CATALOG.iter().map(|d| d.group.key()).collect();
    let expected = [
        "users", "users", "users", "users", "users", "users", "users", "sessions", "roles",
        "roles", "roles", "roles", "audit",
    ];
    assert_eq!(groups, expected);

    let group_order: Vec<_> = PermissionGroup::ALL.iter().map(|g| g.key()).collect();
    assert_eq!(group_order, ["users", "sessions", "roles", "audit"]);
}

#[test]
fn groups_carry_both_display_names() {
    for group in PermissionGroup::ALL {
        assert!(!group.display_name_zh().is_empty());
        assert!(!group.display_name_en().is_empty());
        assert_ne!(group.display_name_zh(), group.display_name_en());
    }
}
