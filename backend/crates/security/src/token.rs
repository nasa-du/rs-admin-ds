//! Refresh tokens and keyed identifier helpers
//! (docs/specs/login-system.md §5.2, §8.3).
//!
//! Refresh tokens are 256-bit opaque values encoded as unpadded base64url. The
//! database only ever stores their SHA-256 digest.

use crate::HmacKey;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// Entropy of an initial refresh token.
pub const REFRESH_TOKEN_BYTES: usize = 32;

/// Entropy behind an administrator-issued temporary password. 18 bytes encode
/// to exactly 24 base64url characters (spec §7.7, ADR-0003 §3).
pub const TEMPORARY_PASSWORD_BYTES: usize = 18;

/// Errors from token generation. The message never includes key material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("secure random number generation failed")]
    Random,
}

/// Generate an initial refresh token from the operating system CSPRNG.
pub fn generate_refresh_token() -> Result<String, TokenError> {
    let mut bytes = [0u8; REFRESH_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|_| TokenError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Generate an administrator-issued temporary password: 24 base64url
/// characters (about 144 bits) from the operating system CSPRNG.
pub fn generate_temporary_password() -> Result<String, TokenError> {
    let mut bytes = [0u8; TEMPORARY_PASSWORD_BYTES];
    getrandom::fill(&mut bytes).map_err(|_| TokenError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// SHA-256 of a raw refresh token, as stored in `auth_refresh_tokens`.
pub fn hash_refresh_token(raw: &str) -> [u8; 32] {
    Sha256::digest(raw.as_bytes()).into()
}

/// Derive the next raw refresh token in a session's rotation chain
/// (spec §5.2):
///
/// `base64url(HMAC-SHA256(key, previous_raw || ":" || counter))`.
///
/// `counter` is the successor's `rotation_counter` (the previous row's
/// counter plus one). The database only ever stores the SHA-256 of the
/// result; derivation is deterministic so a grace-window retry can recompute
/// the same successor.
pub fn derive_refresh_token(key: &HmacKey, previous_raw: &str, counter: i64) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.expose()).expect("HMAC accepts keys of any length");
    mac.update(previous_raw.as_bytes());
    mac.update(b":");
    mac.update(counter.to_string().as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// HMAC-SHA256 of a normalized login identifier, used to record unknown
/// identifiers in audit and throttle state without persisting them.
pub fn identifier_hmac_hex(key: &HmacKey, identifier: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.expose()).expect("HMAC accepts keys of any length");
    mac.update(identifier.as_bytes());
    let bytes = mac.finalize().into_bytes();

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
