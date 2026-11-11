//! `record-key-rotation` against a real PostgreSQL schema (ADR-0005 §8.4,
//! ADR-0008 §3.5).

mod support;

use rs_admin_cli::{KeyRotationInput, KeyType, RotationReason, record_key_rotation};
use sqlx::PgPool;
use support::{audit_keyring, audit_rows, prepare_catalog, verify_audit_chain};

#[sqlx::test(migrations = "../../migrations")]
async fn records_a_critical_rotation_event_on_the_chain(pool: PgPool) {
    prepare_catalog(&pool).await;

    let outcome = record_key_rotation(
        &pool,
        &audit_keyring(),
        KeyRotationInput {
            key_type: KeyType::Jwt,
            old_kid: "prod-2026-11",
            new_kid: "prod-2027-01",
            reason: RotationReason::Scheduled,
        },
    )
    .await
    .expect("record rotation");

    let rows = audit_rows(&pool).await;
    assert_eq!(rows.len(), 1, "one rotation event is written");
    let row = &rows[0];
    assert_eq!(row.actor_label, "cli");
    assert_eq!(row.action, "crypto_key_rotated");
    assert_eq!(row.target_type, "system");
    assert_eq!(row.result, "success");
    assert_eq!(row.severity, "critical");
    assert_eq!(row.details["key_type"], "jwt");
    assert_eq!(row.details["old_kid"], "prod-2026-11");
    assert_eq!(row.details["new_kid"], "prod-2027-01");
    assert_eq!(row.details["reason"], "scheduled");
    assert_eq!(outcome.chain_seq, 1);

    verify_audit_chain(&pool).await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn records_the_audit_chain_key_class_and_compromise_reason(pool: PgPool) {
    prepare_catalog(&pool).await;

    record_key_rotation(
        &pool,
        &audit_keyring(),
        KeyRotationInput {
            key_type: KeyType::AuditChain,
            old_kid: "audit-2026-11",
            new_kid: "audit-2027-01",
            reason: RotationReason::Compromise,
        },
    )
    .await
    .expect("record rotation");

    let rows = audit_rows(&pool).await;
    assert_eq!(rows[0].details["key_type"], "audit_chain");
    assert_eq!(rows[0].details["reason"], "compromise");
    verify_audit_chain(&pool).await;
}
