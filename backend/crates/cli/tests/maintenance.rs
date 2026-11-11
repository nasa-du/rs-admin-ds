//! Maintenance cleanup against a real PostgreSQL schema (AC-OPS-07).

mod support;

use chrono::{Duration, Utc};
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, verify_chain};
use rs_admin_cli::{CliError, MaintenanceStatus, run_maintenance_at};
use rs_admin_store::maintenance::{MaintenancePolicy, advisory_unlock, try_advisory_lock};
use sqlx::PgPool;
use support::{NewUser, audit_keyring, insert_session, insert_user, prepare_catalog, services};
use uuid::Uuid;

fn audit_event() -> AuditEvent {
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

fn default_policy() -> MaintenancePolicy {
    MaintenancePolicy::default()
}

async fn audit_row_hashes(pool: &PgPool) -> Vec<(i64, Vec<u8>, String)> {
    sqlx::query_as("SELECT chain_seq, row_hash, action FROM audit_logs ORDER BY chain_seq")
        .fetch_all(pool)
        .await
        .expect("audit rows")
}

#[sqlx::test(migrations = "../../migrations")]
async fn maintenance_cleans_expired_state_and_writes_the_purge_anchor(pool: PgPool) {
    prepare_catalog(&pool).await;
    let user_id = insert_user(
        &pool,
        NewUser {
            role: None,
            ..NewUser::active("maintenance.user", "maintenance.user@example.com")
        },
    )
    .await;

    // Old session and token, both past the 30-day retention.
    let session_id = insert_session(&pool, user_id).await;
    sqlx::query(
        "UPDATE auth_sessions
         SET created_at = now() - interval '40 days',
             last_seen_at = now() - interval '40 days',
             revoked_at = now() - interval '40 days'
         WHERE id = $1",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("age session");

    let writer = services().audit();
    for _ in 0..3 {
        writer.write(&pool, &audit_event()).await.expect("audit");
    }
    let head_hash = audit_row_hashes(&pool).await[2].1.clone();

    let future = Utc::now() + Duration::days(400);
    let run = run_maintenance_at(&pool, writer, &default_policy(), future)
        .await
        .expect("maintenance");

    assert_eq!(run.status, MaintenanceStatus::Completed);
    assert!(run.outcome.refresh_tokens_deleted >= 1);
    assert_eq!(run.outcome.sessions_deleted, 1);
    assert_eq!(run.outcome.audit.count, 3);
    assert_eq!(run.outcome.audit.from_seq, Some(1));
    assert_eq!(run.outcome.audit.to_seq, Some(3));

    let rows = audit_row_hashes(&pool).await;
    assert_eq!(rows.len(), 1, "only the purge event remains");
    assert_eq!(rows[0].0, 4);
    assert_eq!(rows[0].2, "system_audit_logs_purged");

    let details: serde_json::Value =
        sqlx::query_scalar("SELECT details FROM audit_logs WHERE chain_seq = 4")
            .fetch_one(&pool)
            .await
            .expect("details");
    assert_eq!(details["from_seq"], 1);
    assert_eq!(details["to_seq"], 3);
    assert_eq!(details["count"], 3);
    assert_eq!(details["retention_days"], 180);
    assert_eq!(details["retention_critical_days"], 365);
    assert_eq!(
        details["last_purged_hash"].as_str().expect("hash"),
        head_hash
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );

    let head: (i64, Vec<u8>) =
        sqlx::query_as("SELECT last_seq, last_hash FROM audit_chain_head WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("head");
    assert_eq!(head.0, 4);
    assert_eq!(head.1, rows[0].1);

    verify_chain(&pool, &audit_keyring())
        .await
        .expect("the post-purge chain verifies");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_second_instance_exits_without_work_when_the_lock_is_held(pool: PgPool) {
    prepare_catalog(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: None,
            ..NewUser::active("locked.user", "locked.user@example.com")
        },
    )
    .await;
    let writer = services().audit();
    writer.write(&pool, &audit_event()).await.expect("audit");

    let mut lock_connection = pool.acquire().await.expect("connection");
    assert!(
        try_advisory_lock(&mut lock_connection).await.expect("lock"),
        "the test holds the maintenance lock"
    );

    let run = run_maintenance_at(
        &pool,
        writer,
        &default_policy(),
        Utc::now() + Duration::days(400),
    )
    .await
    .expect("maintenance");

    assert_eq!(run.status, MaintenanceStatus::SkippedLockHeld);
    assert_eq!(
        run.outcome,
        rs_admin_store::maintenance::MaintenanceOutcome::default()
    );
    assert_eq!(
        audit_row_hashes(&pool).await.len(),
        1,
        "nothing was deleted"
    );

    advisory_unlock(&mut lock_connection).await.expect("unlock");
}

#[sqlx::test(migrations = "../../migrations")]
async fn maintenance_without_expired_rows_writes_no_purge_event(pool: PgPool) {
    let writer = services().audit();
    writer.write(&pool, &audit_event()).await.expect("audit");

    let run = run_maintenance_at(&pool, writer, &default_policy(), Utc::now())
        .await
        .expect("maintenance");

    assert_eq!(run.status, MaintenanceStatus::Completed);
    assert_eq!(run.outcome.audit.count, 0);
    assert_eq!(audit_row_hashes(&pool).await.len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_failed_purge_event_rolls_the_audit_deletion_back(pool: PgPool) {
    let writer = services().audit();
    for _ in 0..2 {
        writer.write(&pool, &audit_event()).await.expect("audit");
    }

    // Removing the head makes the purge event write fail after the cleanup
    // statements have run.
    sqlx::query("DELETE FROM audit_chain_head")
        .execute(&pool)
        .await
        .expect("break head");

    let error = run_maintenance_at(
        &pool,
        writer,
        &default_policy(),
        Utc::now() + Duration::days(400),
    )
    .await
    .expect_err("the purge event cannot be written");

    assert!(matches!(error, CliError::Audit(_)));
    assert_eq!(
        audit_row_hashes(&pool).await.len(),
        2,
        "the audit deletion rolled back with the failed event"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn batched_purges_keep_the_chain_verifiable(pool: PgPool) {
    let writer = services().audit();
    for _ in 0..3 {
        writer.write(&pool, &audit_event()).await.expect("audit");
    }

    let mut policy = default_policy();
    policy.audit_batch_size = 1;
    let future = Utc::now() + Duration::days(400);

    for _ in 0..6 {
        let run = run_maintenance_at(&pool, writer, &policy, future)
            .await
            .expect("maintenance");
        assert_eq!(run.status, MaintenanceStatus::Completed);
        assert_eq!(
            run.outcome.audit.count, 1,
            "one bounded batch is removed per run"
        );
        verify_chain(&pool, &audit_keyring())
            .await
            .expect("every intermediate chain state verifies");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_recent_session_is_not_touched(pool: PgPool) {
    prepare_catalog(&pool).await;
    let user_id = insert_user(
        &pool,
        NewUser {
            role: None,
            ..NewUser::active("recent.user", "recent.user@example.com")
        },
    )
    .await;
    let session_id = insert_session(&pool, user_id).await;

    let run = run_maintenance_at(&pool, services().audit(), &default_policy(), Utc::now())
        .await
        .expect("maintenance");

    assert_eq!(run.outcome.refresh_tokens_deleted, 0);
    assert_eq!(run.outcome.sessions_deleted, 0);
    let tokens: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM auth_refresh_tokens WHERE session_id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("tokens");
    assert_eq!(tokens, 1);
}
