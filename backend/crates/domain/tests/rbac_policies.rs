//! Pure RBAC policy behavior (docs/specs/login-system.md §6.2–§6.3).
//!
//! Expected role permission sets are transcribed from the spec, independently
//! of the implementation.

use rs_admin_domain::permission::PermissionKey;
use rs_admin_domain::rbac::{
    BASELINE_ROLE, BuiltInRole, EffectivePermissions, RoleGrant, with_baseline_role,
};
use std::collections::BTreeSet;

const ADMIN_EXPECTED: &[&str] = &[
    "users.read",
    "users.create",
    "users.update",
    "users.set_status",
    "users.reset_password",
    "users.unlock",
    "users.assign_role",
    "sessions.revoke",
    "roles.read",
    "audit.read",
];

fn keys(keys: &[&str]) -> BTreeSet<PermissionKey> {
    keys.iter()
        .map(|key| key.parse().expect("catalog key"))
        .collect()
}

fn codes(role: BuiltInRole) -> Vec<&'static str> {
    role.permission_keys()
        .iter()
        .map(|key| key.as_str())
        .collect()
}

#[test]
fn builtin_role_codes_are_stable() {
    let codes: Vec<_> = BuiltInRole::ALL.iter().map(|role| role.code()).collect();
    assert_eq!(codes, ["user", "admin", "super_admin"]);

    for role in BuiltInRole::ALL {
        assert_eq!(BuiltInRole::from_code(role.code()), Some(*role));
    }
    assert_eq!(BuiltInRole::from_code("root"), None);
    assert_eq!(BuiltInRole::from_code("Admin"), None);
}

#[test]
fn builtin_role_permission_sets_are_fixed() {
    assert!(BuiltInRole::User.permission_keys().is_empty());
    assert_eq!(codes(BuiltInRole::Admin), ADMIN_EXPECTED);
    assert_eq!(
        BuiltInRole::SuperAdmin.permission_keys(),
        PermissionKey::ALL
    );
}

#[test]
fn baseline_role_is_user_and_is_never_removed() {
    assert_eq!(BASELINE_ROLE, BuiltInRole::User);
    assert!(BASELINE_ROLE.is_baseline());
    assert!(!BuiltInRole::Admin.is_baseline());
    assert!(!BuiltInRole::SuperAdmin.is_baseline());

    assert_eq!(with_baseline_role([]), BTreeSet::from([BuiltInRole::User]));
    assert_eq!(
        with_baseline_role([BuiltInRole::Admin, BuiltInRole::Admin]),
        BTreeSet::from([BuiltInRole::User, BuiltInRole::Admin])
    );
    assert_eq!(
        with_baseline_role([BuiltInRole::User, BuiltInRole::SuperAdmin]),
        BTreeSet::from([BuiltInRole::User, BuiltInRole::SuperAdmin])
    );
}

#[test]
fn baseline_role_grants_no_permissions() {
    let effective = EffectivePermissions::from_grants([RoleGrant::BuiltIn(BASELINE_ROLE)]);

    assert!(!effective.is_super_admin());
    assert!(effective.permission_keys().is_empty());
}

#[test]
fn effective_permissions_are_the_union_of_all_roles() {
    let effective = EffectivePermissions::from_grants([
        RoleGrant::BuiltIn(BuiltInRole::Admin),
        RoleGrant::Custom(&[PermissionKey::RolesCreate, PermissionKey::UsersRead]),
        RoleGrant::Custom(&[PermissionKey::UsersAssignRole]),
    ]);

    let expected = keys(&[
        "users.read",
        "users.create",
        "users.update",
        "users.set_status",
        "users.reset_password",
        "users.unlock",
        "users.assign_role",
        "sessions.revoke",
        "roles.read",
        "roles.create",
        "audit.read",
    ]);
    assert_eq!(effective.permission_keys(), &expected);
    assert!(!effective.is_super_admin());
    assert!(effective.contains(PermissionKey::RolesCreate));
    assert!(effective.contains(PermissionKey::UsersRead));
}

#[test]
fn permissions_not_granted_by_any_role_are_denied() {
    let effective =
        EffectivePermissions::from_grants([RoleGrant::Custom(&[PermissionKey::UsersRead])]);

    assert!(effective.contains(PermissionKey::UsersRead));
    for denied in [
        PermissionKey::UsersCreate,
        PermissionKey::RolesRead,
        PermissionKey::RolesCreate,
        PermissionKey::AuditRead,
    ] {
        assert!(!effective.contains(denied), "{denied} must be denied");
    }
}

#[test]
fn super_admin_bypasses_checks_and_owns_the_whole_catalog() {
    let effective =
        EffectivePermissions::from_grants([RoleGrant::BuiltIn(BuiltInRole::SuperAdmin)]);

    assert!(effective.is_super_admin());
    assert_eq!(effective.permission_keys().len(), PermissionKey::ALL.len());
    for permission in PermissionKey::ALL {
        assert!(
            effective.contains(*permission),
            "{permission} must be allowed"
        );
    }
}

#[test]
fn grant_subset_rule_applies_to_non_super_admins() {
    let admin = EffectivePermissions::from_grants([RoleGrant::BuiltIn(BuiltInRole::Admin)]);
    assert!(admin.can_grant(&keys(&["users.read", "audit.read"])));
    assert!(!admin.can_grant(&keys(&["roles.create"])));
    assert!(!admin.can_grant(&keys(&["roles.delete"])));

    let narrow =
        EffectivePermissions::from_grants([RoleGrant::Custom(&[PermissionKey::UsersRead])]);
    assert!(narrow.can_grant(&keys(&["users.read"])));
    assert!(!narrow.can_grant(&keys(&["users.read", "users.create"])));

    let super_admin =
        EffectivePermissions::from_grants([RoleGrant::BuiltIn(BuiltInRole::SuperAdmin)]);
    assert!(super_admin.can_grant(&keys(&["roles.create", "roles.delete"])));
}
