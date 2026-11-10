//! Retention cleanup against a real PostgreSQL schema (spec §4.5, AC-OPS-07).

use chrono::{DateTime, Duration, Utc};
use rs_admin_store::maintenance::{
    AuditPurgeOutcome, MaintenancePolicy, advisory_unlock, cleanup, try_advisory_lock,
};
use sqlx::PgPool;
use uuid::Uuid;

const DAY: Duration = Duration::days(1);

fn token_hash(seed: u8) -> [u8; 32] {
    [seed; 32]
}

async fn insert_user(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (username, email, display_name, password_hash)
         VALUES ('maintenance.test', 'maintenance.test@example.com', 'maintenance.test', 'x')
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("user")
}

struct Session {
    created_at: DateTime<Utc>,
    idle_expires_at: DateTime<Utc>,
    absolute_expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

async fn insert_session(pool: &PgPool, user_id: Uuid, session: Session) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO auth_sessions (
            user_id, created_at, last_seen_at, idle_expires_at,
            absolute_expires_at, authenticated_at, revoked_at
         ) VALUES ($1, $2, $2, $3, $4, $2, $5)
         RETURNING id",
    )
    .bind(user_id)
    .bind(session.created_at)
    .bind(session.idle_expires_at)
    .bind(session.absolute_expires_at)
    .bind(session.revoked_at)
    .fetch_one(pool)
    .await
    .expect("session")
}

async fn insert_token(
    pool: &PgPool,
    session_id: Uuid,
    seed: u8,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
) {
    sqlx::query(
        "INSERT INTO auth_refresh_tokens (
            token_hash, session_id, rotation_counter, issued_at, expires_at, revoked_at
         ) VALUES ($1, $2, 0, now(), $3, $4)",
    )
    .bind(token_hash(seed).as_slice())
    .bind(session_id)
    .bind(expires_at)
    .bind(revoked_at)
    .execute(pool)
    .await
    .expect("refresh token");
}

async fn insert_login_failure(
    pool: &PgPool,
    seed: u8,
    updated_at: DateTime<Utc>,
    locked_until: Option<DateTime<Utc>>,
) {
    sqlx::query(
        "INSERT INTO auth_login_failures (key_hash, failed_count, locked_until, updated_at)
         VALUES ($1, 3, $2, $3)",
    )
    .bind(token_hash(seed).as_slice())
    .bind(locked_until)
    .bind(updated_at)
    .execute(pool)
    .await
    .expect("login failure");
}

async fn insert_throttle(
    pool: &PgPool,
    seed: u8,
    last_failure_at: DateTime<Utc>,
    locked_until: Option<DateTime<Utc>>,
) {
    sqlx::query(
        "INSERT INTO auth_login_throttle (
            scope, key_hash, window_start, failures, locked_until, last_failure_at
         ) VALUES ('ip', $1, $2, 3, $3, $2)",
    )
    .bind(token_hash(seed).as_slice())
    .bind(last_failure_at)
    .bind(locked_until)
    .execute(pool)
    .await
    .expect("login throttle");
}

async fn insert_audit_row(
    pool: &PgPool,
    chain_seq: i64,
    severity: &str,
    occurred_at: DateTime<Utc>,
) -> [u8; 32] {
    let row_hash: [u8; 32] = [chain_seq as u8; 32];
    sqlx::query(
        "INSERT INTO audit_logs (
            occurred_at, actor_label, action, target_type, result, severity,
            request_id, details, chain_seq, prev_hash, row_hash, hmac_key_id
         ) VALUES ($1, 'system', 'system_audit_logs_purged', 'system', 'success',
                   $2, uuidv7(), '{}'::jsonb, $3, $4, $5, 'test-key')",
    )
    .bind(occurred_at)
    .bind(severity)
    .bind(chain_seq)
    .bind(vec![0u8; 32])
    .bind(row_hash.as_slice())
    .execute(pool)
    .await
    .expect("audit row");
    row_hash
}

