//! Transactional audit-chain behaviour against a real PostgreSQL 18 database.

use rs_admin_audit::{
    AuditAction, AuditActor, AuditEvent, AuditResult, AuditWriter, ChainBreakReason, TargetType,
    VerifyError, verify_chain,
};
use rs_admin_security::AuditKeyRing;
use sqlx::PgPool;
use uuid::Uuid;

const KEY_B64: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
const KEY_ID: &str = "2026-11";

fn keyring() -> AuditKeyRing {
    AuditKeyRing::from_json(&format!("{{\"{KEY_ID}\":\"{KEY_B64}\"}}"), KEY_ID).expect("keyring")
}

fn writer() -> AuditWriter {
    AuditWriter::new(keyring())
}

fn login_fail() -> AuditEvent {
    AuditEvent::builder(
        AuditAction::AuthnLoginFail,
        AuditResult::Failure,
        AuditActor::anonymous(),
        Uuid::now_v7(),
    )
    .target(TargetType::System, None, None)
    .detail("reason_code", "bad_credentials")
    .ip("203.0.113.7".parse().expect("ip"))
    .build()
    .expect("event")
}

fn key_rotated() -> AuditEvent {
    AuditEvent::builder(
        AuditAction::CryptoKeyRotated,
        AuditResult::Success,
        AuditActor::system(),
        Uuid::now_v7(),
    )
    .detail("key_type", "jwt")
    .detail("old_kid", "2026-10")
    .detail("new_kid", "2026-11")
    .build()
    .expect("event")
}

fn with_ip(ip: &str) -> AuditEvent {
    AuditEvent::builder(
        AuditAction::CryptoKeyRotated,
        AuditResult::Success,
        AuditActor::system(),
        Uuid::now_v7(),
    )
    .ip(ip.parse().expect("ip"))
    .build()
    .expect("event")
}

fn numeric_details() -> AuditEvent {
    AuditEvent::builder(
        AuditAction::SystemAuditLogsPurged,
        AuditResult::Success,
        AuditActor::system(),
        Uuid::now_v7(),
    )
    .detail("from_seq", 1)
    .detail("to_seq", 2)
    .detail("count", 2)
    .detail("retention_days", 180)
    .detail("ratio", 0.25)
    .detail("big", 1e30)
    .detail(
        "nested",
        serde_json::json!({"actions": ["a", "b"], "ok": true}),
    )
    .build()
    .expect("event")
}

fn user_created(user_id: Uuid) -> AuditEvent {
    AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        AuditActor::user(Uuid::now_v7(), "admin"),
        Uuid::now_v7(),
    )
    .target(TargetType::User, Some(user_id), Some("alice".into()))
    .detail("username", "alice")
    .build()
    .expect("event")
}

#[sqlx::test(migrations = "../../migrations")]
async fn writes_append_linked_rows_and_verify(pool: PgPool) {
    let writer = writer();

    let first = writer.write(&pool, &login_fail()).await.expect("first");
    let second = writer.write(&pool, &key_rotated()).await.expect("second");
    let third = writer
        .write(&pool, &user_created(Uuid::now_v7()))
        .await
        .expect("third");

    assert_eq!(
        (first.chain_seq, second.chain_seq, third.chain_seq),
        (1, 2, 3)
    );

    let rows: Vec<(i64, Vec<u8>, Vec<u8>, String)> = sqlx::query_as(
        "SELECT chain_seq, prev_hash, row_hash, hmac_key_id
         FROM audit_logs ORDER BY chain_seq",
    )
    .fetch_all(&pool)
    .await
    .expect("rows");

    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0].1,
        vec![0u8; 32],
        "first row starts at the zero hash"
    );
    assert_eq!(rows[1].1, rows[0].2, "second row links to the first");
    assert_eq!(rows[2].1, rows[1].2, "third row links to the second");
    assert_eq!(rows[0].3, KEY_ID);
    assert_eq!(rows[2].2, third.row_hash.to_vec());

    let head: (i64, Vec<u8>) =
        sqlx::query_as("SELECT last_seq, last_hash FROM audit_chain_head WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("head");
    assert_eq!(head.0, 3);
    assert_eq!(head.1, third.row_hash.to_vec());

    let verification = verify_chain(&pool, &keyring()).await.expect("valid chain");
    assert_eq!(verification.verified_rows, 3);
    assert_eq!(verification.head_seq, 3);
}

