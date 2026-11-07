//! The audit catalog is closed: every action has a fixed severity rule and
//! target rule, and events cannot be built outside them.

use rs_admin_audit::{
    AuditAction, AuditActor, AuditEvent, AuditEventError, AuditResult, ResultRule, Severity,
    SeverityRule, TargetRule, TargetType,
};
use std::str::FromStr;
use uuid::Uuid;

fn request_id() -> Uuid {
    Uuid::now_v7()
}

fn actor() -> AuditActor {
    AuditActor::user(Uuid::now_v7(), "alice")
}

/// Transcribed independently from docs/specs/login-system.md §8.5.1/§8.5.2.
fn expected() -> Vec<(&'static str, Severity, &'static str, &'static str)> {
    use Severity::{Critical, Info, Warning};
    vec![
        ("authn_login_success", Info, "session", "success"),
        ("authn_login_fail", Warning, "user|system", "failure|denied"),
        ("authn_login_lock", Warning, "user|system", "denied"),
        ("authn_login_rate_limited", Warning, "system", "denied"),
        ("authn_logout", Info, "session", "success"),
        ("authn_logout_all", Info, "user", "success"),
        (
            "authn_refresh_reuse_detected",
            Critical,
            "session",
            "failure",
        ),
        ("authn_session_evicted", Info, "session", "success"),
        ("authn_password_changed", Info, "user", "success"),
        ("authn_password_change_fail", Warning, "user", "failure"),
        ("authn_reauth_success", Info, "user", "success"),
        ("authn_reauth_fail", Warning, "user", "failure"),
        (
            "authz_step_up_denied",
            Warning,
            "user|role|system",
            "denied",
        ),
        ("authz_denied", Warning, "user|role|system", "denied"),
        ("user_created", Info, "user", "success"),
        ("user_updated", Info, "user", "success"),
        ("user_status_changed", Warning, "user", "success"),
        ("user_password_reset", Critical, "user", "success"),
        ("user_unlocked", Warning, "user", "success"),
        ("user_roles_changed", Warning, "user", "success"),
        ("user_sessions_revoked", Warning, "user", "success"),
        ("role_created", Warning, "role", "success"),
        ("role_updated", Warning, "role", "success"),
        ("role_permissions_changed", Critical, "role", "success"),
        ("role_deleted", Warning, "role", "success"),
        ("system_bootstrap_super_admin", Critical, "user", "success"),
        ("system_recover_super_admin", Critical, "user", "success"),
        ("crypto_key_rotated", Critical, "system", "success"),
        ("system_audit_logs_purged", Warning, "system", "success"),
        ("audit_log_accessed", Info, "audit_log", "success"),
    ]
}

fn results(rule: ResultRule) -> String {
    match rule {
        ResultRule::Fixed(result) => result.as_str().to_owned(),
        ResultRule::AnyOf(allowed) => allowed
            .iter()
            .map(|result| result.as_str())
            .collect::<Vec<_>>()
            .join("|"),
    }
}

fn targets(rule: TargetRule) -> String {
    match rule {
        TargetRule::Fixed(target) => target.as_str().to_owned(),
        TargetRule::Flexible(allowed) => allowed
            .iter()
            .map(|target| target.as_str())
            .collect::<Vec<_>>()
            .join("|"),
    }
}

#[test]
fn catalog_exactly_matches_the_spec() {
    let expected_rows = expected();
    assert_eq!(AuditAction::ALL.len(), expected_rows.len());

    for (action, (name, severity, targets_expected, results_expected)) in
        AuditAction::ALL.iter().zip(expected_rows)
    {
        assert_eq!(action.as_str(), name, "action name mismatch");
        assert_eq!(
            action.default_severity(),
            severity,
            "default severity mismatch for {name}"
        );
        assert_eq!(targets(action.target_rule()), targets_expected, "{name}");
        assert_eq!(results(action.result_rule()), results_expected, "{name}");
    }
}

#[test]
fn action_names_are_unique_and_round_trip() {
    let unique: std::collections::HashSet<_> = AuditAction::ALL
        .iter()
        .map(|action| action.as_str())
        .collect();
    assert_eq!(unique.len(), AuditAction::ALL.len());

    for action in AuditAction::ALL {
        assert_eq!(AuditAction::from_str(action.as_str()).unwrap(), *action);
    }
    assert!(AuditAction::from_str("authn_login_succes").is_err());
}

#[test]
fn severity_result_and_target_dictionaries_round_trip() {
    for severity in Severity::ALL {
        assert_eq!(Severity::from_str(severity.as_str()).unwrap(), *severity);
    }
    for result in AuditResult::ALL {
        assert_eq!(AuditResult::from_str(result.as_str()).unwrap(), *result);
    }
    for target in TargetType::ALL {
        assert_eq!(TargetType::from_str(target.as_str()).unwrap(), *target);
    }
}

#[test]
fn user_roles_changed_is_the_only_elevated_severity_action() {
    for action in AuditAction::ALL {
        match action {
            AuditAction::UserRolesChanged => assert_eq!(
                action.severity_rule(),
                SeverityRule::Elevated {
                    default: Severity::Warning,
                    elevated: Severity::Critical,
                }
            ),
            _ => assert!(
                matches!(action.severity_rule(), SeverityRule::Fixed(_)),
                "{} must have a fixed severity",
                action.as_str()
            ),
        }
    }
}

