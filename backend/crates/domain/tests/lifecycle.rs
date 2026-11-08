//! Pure user-status and last-super-admin policy
//! (docs/specs/login-system.md §5.9, §6.3 rule 7).

use rs_admin_domain::lifecycle::{UserStatus, demotes_last_super_admin, disables_last_super_admin};

#[test]
fn status_round_trips_its_storage_identifier() {
    for status in [UserStatus::Active, UserStatus::Disabled] {
        assert_eq!(UserStatus::parse(status.as_str()), Some(status));
    }
    assert_eq!(UserStatus::parse(" active "), Some(UserStatus::Active));
    assert_eq!(UserStatus::parse("pending"), None);
}

#[test]
fn disabling_is_refused_only_for_the_last_enabled_super_admin() {
    assert!(disables_last_super_admin(true, 1));
    assert!(disables_last_super_admin(true, 0));
    assert!(!disables_last_super_admin(true, 2));
    assert!(!disables_last_super_admin(false, 1));
    assert!(!disables_last_super_admin(false, 0));
}

#[test]
fn demotion_is_refused_only_when_it_removes_the_last_enabled_super_admin() {
    // The target is the last enabled super administrator and loses the role.
    assert!(demotes_last_super_admin(true, false, 1));
    // Keeping the role, or another enabled super administrator remaining, is
    // allowed.
    assert!(!demotes_last_super_admin(true, true, 1));
    assert!(!demotes_last_super_admin(true, false, 2));
    // A target that is not an enabled super administrator is never the last.
    assert!(!demotes_last_super_admin(false, false, 1));
}
