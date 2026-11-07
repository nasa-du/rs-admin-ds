//! Transactional audit writer.
//!
//! Every write locks the single chain-head row, appends one linked row and
//! advances the head inside the caller's transaction. The caller must treat
//! an error as fatal for the business action (fail-closed).

use crate::error::AuditError;
use crate::event::AuditEvent;
use crate::hash::{AuditRow, compute_row_hash};
use rs_admin_security::AuditKeyRing;
use rs_admin_store::audit;
use sqlx::{PgPool, Postgres, Transaction};
use std::net::IpAddr;
use uuid::Uuid;

/// Identifiers of the row that was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditRecord {
    pub id: Uuid,
    pub chain_seq: i64,
    pub row_hash: [u8; 32],
}

/// Writes audit events into the tamper-evident chain.
#[derive(Clone)]
pub struct AuditWriter {
    keys: AuditKeyRing,
}

impl AuditWriter {
    pub fn new(keys: AuditKeyRing) -> Self {
        Self { keys }
    }

    /// Append an event inside the caller's transaction. On error the caller
    /// must roll the transaction back; no audit row is visible either way.
    pub async fn write_in_tx(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        event: &AuditEvent,
    ) -> Result<AuditRecord, AuditError> {
        let head = audit::lock_head(transaction)
            .await?
            .ok_or(AuditError::ChainHeadMissing)?;
        let chain_seq = head
            .last_seq
            .checked_add(1)
            .ok_or(AuditError::CorruptChainHead)?;
        let occurred_at = audit::transaction_now(transaction).await?;

        let row = AuditRow {
            id: Uuid::now_v7(),
            occurred_at,
            actor_user_id: event.actor().user_id(),
            actor_label: event.actor().label().to_owned(),
            action: event.action().as_str().to_owned(),
            target_type: event.target().target_type().as_str().to_owned(),
            target_id: event.target().id(),
            target_label: event.target().label().map(str::to_owned),
            result: event.result().as_str().to_owned(),
            severity: event.severity().as_str().to_owned(),
            ip: event.ip().map(|ip| normalize_ip(ip).to_string()),
            user_agent: event.user_agent().map(str::to_owned),
            request_id: event.request_id(),
            details: event.details().clone(),
            chain_seq,
            prev_hash: head.last_hash,
            hmac_key_id: self.keys.current_id().to_owned(),
        };
        let row_hash = compute_row_hash(self.keys.current(), &row)?;

        audit::append(transaction, &row, &row_hash).await?;
        audit::advance_head(transaction, chain_seq, &row_hash).await?;

        Ok(AuditRecord {
            id: row.id,
            chain_seq,
            row_hash,
        })
    }

    /// Append an event in its own transaction. Used for events that have no
    /// business transaction (for example a failed login); a failure means the
    /// HTTP response must be 503.
    pub async fn write(
        &self,
        pool: &PgPool,
        event: &AuditEvent,
    ) -> Result<AuditRecord, AuditError> {
        let mut transaction = pool.begin().await.map_err(AuditError::Database)?;
        let record = self.write_in_tx(&mut transaction, event).await?;
        transaction.commit().await.map_err(AuditError::Database)?;
        Ok(record)
    }
}

/// PostgreSQL renders IPv4-mapped IPv6 addresses as IPv6, while the stored
/// canonical row must round-trip through `IpAddr::to_string()`; normalize to
/// IPv4 first so the hash remains reproducible.
fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ipv6) => ipv6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        IpAddr::V4(_) => ip,
    }
}
