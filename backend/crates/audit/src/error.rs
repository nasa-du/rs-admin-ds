//! Audit errors. Display and Debug output are stable and never include SQL,
//! connection details or key material.

use crate::event::AuditEventError;
use rs_admin_store::audit::AuditStoreError;
use std::fmt;

/// Errors from writing audit rows.
#[derive(thiserror::Error)]
pub enum AuditError {
    #[error("audit event is invalid: {0}")]
    InvalidEvent(#[from] AuditEventError),
    #[error("audit chain head is missing")]
    ChainHeadMissing,
    #[error("audit chain head is corrupt")]
    CorruptChainHead,
    #[error("audit row is corrupt")]
    CorruptRow,
    #[error("audit row cannot be canonicalized")]
    Canonicalization(#[source] serde_json::Error),
    #[error("audit write failed")]
    Database(#[source] sqlx::Error),
}

impl fmt::Debug for AuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AuditError")
            .field(&self.to_string())
            .finish()
    }
}

impl From<AuditStoreError> for AuditError {
    fn from(error: AuditStoreError) -> Self {
        match error {
            AuditStoreError::Database(source) => Self::Database(source),
            AuditStoreError::CorruptRow => Self::CorruptRow,
            AuditStoreError::CorruptHead => Self::CorruptChainHead,
        }
    }
}

/// Where a chain verification stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainBreakReason {
    PrefixMissing { found: i64 },
    AnchorMismatch,
    SequenceGap { expected: i64, found: i64 },
    PrevHashMismatch,
    RowHashMismatch,
    UnknownKeyId,
    CorruptRow,
    HeadMissing,
    HeadSeqMismatch { head: i64, last: i64 },
    HeadHashMismatch,
}

impl fmt::Display for ChainBreakReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrefixMissing { found } => write!(
                formatter,
                "chain starts at {found} but no purge anchor is present"
            ),
            Self::AnchorMismatch => {
                formatter.write_str("first row does not start at the zero hash anchor")
            }
            Self::SequenceGap { expected, found } => {
                write!(
                    formatter,
                    "sequence gap: expected {expected}, found {found}"
                )
            }
            Self::PrevHashMismatch => {
                formatter.write_str("prev_hash does not link to the previous row")
            }
            Self::RowHashMismatch => {
                formatter.write_str("row_hash does not match the canonical row")
            }
            Self::UnknownKeyId => formatter.write_str("hmac_key_id is not in the keyring"),
            Self::CorruptRow => formatter.write_str("row cannot be represented canonically"),
            Self::HeadMissing => formatter.write_str("audit_chain_head is missing"),
            Self::HeadSeqMismatch { head, last } => write!(
                formatter,
                "audit_chain_head.last_seq is {head} but the last row is {last}"
            ),
            Self::HeadHashMismatch => {
                formatter.write_str("audit_chain_head.last_hash does not match the last row")
            }
        }
    }
}

/// A broken chain, located by sequence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainBreak {
    pub chain_seq: i64,
    pub reason: ChainBreakReason,
}

impl fmt::Display for ChainBreak {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "audit chain is broken at sequence {}: {}",
            self.chain_seq, self.reason
        )
    }
}

impl std::error::Error for ChainBreak {}

/// Errors from chain verification.
#[derive(thiserror::Error)]
pub enum VerifyError {
    #[error("{0}")]
    Broken(#[from] ChainBreak),
    #[error("audit chain could not be read")]
    Database(#[source] sqlx::Error),
}

impl fmt::Debug for VerifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("VerifyError")
            .field(&self.to_string())
            .finish()
    }
}

impl VerifyError {
    pub(crate) fn from_store(error: AuditStoreError, chain_seq: i64) -> Self {
        match error {
            AuditStoreError::Database(source) => Self::Database(source),
            AuditStoreError::CorruptRow | AuditStoreError::CorruptHead => {
                Self::Broken(ChainBreak {
                    chain_seq,
                    reason: ChainBreakReason::CorruptRow,
                })
            }
        }
    }
}
