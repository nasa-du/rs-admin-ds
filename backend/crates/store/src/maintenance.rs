//! Retention cleanup SQL for the scheduled maintenance job (spec §4.5,
//! ADR-0008 §7.2).
//!
//! The job deletes in dependency order: refresh tokens, sessions, login
//! failures/throttle rows, then an aged prefix of the audit chain. Audit rows
//! are removed as a contiguous prefix so the HMAC chain keeps verifying: the
//! newest deleted row's hash becomes the anchor recorded by the
//! `system_audit_logs_purged` event written in the same transaction.
//!
//! Every statement is bounded so a first run on a large backlog cannot hold a
//! long transaction. Errors never carry SQL or connection details.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Postgres, Transaction};
use std::fmt;

/// Fixed advisory-lock key for the maintenance job. Derived from the ASCII
/// bytes `RSADMIN` plus a format version byte; distinct from the catalog-sync
/// and bootstrap keys.
pub const MAINTENANCE_LOCK_KEY: i64 = 0x5253_4144_4D49_4E03;

/// Default retention for expired/revoked sessions and refresh tokens, and for
/// unlocked login-failure/throttle rows (spec §4.5).
pub const DEFAULT_SESSION_RETENTION_DAYS: u32 = 30;

/// Default number of rows removed per delete statement for session/token and
/// throttle tables.
pub const DEFAULT_TABLE_BATCH_SIZE: u32 = 10_000;

/// Default number of audit rows removed per maintenance run.
pub const DEFAULT_AUDIT_BATCH_SIZE: u32 = 5_000;

/// Upper bound on delete batches per table per run, so a large backlog is
/// drained over several scheduled runs.
pub const DEFAULT_MAX_BATCHES_PER_TABLE: u32 = 20;

/// Retention and batching knobs for one maintenance run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaintenancePolicy {
    pub session_retention_days: u32,
    pub audit_info_retention_days: u32,
    pub audit_critical_retention_days: u32,
    pub table_batch_size: u32,
    pub audit_batch_size: u32,
    pub max_batches_per_table: u32,
}

impl Default for MaintenancePolicy {
    fn default() -> Self {
        Self {
            session_retention_days: DEFAULT_SESSION_RETENTION_DAYS,
            audit_info_retention_days: 180,
            audit_critical_retention_days: 365,
            table_batch_size: DEFAULT_TABLE_BATCH_SIZE,
            audit_batch_size: DEFAULT_AUDIT_BATCH_SIZE,
            max_batches_per_table: DEFAULT_MAX_BATCHES_PER_TABLE,
        }
    }
}

/// What one maintenance run removed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MaintenanceOutcome {
    pub refresh_tokens_deleted: u64,
    pub sessions_deleted: u64,
    pub login_failures_deleted: u64,
    pub login_throttle_deleted: u64,
    pub audit: AuditPurgeOutcome,
}

/// The removed audit prefix. Empty when no audit row was old enough.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditPurgeOutcome {
    pub from_seq: Option<i64>,
    pub to_seq: Option<i64>,
    pub count: u64,
    pub last_purged_hash: Option<[u8; 32]>,
}

/// Errors from the maintenance SQL layer. Display and debug never include SQL
/// or connection details.
#[derive(thiserror::Error)]
pub enum MaintenanceError {
    #[error("maintenance database query failed")]
    Database(#[source] sqlx::Error),
    #[error("stored audit row is corrupt")]
    CorruptAuditRow,
}

impl fmt::Debug for MaintenanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("MaintenanceError")
            .field(&self.to_string())
            .finish()
    }
}

/// Try to take the session-level maintenance lock. The caller must keep the
/// connection alive until it runs [`advisory_unlock`]; a lost connection
/// releases the lock automatically.
pub async fn try_advisory_lock(connection: &mut PgConnection) -> Result<bool, MaintenanceError> {
    sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(MAINTENANCE_LOCK_KEY)
        .fetch_one(&mut *connection)
        .await
        .map_err(MaintenanceError::Database)
}

