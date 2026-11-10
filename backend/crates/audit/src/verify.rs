//! Recompute the audit chain from stored rows and detect tampering.
//!
//! Verification starts at the zero-hash anchor when the chain still begins at
//! `chain_seq = 1`. Retention purges remove a prefix of the chain, so when the
//! earliest surviving row has a higher sequence its `prev_hash` must match the
//! `last_purged_hash` recorded by a `system_audit_logs_purged` event. The
//! surviving rows must then be contiguous and linked to the chain head.

use crate::catalog::AuditAction;
use crate::error::{ChainBreak, ChainBreakReason, VerifyError};
use crate::hash::compute_row_hash;
use rs_admin_security::AuditKeyRing;
use rs_admin_store::audit::{self, StoredAuditRow};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashSet;

/// Result of a successful chain verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainVerification {
    pub verified_rows: u64,
    pub head_seq: i64,
}

/// Recompute every stored row hash and verify the links, anchor and chain
/// head.
pub async fn verify_chain(
    pool: &PgPool,
    keys: &AuditKeyRing,
) -> Result<ChainVerification, VerifyError> {
    let stored_rows = audit::load_rows(pool)
        .await
        .map_err(|error| VerifyError::from_store(error, 0))?;

    let purge_anchors = purge_anchors(&stored_rows);
    let mut previous_seq: Option<i64> = None;
    let mut previous_hash: Option<[u8; 32]> = None;

    for stored in &stored_rows {
        let seq = stored.row.chain_seq;

        match previous_seq {
            None if seq == 1 => {
                if stored.row.prev_hash != [0u8; 32] {
                    return Err(chain_break(seq, ChainBreakReason::AnchorMismatch).into());
                }
            }
            None => {
                // A retention purge removed the head of the chain. The first
                // surviving row must be linked to a recorded purge anchor.
                if !purge_anchors.contains(&stored.row.prev_hash) {
                    return Err(
                        chain_break(seq, ChainBreakReason::PrefixMissing { found: seq }).into(),
                    );
                }
            }
            Some(expected) => {
                if seq != expected + 1 {
                    return Err(chain_break(
                        seq,
                        ChainBreakReason::SequenceGap {
                            expected: expected + 1,
                            found: seq,
                        },
                    )
                    .into());
                }
                if Some(stored.row.prev_hash) != previous_hash {
                    return Err(chain_break(seq, ChainBreakReason::PrevHashMismatch).into());
                }
            }
        }

        let key = keys
            .get(&stored.row.hmac_key_id)
            .ok_or_else(|| chain_break(seq, ChainBreakReason::UnknownKeyId))?;
        let computed = compute_row_hash(key, &stored.row)
            .map_err(|_| chain_break(seq, ChainBreakReason::CorruptRow))?;
        if computed != stored.row_hash {
            return Err(chain_break(seq, ChainBreakReason::RowHashMismatch).into());
        }

        previous_seq = Some(seq);
        previous_hash = Some(stored.row_hash);
    }

    let last_seq = previous_seq.unwrap_or(0);
    let head = audit::load_head(pool)
        .await
        .map_err(|error| VerifyError::from_store(error, last_seq))?;
    let Some(head) = head else {
        return Err(chain_break(last_seq, ChainBreakReason::HeadMissing).into());
    };

    if head.last_seq != last_seq {
        return Err(chain_break(
            head.last_seq,
            ChainBreakReason::HeadSeqMismatch {
                head: head.last_seq,
                last: last_seq,
            },
        )
        .into());
    }

    let expected_hash = previous_hash.unwrap_or([0u8; 32]);
    if head.last_hash != expected_hash {
        return Err(chain_break(head.last_seq, ChainBreakReason::HeadHashMismatch).into());
    }

    Ok(ChainVerification {
        verified_rows: stored_rows.len() as u64,
        head_seq: head.last_seq,
    })
}

/// Hashes recorded as the tail of a removed chain prefix.
fn purge_anchors(rows: &[StoredAuditRow]) -> HashSet<[u8; 32]> {
    let action = AuditAction::SystemAuditLogsPurged.as_str();
    rows.iter()
        .filter(|stored| stored.row.action == action)
        .filter_map(|stored| stored.row.details.get("last_purged_hash"))
        .filter_map(Value::as_str)
        .filter_map(decode_hash)
        .collect()
}

/// Decode a lowercase or uppercase 64-character hex SHA-256 value.
fn decode_hash(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut hash = [0u8; 32];
    for (index, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(value.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(hash)
}

fn chain_break(chain_seq: i64, reason: ChainBreakReason) -> ChainBreak {
    ChainBreak { chain_seq, reason }
}
