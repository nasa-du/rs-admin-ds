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

/// SHA-256 of a raw refresh token, as stored in `auth_refresh_tokens`.
pub fn hash_refresh_token(raw: &str) -> [u8; 32] {
    Sha256::digest(raw.as_bytes()).into()
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
