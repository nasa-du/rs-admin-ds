//! `record-key-rotation`: append the `crypto_key_rotated` audit event
//! (ADR-0005 §8.4, ADR-0008 §3.5).
//!
//! Key material lives outside the database and is rotated by the operator. The
//! audit chain still needs a record of each rotation, so this command writes a
//! critical `crypto_key_rotated` event with the key type, old/new key ids and
//! the documented reason. It never reads or prints the key itself.

use crate::error::CliError;
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, AuditWriter};
use rs_admin_security::AuditKeyRing;
use sqlx::PgPool;
use uuid::Uuid;

/// The application key classes covered by `crypto_key_rotated`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyType {
    Jwt,
    RefreshDerivation,
    Throttle,
    AuditChain,
}

impl KeyType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jwt => "jwt",
            Self::RefreshDerivation => "refresh_derivation",
            Self::Throttle => "throttle",
            Self::AuditChain => "audit_chain",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "jwt" => Some(Self::Jwt),
            "refresh_derivation" => Some(Self::RefreshDerivation),
            "throttle" => Some(Self::Throttle),
            "audit_chain" => Some(Self::AuditChain),
            _ => None,
        }
    }
}

/// Why the key was rotated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationReason {
    Scheduled,
    Compromise,
}

impl RotationReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Compromise => "compromise",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "scheduled" => Some(Self::Scheduled),
            "compromise" => Some(Self::Compromise),
            _ => None,
        }
    }
}

/// Inputs for one rotation audit record.
pub struct KeyRotationInput<'a> {
    pub key_type: KeyType,
    pub old_kid: &'a str,
    pub new_kid: &'a str,
    pub reason: RotationReason,
}

/// Result of appending the rotation event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyRotationOutcome {
    pub audit_id: Uuid,
    pub chain_seq: i64,
}

/// Append the critical `crypto_key_rotated` event in its own transaction.
pub async fn record_key_rotation(
    pool: &PgPool,
    keys: &AuditKeyRing,
    input: KeyRotationInput<'_>,
) -> Result<KeyRotationOutcome, CliError> {
    let event = AuditEvent::builder(
        AuditAction::CryptoKeyRotated,
        AuditResult::Success,
        AuditActor::cli(),
        Uuid::now_v7(),
    )
    .detail("key_type", input.key_type.as_str())
    .detail("old_kid", input.old_kid)
    .detail("new_kid", input.new_kid)
    .detail("reason", input.reason.as_str())
    .build()?;

    let record = AuditWriter::new(keys.clone()).write(pool, &event).await?;

    Ok(KeyRotationOutcome {
        audit_id: record.id,
        chain_seq: record.chain_seq,
    })
}
