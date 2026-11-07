//! Recompute the audit chain from stored rows and detect tampering.
//!
//! Verification starts at the zero-hash anchor: the chain must begin at
//! `chain_seq = 1` with a 32-byte zero `prev_hash`. Retention purges are not
//! implemented yet; when they land, the verifier must accept a purge event's
//! recorded anchor instead of requiring sequence 1.

use crate::error::{ChainBreak, ChainBreakReason, VerifyError};
use crate::hash::compute_row_hash;
use rs_admin_security::AuditKeyRing;
use rs_admin_store::audit;
use sqlx::PgPool;

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

    let mut previous_hash: Option<[u8; 32]> = None;
    let mut verified_rows = 0u64;

    for (expected_seq, stored) in (1_i64..).zip(stored_rows) {
        let seq = stored.row.chain_seq;

        if seq != expected_seq {
            let reason = if expected_seq == 1 {
                ChainBreakReason::PrefixMissing { found: seq }
            } else {
                ChainBreakReason::SequenceGap {
                    expected: expected_seq,
                    found: seq,
                }
            };
            return Err(chain_break(seq, reason).into());
        }

        if expected_seq == 1 && stored.row.prev_hash != [0u8; 32] {
            return Err(chain_break(seq, ChainBreakReason::AnchorMismatch).into());
        }

        if let Some(previous_hash) = previous_hash
            && stored.row.prev_hash != previous_hash
        {
            return Err(chain_break(seq, ChainBreakReason::PrevHashMismatch).into());
        }

        let key = keys
            .get(&stored.row.hmac_key_id)
            .ok_or_else(|| chain_break(seq, ChainBreakReason::UnknownKeyId))?;
        let computed = compute_row_hash(key, &stored.row)
            .map_err(|_| chain_break(seq, ChainBreakReason::CorruptRow))?;
        if computed != stored.row_hash {
            return Err(chain_break(seq, ChainBreakReason::RowHashMismatch).into());
        }

        previous_hash = Some(stored.row_hash);
        verified_rows += 1;
    }

    let last_seq = verified_rows as i64;
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
        verified_rows,
        head_seq: head.last_seq,
    })
}

fn chain_break(chain_seq: i64, reason: ChainBreakReason) -> ChainBreak {
    ChainBreak { chain_seq, reason }
}