/// Release the session-level maintenance lock.
pub async fn advisory_unlock(connection: &mut PgConnection) -> Result<(), MaintenanceError> {
    sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1)")
        .bind(MAINTENANCE_LOCK_KEY)
        .fetch_one(&mut *connection)
        .await
        .map_err(MaintenanceError::Database)?;
    Ok(())
}

/// The database clock, captured once so every cutoff in a run is consistent.
pub async fn database_now(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<DateTime<Utc>, MaintenanceError> {
    sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut **transaction)
        .await
        .map_err(MaintenanceError::Database)
}

/// Run one cleanup transaction. The caller must hold the maintenance advisory
/// lock. The audit purge and its `system_audit_logs_purged` event are written
/// by the caller in this same transaction, so a failed event write rolls the
/// deletions back.
pub async fn cleanup(
    transaction: &mut Transaction<'_, Postgres>,
    now: DateTime<Utc>,
    policy: &MaintenancePolicy,
) -> Result<MaintenanceOutcome, MaintenanceError> {
    let session_days = days_i32(policy.session_retention_days);
    let batch = i64::from(policy.table_batch_size.max(1));

    let refresh_tokens_deleted = loop_delete(
        transaction,
        refresh_token_delete_sql(),
        now,
        session_days,
        batch,
        policy.max_batches_per_table,
    )
    .await?;
    let sessions_deleted = loop_delete(
        transaction,
        session_delete_sql(),
        now,
        session_days,
        batch,
        policy.max_batches_per_table,
    )
    .await?;
    let login_failures_deleted = loop_delete(
        transaction,
        login_failure_delete_sql(),
        now,
        session_days,
        batch,
        policy.max_batches_per_table,
    )
    .await?;
    let login_throttle_deleted = loop_delete(
        transaction,
        login_throttle_delete_sql(),
        now,
        session_days,
        batch,
        policy.max_batches_per_table,
    )
    .await?;
    let audit = purge_audit_prefix(transaction, now, policy).await?;

    Ok(MaintenanceOutcome {
        refresh_tokens_deleted,
        sessions_deleted,
        login_failures_deleted,
        login_throttle_deleted,
        audit,
    })
}

/// Delete in batches until fewer than one batch is removed or the per-run cap
/// is reached. All four statements bind `($now, $days, $limit)`.
async fn loop_delete(
    transaction: &mut Transaction<'_, Postgres>,
    sql: &'static str,
    now: DateTime<Utc>,
    days: i32,
    batch: i64,
    max_batches: u32,
) -> Result<u64, MaintenanceError> {
    let mut deleted_total = 0u64;
    for _ in 0..max_batches.max(1) {
        let deleted = sqlx::query(sql)
            .bind(now)
            .bind(days)
            .bind(batch)
            .execute(&mut **transaction)
            .await
            .map_err(MaintenanceError::Database)?
            .rows_affected();
        deleted_total += deleted;
        if deleted < batch as u64 {
            break;
        }
    }
    Ok(deleted_total)
}

fn refresh_token_delete_sql() -> &'static str {
    "WITH doomed AS (
        SELECT t.token_hash
        FROM auth_refresh_tokens t
        WHERE t.expires_at < $1 - make_interval(days => $2)
           OR (t.revoked_at IS NOT NULL AND t.revoked_at < $1 - make_interval(days => $2))
           OR EXISTS (
                SELECT 1
                FROM auth_sessions s
                WHERE s.id = t.session_id
                  AND (
                       (s.revoked_at IS NOT NULL AND s.revoked_at < $1 - make_interval(days => $2))
                       OR s.absolute_expires_at < $1 - make_interval(days => $2)
                       OR (s.idle_expires_at < $1 - make_interval(days => $2) AND s.revoked_at IS NULL)
                  )
           )
        ORDER BY t.expires_at
        LIMIT $3
     )
     DELETE FROM auth_refresh_tokens t
     USING doomed d
     WHERE t.token_hash = d.token_hash"
}

