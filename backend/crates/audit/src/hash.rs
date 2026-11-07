//! Canonical audit row hashing.
//!
//! `row_hash = HMAC-SHA256(key, prev_hash || 0x00 || JCS(row))`, where `row`
//! contains every stored column except `row_hash` (spec §8.5.3). The row type
//! lives in the store layer; this module owns its canonical form.

pub use rs_admin_store::audit::AuditRow;

use crate::error::AuditError;
use crate::jcs::canonical_json;
use chrono::SecondsFormat;
use hmac::{Hmac, Mac};
use rs_admin_security::HmacKey;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::Sha256;
use uuid::Uuid;

/// Canonical JSON of the row, including `prev_hash` as lowercase hex.
pub fn canonical_audit_row(row: &AuditRow) -> Result<String, AuditError> {
    let occurred_at = row.occurred_at.to_rfc3339_opts(SecondsFormat::Micros, true);
    let prev_hash = hex_encode(&row.prev_hash);
    let canonical = CanonicalRow {
        id: row.id,
        occurred_at,
        actor_user_id: row.actor_user_id,
        actor_label: &row.actor_label,
        action: &row.action,
        target_type: &row.target_type,
        target_id: row.target_id,
        target_label: row.target_label.as_deref(),
        result: &row.result,
        severity: &row.severity,
        ip: row.ip.as_deref(),
        user_agent: row.user_agent.as_deref(),
        request_id: row.request_id,
        details: &row.details,
        chain_seq: row.chain_seq,
        prev_hash,
        hmac_key_id: &row.hmac_key_id,
    };
    canonical_json(&canonical)
}

#[derive(Serialize)]
struct CanonicalRow<'a> {
    id: Uuid,
    occurred_at: String,
    actor_user_id: Option<Uuid>,
    actor_label: &'a str,
    action: &'a str,
    target_type: &'a str,
    target_id: Option<Uuid>,
    target_label: Option<&'a str>,
    result: &'a str,
    severity: &'a str,
    ip: Option<&'a str>,
    user_agent: Option<&'a str>,
    request_id: Uuid,
    details: &'a Map<String, Value>,
    chain_seq: i64,
    prev_hash: String,
    hmac_key_id: &'a str,
}

/// Compute the HMAC of a canonical row chained to `row.prev_hash`.
pub fn compute_row_hash(key: &HmacKey, row: &AuditRow) -> Result<[u8; 32], AuditError> {
    let canonical = canonical_audit_row(row)?;

    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.expose()).expect("HMAC accepts keys of any length");
    mac.update(&row.prev_hash);
    mac.update(&[0u8]);
    mac.update(canonical.as_bytes());

    Ok(mac.finalize().into_bytes().into())
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
