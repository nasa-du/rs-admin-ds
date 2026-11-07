//! SQL for the tamper-evident audit chain.
//!
//! The audit crate owns the event catalog, canonicalization and hashing; this
//! module owns every statement that touches the audit tables, keeping `store`
//! as the only crate that writes SQL (spec §2.2).

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::{PgPool, Postgres, Transaction};
use std::fmt;
use uuid::Uuid;

/// Canonical audit row fields hashed by the audit crate (everything except
/// `row_hash`).
#[derive(Clone, Debug, PartialEq)]
pub struct AuditRow {
    pub id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub actor_user_id: Option<Uuid>,
    pub actor_label: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub target_label: Option<String>,
    pub result: String,
    pub severity: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub request_id: Uuid,
    pub details: Map<String, Value>,
    pub chain_seq: i64,
    pub prev_hash: [u8; 32],
    pub hmac_key_id: String,
}

/// The single chain-head row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainHead {
    pub last_seq: i64,
    pub last_hash: [u8; 32],
}

/// A stored row and the hash recorded next to it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredAuditRow {
    pub row: AuditRow,
    pub row_hash: [u8; 32],
}

/// Errors from the audit SQL layer. Display and debug never include SQL.
#[derive(thiserror::Error)]
pub enum AuditStoreError {
    #[error("audit store query failed")]
    Database(#[source] sqlx::Error),
    #[error("stored audit row is corrupt")]
    CorruptRow,
    #[error("stored audit chain head is corrupt")]
    CorruptHead,
}

impl fmt::Debug for AuditStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AuditStoreError")
            .field(&self.to_string())
            .finish()
    }
}

/// Lock the chain head for the rest of the transaction.
pub async fn lock_head(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<Option<ChainHead>, AuditStoreError> {
    let head: Option<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT last_seq, last_hash FROM audit_chain_head WHERE id = 1 FOR UPDATE")
            .fetch_optional(&mut **transaction)
            .await
            .map_err(AuditStoreError::Database)?;

    head.map(|(last_seq, last_hash)| {
        Ok(ChainHead {
            last_seq,
            last_hash: to_hash(&last_hash).ok_or(AuditStoreError::CorruptHead)?,
        })
    })
    .transpose()
}

/// The database clock within the current transaction.
pub async fn transaction_now(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<DateTime<Utc>, AuditStoreError> {
    sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut **transaction)
        .await
        .map_err(AuditStoreError::Database)
}

/// Append one linked audit row.
pub async fn append(
    transaction: &mut Transaction<'_, Postgres>,
    row: &AuditRow,
    row_hash: &[u8; 32],
) -> Result<(), AuditStoreError> {
    sqlx::query(
        "INSERT INTO audit_logs (
            id, occurred_at, actor_user_id, actor_label, action, target_type,
            target_id, target_label, result, severity, ip, user_agent,
            request_id, details, chain_seq, prev_hash, row_hash, hmac_key_id
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::inet, $12,
            $13, $14, $15, $16, $17, $18
         )",
    )
    .bind(row.id)
    .bind(row.occurred_at)
    .bind(row.actor_user_id)
    .bind(&row.actor_label)
    .bind(&row.action)
    .bind(&row.target_type)
    .bind(row.target_id)
    .bind(row.target_label.as_deref())
    .bind(&row.result)
    .bind(&row.severity)
    .bind(row.ip.as_deref())
    .bind(row.user_agent.as_deref())
    .bind(row.request_id)
    .bind(sqlx::types::Json(&row.details))
    .bind(row.chain_seq)
    .bind(row.prev_hash.as_slice())
    .bind(row_hash.as_slice())
    .bind(&row.hmac_key_id)
    .execute(&mut **transaction)
    .await
    .map_err(AuditStoreError::Database)?;

    Ok(())
}

/// Advance the locked chain head to the row just appended.
pub async fn advance_head(
    transaction: &mut Transaction<'_, Postgres>,
    last_seq: i64,
    last_hash: &[u8; 32],
) -> Result<(), AuditStoreError> {
    let updated = sqlx::query(
        "UPDATE audit_chain_head
         SET last_seq = $1, last_hash = $2, updated_at = now()
         WHERE id = 1",
    )
    .bind(last_seq)
    .bind(last_hash.as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(AuditStoreError::Database)?;

    if updated.rows_affected() != 1 {
        return Err(AuditStoreError::CorruptHead);
    }
    Ok(())
}

/// Read every stored row in chain order.
pub async fn load_rows(pool: &PgPool) -> Result<Vec<StoredAuditRow>, AuditStoreError> {
    let stored: Vec<StoredRow> = sqlx::query_as(
        "SELECT
            id, occurred_at, actor_user_id, actor_label, action, target_type,
            target_id, target_label, result, severity, host(ip) AS ip, user_agent,
            request_id, details, chain_seq, prev_hash, row_hash, hmac_key_id
         FROM audit_logs
         ORDER BY chain_seq",
    )
    .fetch_all(pool)
    .await
    .map_err(AuditStoreError::Database)?;

    stored.into_iter().map(StoredRow::into_parts).collect()
}

/// Read the chain head without locking it.
pub async fn load_head(pool: &PgPool) -> Result<Option<ChainHead>, AuditStoreError> {
    let head: Option<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT last_seq, last_hash FROM audit_chain_head WHERE id = 1")
            .fetch_optional(pool)
            .await
            .map_err(AuditStoreError::Database)?;

    head.map(|(last_seq, last_hash)| {
        Ok(ChainHead {
            last_seq,
            last_hash: to_hash(&last_hash).ok_or(AuditStoreError::CorruptHead)?,
        })
    })
    .transpose()
}

#[derive(sqlx::FromRow)]
struct StoredRow {
    id: Uuid,
    occurred_at: DateTime<Utc>,
    actor_user_id: Option<Uuid>,
    actor_label: String,
    action: String,
    target_type: String,
    target_id: Option<Uuid>,
    target_label: Option<String>,
    result: String,
    severity: String,
    ip: Option<String>,
    user_agent: Option<String>,
    request_id: Uuid,
    details: sqlx::types::Json<Value>,
    chain_seq: i64,
    prev_hash: Vec<u8>,
    row_hash: Vec<u8>,
    hmac_key_id: String,
}

impl StoredRow {
    fn into_parts(self) -> Result<StoredAuditRow, AuditStoreError> {
        let prev_hash = to_hash(&self.prev_hash).ok_or(AuditStoreError::CorruptRow)?;
        let row_hash = to_hash(&self.row_hash).ok_or(AuditStoreError::CorruptRow)?;
        let details = match self.details.0 {
            Value::Object(details) => details,
            _ => return Err(AuditStoreError::CorruptRow),
        };

        Ok(StoredAuditRow {
            row: AuditRow {
                id: self.id,
                occurred_at: self.occurred_at,
                actor_user_id: self.actor_user_id,
                actor_label: self.actor_label,
                action: self.action,
                target_type: self.target_type,
                target_id: self.target_id,
                target_label: self.target_label,
                result: self.result,
                severity: self.severity,
                ip: self.ip,
                user_agent: self.user_agent,
                request_id: self.request_id,
                details,
                chain_seq: self.chain_seq,
                prev_hash,
                hmac_key_id: self.hmac_key_id,
            },
            row_hash,
        })
    }
}

fn to_hash(bytes: &[u8]) -> Option<[u8; 32]> {
    bytes.try_into().ok()
}
