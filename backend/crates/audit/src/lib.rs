//! Closed audit event catalog, RFC 8785 canonicalization, HMAC chain and
//! transactional writer.
//!
//! The writer is deliberately synchronous with the caller's transaction:
//! `write_in_tx` either appends and updates the chain head inside that
//! transaction or returns an error, so callers must propagate the error and
//! roll the business action back (fail-closed, spec §8.5).

mod catalog;
mod error;
mod event;
mod hash;
mod jcs;
mod verify;
mod writer;

pub use catalog::{
    AuditAction, AuditResult, ResultRule, Severity, SeverityRule, TargetRule, TargetType,
    UnknownCatalogValue,
};
pub use error::{AuditError, ChainBreak, ChainBreakReason, VerifyError};
pub use event::{
    AuditActor, AuditEvent, AuditEventBuilder, AuditEventError, AuditTarget,
    redact_forbidden_details,
};
pub use hash::{AuditRow, canonical_audit_row, compute_row_hash};
pub use jcs::canonical_json;
pub use verify::{ChainVerification, verify_chain};
pub use writer::{AuditRecord, AuditWriter};