#[sqlx::test(migrations = "../../migrations")]
async fn ip_addresses_round_trip_canonically(pool: PgPool) {
    let writer = writer();
    writer
        .write(&pool, &with_ip("2001:db8::1"))
        .await
        .expect("ipv6 write");
    writer
        .write(&pool, &with_ip("::ffff:203.0.113.7"))
        .await
        .expect("mapped ipv4 write");

    verify_chain(&pool, &keyring())
        .await
        .expect("stored inet text must match the hashed representation");

    let ips: Vec<Option<String>> =
        sqlx::query_scalar("SELECT host(ip) FROM audit_logs ORDER BY chain_seq")
            .fetch_all(&pool)
            .await
            .expect("ips");
    assert_eq!(
        ips,
        vec![
            Some("2001:db8::1".to_owned()),
            Some("203.0.113.7".to_owned())
        ]
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn numeric_and_nested_details_survive_the_jsonb_round_trip(pool: PgPool) {
    let writer = writer();
    writer
        .write(&pool, &numeric_details())
        .await
        .expect("write");

    verify_chain(&pool, &keyring())
        .await
        .expect("canonicalization after jsonb must match the writer");
}

#[sqlx::test(migrations = "../../migrations")]
async fn tampering_with_details_is_detected(pool: PgPool) {
    let writer = writer();
    writer.write(&pool, &login_fail()).await.expect("first");
    writer.write(&pool, &key_rotated()).await.expect("second");
    writer
        .write(&pool, &user_created(Uuid::now_v7()))
        .await
        .expect("third");

    sqlx::query("UPDATE audit_logs SET details = '{\"reason_code\":\"tampered\"}'::jsonb WHERE chain_seq = 2")
        .execute(&pool)
        .await
        .expect("tamper");

    let error = verify_chain(&pool, &keyring())
        .await
        .expect_err("tampered row must fail verification");
    assert!(matches!(
        error,
        VerifyError::Broken(chain_break)
            if chain_break.chain_seq == 2
                && chain_break.reason == ChainBreakReason::RowHashMismatch
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_row_is_detected(pool: PgPool) {
    let writer = writer();
    writer.write(&pool, &login_fail()).await.expect("first");
    writer.write(&pool, &key_rotated()).await.expect("second");
    writer
        .write(&pool, &user_created(Uuid::now_v7()))
        .await
        .expect("third");

    sqlx::query("DELETE FROM audit_logs WHERE chain_seq = 2")
        .execute(&pool)
        .await
        .expect("delete");

    let error = verify_chain(&pool, &keyring())
        .await
        .expect_err("deleted row must fail verification");
    assert!(matches!(
        error,
        VerifyError::Broken(chain_break)
            if chain_break.chain_seq == 3
                && matches!(
                    chain_break.reason,
                    ChainBreakReason::SequenceGap { expected: 2, found: 3 }
                )
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_the_first_row_is_detected(pool: PgPool) {
    let writer = writer();
    writer.write(&pool, &login_fail()).await.expect("first");
    writer.write(&pool, &key_rotated()).await.expect("second");

    sqlx::query("DELETE FROM audit_logs WHERE chain_seq = 1")
        .execute(&pool)
        .await
        .expect("delete prefix");

    let error = verify_chain(&pool, &keyring())
        .await
        .expect_err("a truncated prefix must fail verification");
    assert!(matches!(
        error,
        VerifyError::Broken(chain_break)
            if chain_break.chain_seq == 2
                && chain_break.reason == ChainBreakReason::PrefixMissing { found: 2 }
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn tampering_with_the_anchor_is_detected(pool: PgPool) {
    let writer = writer();
    writer.write(&pool, &login_fail()).await.expect("first");

    let mut rows = rs_admin_store::audit::load_rows(&pool)
        .await
        .expect("load rows");
    let mut row = rows.remove(0).row;
    row.prev_hash = [1u8; 32];
    let key = rs_admin_security::HmacKey::from_base64(KEY_B64).expect("key");
    let hash = rs_admin_audit::compute_row_hash(&key, &row).expect("hash");

    sqlx::query("UPDATE audit_logs SET prev_hash = $1, row_hash = $2 WHERE chain_seq = 1")
        .bind(vec![1u8; 32])
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("tamper anchor");
    sqlx::query("UPDATE audit_chain_head SET last_hash = $1 WHERE id = 1")
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("tamper head");

    let error = verify_chain(&pool, &keyring())
        .await
        .expect_err("a non-zero anchor must fail verification");
    assert!(matches!(
        error,
        VerifyError::Broken(chain_break)
            if chain_break.chain_seq == 1
                && chain_break.reason == ChainBreakReason::AnchorMismatch
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn tampering_with_the_chain_head_is_detected(pool: PgPool) {
    let writer = writer();
    writer.write(&pool, &login_fail()).await.expect("first");
    writer.write(&pool, &key_rotated()).await.expect("second");

    sqlx::query("UPDATE audit_chain_head SET last_hash = $1 WHERE id = 1")
        .bind(vec![9u8; 32])
        .execute(&pool)
        .await
        .expect("tamper head");

    let error = verify_chain(&pool, &keyring())
        .await
        .expect_err("tampered head must fail verification");
    assert!(matches!(
        error,
        VerifyError::Broken(chain_break)
            if chain_break.reason == ChainBreakReason::HeadHashMismatch
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn audit_failure_rolls_back_the_business_transaction(pool: PgPool) {
    sqlx::query("CREATE TABLE business_probe (id integer PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("probe table");

    // Removing the chain head makes the audit write fail after the business
    // statement has run, so the caller must roll everything back.
    sqlx::query("DELETE FROM audit_chain_head")
        .execute(&pool)
        .await
        .expect("break head");

    let mut transaction = pool.begin().await.expect("begin");
    sqlx::query("INSERT INTO business_probe (id) VALUES (1)")
        .execute(&mut *transaction)
        .await
        .expect("business insert");

    let result = writer().write_in_tx(&mut transaction, &login_fail()).await;
    assert!(result.is_err(), "audit write must fail");

    transaction.rollback().await.expect("rollback");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM business_probe")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 0, "business change must roll back with the audit");
}

#[sqlx::test(migrations = "../../migrations")]
async fn failed_audit_write_is_not_committed_by_the_convenience_writer(pool: PgPool) {
    sqlx::query("DELETE FROM audit_chain_head")
        .execute(&pool)
        .await
        .expect("break head");

    writer()
        .write(&pool, &login_fail())
        .await
        .expect_err("audit write must fail");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}
