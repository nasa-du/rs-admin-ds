//! The row hash is a stable, independently reproducible function of the
//! canonical row and the previous hash.

use chrono::{DateTime, Utc};
use rs_admin_audit::{AuditRow, canonical_audit_row, compute_row_hash};
use rs_admin_security::HmacKey;
use serde_json::{Map, Value};
use uuid::Uuid;

const GOLDEN_JSON: &str = concat!(
    r#"{"action":"authn_login_fail","actor_label":"anonymous","actor_user_id":null,"#,
    r#""chain_seq":1,"details":{"attempt":3,"reason_code":"bad_credentials"},"#,
    r#""hmac_key_id":"2026-11","id":"0190a5b4-7d3e-7c2a-9f10-1234567890ab","#,
    r#""ip":"203.0.113.7","occurred_at":"2026-11-07T12:00:00.123456Z","#,
    r#""prev_hash":"0000000000000000000000000000000000000000000000000000000000000000","#,
    r#""request_id":"0190a5b4-7d3f-7b1e-8c2d-abcdefabcdef","result":"failure","#,
    r#""severity":"warning","target_id":null,"target_label":null,"#,
    r#""target_type":"system","user_agent":null}"#
);

fn key() -> HmacKey {
    HmacKey::from_bytes([42u8; 32]).expect("32-byte key")
}

fn details() -> Map<String, Value> {
    serde_json::json!({"reason_code": "bad_credentials", "attempt": 3})
        .as_object()
        .cloned()
        .expect("object")
}

fn row() -> AuditRow {
    AuditRow {
        id: Uuid::parse_str("0190a5b4-7d3e-7c2a-9f10-1234567890ab").expect("uuid"),
        occurred_at: DateTime::parse_from_rfc3339("2026-11-07T12:00:00.123456Z")
            .expect("timestamp")
            .with_timezone(&Utc),
        actor_user_id: None,
        actor_label: "anonymous".into(),
        action: "authn_login_fail".into(),
        target_type: "system".into(),
        target_id: None,
        target_label: None,
        result: "failure".into(),
        severity: "warning".into(),
        ip: Some("203.0.113.7".into()),
        user_agent: None,
        request_id: Uuid::parse_str("0190a5b4-7d3f-7b1e-8c2d-abcdefabcdef").expect("uuid"),
        details: details(),
        chain_seq: 1,
        prev_hash: [0u8; 32],
        hmac_key_id: "2026-11".into(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn canonical_json_matches_the_independent_golden_value() {
    assert_eq!(canonical_audit_row(&row()).expect("canonical"), GOLDEN_JSON);
}

#[test]
fn row_hash_matches_the_independent_golden_value() {
    // Computed with Python's hmac/sha256 over prev_hash || 0x00 || JCS(row).
    assert_eq!(
        hex(&compute_row_hash(&key(), &row()).expect("hash")),
        "96df033e3baff9c09d571fea4b7b938c6c0cb43bed594ecda877470b3cb7562c"
    );
}

#[test]
fn every_hashed_field_changes_the_row_hash() {
    let baseline = compute_row_hash(&key(), &row()).expect("hash");

    let mut changed = row();
    changed.actor_label = "someone".into();
    assert_ne!(compute_row_hash(&key(), &changed).unwrap(), baseline);

    let mut changed = row();
    changed.chain_seq = 2;
    assert_ne!(compute_row_hash(&key(), &changed).unwrap(), baseline);

    let mut changed = row();
    changed.prev_hash = [1u8; 32];
    assert_ne!(compute_row_hash(&key(), &changed).unwrap(), baseline);

    let mut changed = row();
    changed.severity = "critical".into();
    assert_ne!(compute_row_hash(&key(), &changed).unwrap(), baseline);

    let mut changed = row();
    changed.ip = Some("198.51.100.9".into());
    assert_ne!(compute_row_hash(&key(), &changed).unwrap(), baseline);

    let mut changed = row();
    changed.details = serde_json::json!({"reason_code": "tampered"})
        .as_object()
        .cloned()
        .unwrap();
    assert_ne!(compute_row_hash(&key(), &changed).unwrap(), baseline);

    let mut changed = row();
    changed.hmac_key_id = "2026-10".into();
    assert_ne!(compute_row_hash(&key(), &changed).unwrap(), baseline);
}
