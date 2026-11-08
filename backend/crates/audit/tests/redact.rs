//! Read-time defense in depth for audit `details`: rows written before a key
//! was forbidden (or by a future bug) must not leak secret values through the
//! query API (docs/specs/login-system.md §8.5.2, §9.10).

use rs_admin_audit::redact_forbidden_details;
use serde_json::json;

#[test]
fn masks_forbidden_keys_at_every_depth_and_keeps_diagnostic_keys() {
    let details = json!({
        "password": "hunter2",
        "Password_Hash": "$argon2id$...",
        "nested": {
            "access_token": "eyJ...",
            "permission_keys": ["users.read"],
        },
        "events": [
            { "client_secret": "shh" },
            { "key_type": "audit_chain", "old_kid": "k-1" },
        ],
        "key_type": "jwt",
        "permission_keys": ["audit.read"],
    });

    let redacted = redact_forbidden_details(&details);

    assert_eq!(redacted["password"], "***");
    assert_eq!(redacted["Password_Hash"], "***");
    assert_eq!(
        redacted["nested"]["access_token"], "***",
        "nested forbidden keys must be masked"
    );
    assert_eq!(
        redacted["nested"]["permission_keys"],
        json!(["users.read"]),
        "the blacklist is exact-key, never a substring match"
    );
    assert_eq!(redacted["events"][0]["client_secret"], "***");
    assert_eq!(redacted["events"][1]["key_type"], "audit_chain");
    assert_eq!(redacted["events"][1]["old_kid"], "k-1");
    assert_eq!(redacted["key_type"], "jwt");
    assert_eq!(redacted["permission_keys"], json!(["audit.read"]));
}

#[test]
fn leaves_arrays_and_scalars_untouched() {
    assert_eq!(redact_forbidden_details(&json!("plain")), json!("plain"));
    assert_eq!(
        redact_forbidden_details(&json!([{"secret": "x"}, {"value": 1}])),
        json!([{"secret": "***"}, {"value": 1}])
    );
}
