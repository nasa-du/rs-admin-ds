//! RFC 8785 JSON Canonicalization Scheme.

use crate::error::AuditError;
use serde::Serialize;

/// Serialize a value using RFC 8785 JCS.
///
/// JCS sorts object keys by UTF-16 code units, escapes strings the same way
/// ECMAScript does, and formats numbers with ECMAScript's shortest
/// round-trip representation. `serde_jcs` implements all three.
pub fn canonical_json<T>(value: &T) -> Result<String, AuditError>
where
    T: Serialize + ?Sized,
{
    serde_jcs::to_string(value).map_err(AuditError::Canonicalization)
}