async fn run_cleanup(
    pool: &PgPool,
    policy: MaintenancePolicy,
    now: DateTime<Utc>,
) -> rs_admin_store::maintenance::MaintenanceOutcome {
    let mut transaction = pool.begin().await.expect("begin");
    let outcome = cleanup(&mut transaction, now, &policy)
        .await
        .expect("cleanup");
    transaction.commit().await.expect("commit");
    outcome
}

fn policy() -> MaintenancePolicy {
    MaintenancePolicy {
        session_retention_days: 30,
        audit_info_retention_days: 180,
        audit_critical_retention_days: 365,
        table_batch_size: 1000,
        audit_batch_size: 1000,
        max_batches_per_table: 10,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn cleanup_removes_expired_tokens_sessions_and_unlocked_throttle_rows(pool: PgPool) {
    let now = Utc::now();
    let user_id = insert_user(&pool).await;

    // Session whose tokens and row are both old enough to delete.
    let old_revoked = insert_session(
        &pool,
        user_id,
        Session {
            created_at: now - DAY * 60,
            idle_expires_at: now + DAY,
            absolute_expires_at: now + DAY,
            revoked_at: Some(now - DAY * 40),
        },
    )
    .await;
    insert_token(&pool, old_revoked, 1, now + DAY, Some(now - DAY * 40)).await;

    // Session that expired over 30 days ago without an explicit revoke.
    let old_idle = insert_session(
        &pool,
        user_id,
        Session {
            created_at: now - DAY * 60,
            idle_expires_at: now - DAY * 40,
            absolute_expires_at: now - DAY * 39,
            revoked_at: None,
        },
    )
    .await;
    insert_token(&pool, old_idle, 2, now - DAY * 40, None).await;

    // Revoked ten days ago: still inside the 30-day window.
    let recent_revoked = insert_session(
        &pool,
        user_id,
        Session {
            created_at: now - DAY * 20,
            idle_expires_at: now + DAY,
            absolute_expires_at: now + DAY,
            revoked_at: Some(now - DAY * 10),
        },
    )
    .await;
    insert_token(&pool, recent_revoked, 3, now + DAY, Some(now - DAY * 10)).await;

    // Active session and token must survive.
    let active = insert_session(
        &pool,
        user_id,
        Session {
            created_at: now - DAY,
            idle_expires_at: now + DAY,
            absolute_expires_at: now + DAY * 10,
            revoked_at: None,
        },
    )
    .await;
    insert_token(&pool, active, 4, now + DAY * 10, None).await;

    insert_login_failure(&pool, 10, now - DAY * 40, None).await;
    insert_login_failure(&pool, 11, now - DAY * 40, Some(now + Duration::minutes(5))).await;
    insert_login_failure(&pool, 12, now - DAY * 10, None).await;
    insert_throttle(&pool, 20, now - DAY * 40, None).await;
    insert_throttle(&pool, 21, now - DAY * 40, Some(now + Duration::minutes(5))).await;
    insert_throttle(&pool, 22, now - DAY * 10, None).await;

    let outcome = run_cleanup(&pool, policy(), now).await;

    assert_eq!(outcome.refresh_tokens_deleted, 2);
    assert_eq!(outcome.sessions_deleted, 2);
    assert_eq!(outcome.login_failures_deleted, 1);
    assert_eq!(outcome.login_throttle_deleted, 1);
    assert_eq!(outcome.audit, AuditPurgeOutcome::default());

    let sessions: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM auth_sessions ORDER BY id")
        .fetch_all(&pool)
        .await
        .expect("sessions");
    assert_eq!(sessions.len(), 2, "the recent and active sessions remain");
    assert!(sessions.contains(&recent_revoked));
    assert!(sessions.contains(&active));

    let failures: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth_login_failures")
        .fetch_one(&pool)
        .await
        .expect("failures");
    assert_eq!(failures, 2, "a locked failure is retained");
    let throttles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth_login_throttle")
        .fetch_one(&pool)
        .await
        .expect("throttles");
    assert_eq!(throttles, 2, "a locked throttle row is retained");
}

#[sqlx::test(migrations = "../../migrations")]
async fn audit_cleanup_stops_before_the_first_retained_row(pool: PgPool) {
    let now = Utc::now();
    let first = insert_audit_row(&pool, 1, "info", now - DAY * 200).await;
    insert_audit_row(&pool, 2, "error", now - DAY * 200).await;
    insert_audit_row(&pool, 3, "info", now - DAY * 200).await;
    insert_audit_row(&pool, 4, "warning", now - DAY * 10).await;
    insert_audit_row(&pool, 5, "critical", now - DAY * 400).await;

    let mut settings = policy();
    settings.audit_batch_size = 1000;
    let outcome = run_cleanup(&pool, settings, now).await;

    assert_eq!(
        outcome.audit,
        AuditPurgeOutcome {
            from_seq: Some(1),
            to_seq: Some(1),
            count: 1,
            last_purged_hash: Some(first),
        },
        "the 200-day-old error row is still inside its 365-day tier"
    );
    let remaining: Vec<i64> =
        sqlx::query_scalar("SELECT chain_seq FROM audit_logs ORDER BY chain_seq")
            .fetch_all(&pool)
            .await
            .expect("audit rows");
    assert_eq!(remaining, [2, 3, 4, 5]);

    let second = run_cleanup(&pool, settings, now).await;
    assert_eq!(second.audit, AuditPurgeOutcome::default());
}

#[sqlx::test(migrations = "../../migrations")]
async fn audit_cleanup_batches_the_expired_prefix(pool: PgPool) {
    let now = Utc::now();
    let hashes: Vec<[u8; 32]> = {
        let mut hashes = Vec::new();
        for seq in 1..=5 {
            hashes.push(insert_audit_row(&pool, seq, "info", now - DAY * 200).await);
        }
        hashes
    };

    let mut settings = policy();
    settings.audit_batch_size = 2;

    let first = run_cleanup(&pool, settings, now).await;
    assert_eq!(
        first.audit,
        AuditPurgeOutcome {
            from_seq: Some(1),
            to_seq: Some(2),
            count: 2,
            last_purged_hash: Some(hashes[1]),
        }
    );

    let second = run_cleanup(&pool, settings, now).await;
    assert_eq!(
        second.audit,
        AuditPurgeOutcome {
            from_seq: Some(3),
            to_seq: Some(4),
            count: 2,
            last_purged_hash: Some(hashes[3]),
        }
    );

    let third = run_cleanup(&pool, settings, now).await;
    assert_eq!(
        third.audit,
        AuditPurgeOutcome {
            from_seq: Some(5),
            to_seq: Some(5),
            count: 1,
            last_purged_hash: Some(hashes[4]),
        }
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(remaining, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn audit_cleanup_removes_every_expired_row(pool: PgPool) {
    let now = Utc::now();
    insert_audit_row(&pool, 1, "info", now - DAY * 400).await;
    insert_audit_row(&pool, 2, "critical", now - DAY * 400).await;
    let final_hash = insert_audit_row(&pool, 3, "warning", now - DAY * 400).await;

    let outcome = run_cleanup(&pool, policy(), now).await;

    assert_eq!(outcome.audit.count, 3);
    assert_eq!(outcome.audit.from_seq, Some(1));
    assert_eq!(outcome.audit.to_seq, Some(3));
    assert_eq!(
        outcome.audit.last_purged_hash,
        Some(final_hash),
        "the anchor must be the last deleted row"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_maintenance_lock_is_exclusive(pool: PgPool) {
    let mut first = pool.acquire().await.expect("connection");
    assert!(
        try_advisory_lock(&mut first).await.expect("lock"),
        "the first connection takes the lock"
    );

    let mut second = pool.acquire().await.expect("connection");
    assert!(
        !try_advisory_lock(&mut second).await.expect("lock attempt"),
        "a concurrent run must not take the lock"
    );
    drop(second);

    advisory_unlock(&mut first).await.expect("unlock");
    let mut third = pool.acquire().await.expect("connection");
    assert!(
        try_advisory_lock(&mut third).await.expect("lock"),
        "the lock is available after release"
    );
    advisory_unlock(&mut third).await.expect("unlock");
}