fn session_delete_sql() -> &'static str {
    "WITH doomed AS (
        SELECT s.id
        FROM auth_sessions s
        WHERE (
                (s.revoked_at IS NOT NULL AND s.revoked_at < $1 - make_interval(days => $2))
                OR s.absolute_expires_at < $1 - make_interval(days => $2)
                OR (s.idle_expires_at < $1 - make_interval(days => $2) AND s.revoked_at IS NULL)
              )
          AND NOT EXISTS (
                SELECT 1 FROM auth_refresh_tokens t WHERE t.session_id = s.id
              )
        ORDER BY s.id
        LIMIT $3
     )
     DELETE FROM auth_sessions s
     USING doomed d
     WHERE s.id = d.id"
}

fn login_failure_delete_sql() -> &'static str {
    "WITH doomed AS (
        SELECT f.key_hash
        FROM auth_login_failures f
        WHERE f.updated_at < $1 - make_interval(days => $2)
          AND (f.locked_until IS NULL OR f.locked_until <= $1)
        ORDER BY f.updated_at
        LIMIT $3
     )
     DELETE FROM auth_login_failures f
     USING doomed d
     WHERE f.key_hash = d.key_hash"
}

fn login_throttle_delete_sql() -> &'static str {
    "WITH doomed AS (
        SELECT t.scope, t.key_hash
        FROM auth_login_throttle t
        WHERE t.last_failure_at < $1 - make_interval(days => $2)
          AND (t.locked_until IS NULL OR t.locked_until <= $1)
        ORDER BY t.last_failure_at
        LIMIT $3
     )
     DELETE FROM auth_login_throttle t
     USING doomed d
     WHERE t.scope = d.scope AND t.key_hash = d.key_hash"
}

/// Remove one bounded prefix of audit rows that is past its severity tier's
/// retention. Rows before the first still-retained row are deleted, so the
/// remainder stays a contiguous chain. Returns the removed interval and the
/// last removed row's hash (the anchor for verification).
async fn purge_audit_prefix(
    transaction: &mut Transaction<'_, Postgres>,
    now: DateTime<Utc>,
    policy: &MaintenancePolicy,
) -> Result<AuditPurgeOutcome, MaintenanceError> {
    let info_days = days_i32(policy.audit_info_retention_days);
    let critical_days = days_i32(policy.audit_critical_retention_days);

    // The first row that is still inside its severity tier. Every row with a
    // lower chain_seq is expired, because severity only decides which cutoff
    // applies.
    let boundary: Option<i64> = sqlx::query_scalar(
        "SELECT chain_seq
         FROM audit_logs
         WHERE (severity IN ('info', 'warning') AND occurred_at >= $1 - make_interval(days => $2))
            OR (severity IN ('error', 'critical') AND occurred_at >= $1 - make_interval(days => $3))
         ORDER BY chain_seq
         LIMIT 1",
    )
    .bind(now)
    .bind(info_days)
    .bind(critical_days)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(MaintenanceError::Database)?;

    let removed: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "WITH doomed AS (
            SELECT chain_seq
            FROM audit_logs
            WHERE ($1::bigint IS NULL OR chain_seq < $1)
            ORDER BY chain_seq
            LIMIT $2
         )
         DELETE FROM audit_logs a
         USING doomed d
         WHERE a.chain_seq = d.chain_seq
         RETURNING a.chain_seq, a.row_hash",
    )
    .bind(boundary)
    .bind(i64::from(policy.audit_batch_size.max(1)))
    .fetch_all(&mut **transaction)
    .await
    .map_err(MaintenanceError::Database)?;

    if removed.is_empty() {
        return Ok(AuditPurgeOutcome::default());
    }

    let count = removed.len() as u64;
    let from_seq = removed.iter().map(|(seq, _)| *seq).min();
    let (to_seq, last_hash) = removed
        .into_iter()
        .max_by_key(|(seq, _)| *seq)
        .expect("non-empty");
    let last_purged_hash: [u8; 32] = last_hash
        .try_into()
        .map_err(|_| MaintenanceError::CorruptAuditRow)?;

    Ok(AuditPurgeOutcome {
        from_seq,
        to_seq: Some(to_seq),
        count,
        last_purged_hash: Some(last_purged_hash),
    })
}

fn days_i32(days: u32) -> i32 {
    i32::try_from(days).unwrap_or(i32::MAX)
}