#[test]
fn events_reject_targets_outside_the_action_rule() {
    let error = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        actor(),
        request_id(),
    )
    .target(TargetType::Role, Some(Uuid::now_v7()), None)
    .build()
    .expect_err("user_created cannot target a role");
    assert!(error.to_string().contains("target"));

    AuditEvent::builder(
        AuditAction::AuthzDenied,
        AuditResult::Denied,
        actor(),
        request_id(),
    )
    .target(TargetType::Role, Some(Uuid::now_v7()), Some("ops".into()))
    .build()
    .expect("authz_denied may target a role");

    AuditEvent::builder(
        AuditAction::AuthnLoginSuccess,
        AuditResult::Success,
        actor(),
        request_id(),
    )
    .target(TargetType::System, None, None)
    .build()
    .expect_err("login success targets a session");
}

#[test]
fn flexible_target_actions_require_an_explicit_target() {
    let result = AuditEvent::builder(
        AuditAction::AuthnLoginFail,
        AuditResult::Failure,
        AuditActor::anonymous(),
        request_id(),
    )
    .build();

    assert!(matches!(result, Err(AuditEventError::MissingTarget)));
}

#[test]
fn events_require_an_id_for_non_system_targets() {
    let error = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        actor(),
        request_id(),
    )
    .target(TargetType::User, None, Some("bob".into()))
    .build()
    .expect_err("user target requires an id");
    assert!(error.to_string().contains("target id"));

    AuditEvent::builder(
        AuditAction::CryptoKeyRotated,
        AuditResult::Success,
        AuditActor::system(),
        request_id(),
    )
    .build()
    .expect("system target needs no id");
}

#[test]
fn severity_overrides_follow_the_closed_rule() {
    let fixed = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        actor(),
        request_id(),
    )
    .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
    .severity(Severity::Critical);

    assert!(fixed.build().is_err(), "user_created severity is fixed");

    let elevated = AuditEvent::builder(
        AuditAction::UserRolesChanged,
        AuditResult::Success,
        actor(),
        request_id(),
    )
    .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
    .severity(Severity::Critical);

    let event = elevated.build().expect("critical role change is allowed");
    assert_eq!(event.severity(), Severity::Critical);
}

#[test]
fn result_must_match_the_action_catalog() {
    let success_action_with_failure = AuditEvent::builder(
        AuditAction::AuthnLoginSuccess,
        AuditResult::Failure,
        actor(),
        request_id(),
    )
    .target(TargetType::Session, Some(Uuid::now_v7()), None)
    .build();
    assert!(matches!(
        success_action_with_failure,
        Err(AuditEventError::InvalidResult(AuditResult::Failure))
    ));

    AuditEvent::builder(
        AuditAction::AuthnLoginFail,
        AuditResult::Denied,
        AuditActor::anonymous(),
        request_id(),
    )
    .target(TargetType::System, None, None)
    .build()
    .expect("a disabled account denies rather than fails");

    let failed_login_with_success = AuditEvent::builder(
        AuditAction::AuthnLoginFail,
        AuditResult::Success,
        AuditActor::anonymous(),
        request_id(),
    )
    .target(TargetType::System, None, None)
    .build();
    assert!(failed_login_with_success.is_err());

    let created_with_denied = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Denied,
        actor(),
        request_id(),
    )
    .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
    .build();
    assert!(created_with_denied.is_err());
}

#[test]
fn details_reject_secret_keys_at_any_depth() {
    for key in [
        "password",
        "password_hash",
        "temporary_password",
        "access_token",
        "refresh_token",
        "authorization",
        "cookie",
        "secret",
        "api_key",
        "private_key",
        "client_secret",
    ] {
        let details = serde_json::json!({ key: "must-not-be-stored" })
            .as_object()
            .cloned()
            .expect("object");
        let result = AuditEvent::builder(
            AuditAction::UserCreated,
            AuditResult::Success,
            actor(),
            request_id(),
        )
        .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
        .details(details)
        .build();
        assert!(result.is_err(), "details key {key} must be rejected");
    }

    let nested = serde_json::json!({"outer": {"password_hash": "nope"}})
        .as_object()
        .cloned()
        .expect("object");
    let result = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        actor(),
        request_id(),
    )
    .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
    .details(nested)
    .build();
    assert!(result.is_err(), "nested secret keys must be rejected");

    let harmless = serde_json::json!({
        "temporary_password_expires_at": "2026-11-10T00:00:00Z",
        "permission_keys": ["audit.read"],
        "key_type": "jwt"
    })
    .as_object()
    .cloned()
    .expect("object");
    AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        actor(),
        request_id(),
    )
    .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
    .details(harmless)
    .build()
    .expect("exact-key matching must not reject diagnostic fields");
}

#[test]
fn user_agent_is_truncated_to_512_characters() {
    let long = "é".repeat(600);
    let event = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        actor(),
        request_id(),
    )
    .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
    .user_agent(long)
    .build()
    .expect("valid event");

    assert_eq!(event.user_agent().expect("user agent").chars().count(), 512);
}

#[test]
fn events_reject_empty_actor_labels_and_nil_request_ids() {
    let empty_actor = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        AuditActor::user(Uuid::now_v7(), ""),
        request_id(),
    )
    .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
    .build();
    assert!(empty_actor.is_err());

    let nil_request = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        actor(),
        Uuid::nil(),
    )
    .target(TargetType::User, Some(Uuid::now_v7()), Some("bob".into()))
    .build();
    assert!(nil_request.is_err());
}
