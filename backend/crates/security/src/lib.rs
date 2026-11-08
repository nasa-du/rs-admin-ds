//! Key material parsing and validation shared by authentication and audit.
//!
//! Secrets are wrapped so that `Debug` output and error messages never reveal
//! key material. Parsing errors are intentionally coarse: they name the
//! problem, not the offending bytes.

pub mod jwt;
pub mod password;
pub mod throttle;
pub mod token;

pub use jwt::{
    ACCESS_TOKEN_TYPE, AccessTokenClaims, JWT_ACCESS_TOKEN_TTL_SECONDS, JWT_LEEWAY_SECONDS,
    JwtError, JwtIssuer,
};
pub use password::{
    ARGON2_MEMORY_KIB, ARGON2_OUTPUT_BYTES, ARGON2_PARALLELISM, ARGON2_TIME_COST,
    PASSWORD_MAX_BYTES, PASSWORD_MAX_CHARS, PASSWORD_MIN_CHARS, PasswordError, PasswordPolicy,
    PasswordService, PolicyViolation, VerifyOutcome, exceeds_authentication_limit,
};
pub use throttle::{
    ClientIpResolution, ClientIpSource, TrustedProxies, account_lock_seconds, global_throttle_key,
    identifier_throttle_key, ip_throttle_key, normalize_ip,
};
pub use token::{
    REFRESH_TOKEN_BYTES, TokenError, derive_refresh_token, generate_refresh_token,
    hash_refresh_token, identifier_hmac_hex,
};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::pkcs8::DecodePrivateKey;
use jsonwebtoken::{DecodingKey, EncodingKey};
use secrecy::{ExposeSecret, SecretBox};
use std::collections::BTreeMap;
use std::fmt;

/// Minimum size of every HMAC or derivation key (spec §8.7).
pub const MIN_HMAC_KEY_BYTES: usize = 32;

/// Errors produced while parsing key material. Messages never contain values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("key is not a valid PEM-encoded Ed25519 key")]
    InvalidEd25519Pem,
    #[error("key is not valid base64")]
    InvalidBase64,
    #[error("key is shorter than the required {minimum} bytes")]
    KeyTooShort { minimum: usize },
    #[error("audit chain keyring is not a valid key-id-to-base64-key object")]
    InvalidKeyRing,
    #[error("audit chain keyring does not contain the current key id")]
    CurrentKeyMissing,
    #[error("JWT issuer, audience and key id must be non-empty")]
    InvalidJwtConfiguration,
}

/// A symmetric key used for HMAC or key derivation.
#[derive(Clone)]
pub struct HmacKey {
    bytes: SecretBox<[u8]>,
}

impl HmacKey {
    /// Decode a standard-base64 key. Surrounding whitespace is ignored.
    pub fn from_base64(encoded: &str) -> Result<Self, KeyError> {
        let bytes = STANDARD
            .decode(encoded.trim())
            .map_err(|_| KeyError::InvalidBase64)?;
        Self::from_bytes(bytes)
    }

    /// Wrap raw key bytes, enforcing the 32-byte minimum.
    pub fn from_bytes(bytes: impl Into<Box<[u8]>>) -> Result<Self, KeyError> {
        let bytes = bytes.into();
        if bytes.len() < MIN_HMAC_KEY_BYTES {
            return Err(KeyError::KeyTooShort {
                minimum: MIN_HMAC_KEY_BYTES,
            });
        }
        Ok(Self {
            bytes: SecretBox::new(bytes),
        })
    }

    /// Expose the key bytes to the cryptographic primitive that needs them.
    pub fn expose(&self) -> &[u8] {
        self.bytes.expose_secret()
    }

    pub fn len(&self) -> usize {
        self.bytes.expose_secret().len()
    }

    pub fn is_empty(&self) -> bool {
        self.expose().is_empty()
    }
}

impl fmt::Debug for HmacKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HmacKey([REDACTED])")
    }
}

/// A key ring holding the current audit-chain key plus any older keys that are
/// still required to verify historical rows (spec §8.7).
#[derive(Clone)]
pub struct AuditKeyRing {
    current_id: String,
    keys: BTreeMap<String, HmacKey>,
}

impl AuditKeyRing {
    /// Parse the `AUDIT_CHAIN_KEYS` JSON object (`key_id → base64 key`).
    ///
    /// The ring is rejected unless it is non-empty, every key is at least 32
    /// bytes, and `current_id` is present.
    pub fn from_json(json: &str, current_id: &str) -> Result<Self, KeyError> {
        if current_id.trim().is_empty() {
            return Err(KeyError::InvalidKeyRing);
        }

        let entries: BTreeMap<String, String> =
            serde_json::from_str(json).map_err(|_| KeyError::InvalidKeyRing)?;
        if entries.is_empty() {
            return Err(KeyError::InvalidKeyRing);
        }

        let mut keys = BTreeMap::new();
        for (key_id, encoded) in entries {
            if key_id.is_empty() || key_id.trim() != key_id {
                return Err(KeyError::InvalidKeyRing);
            }
            let key = HmacKey::from_base64(&encoded)?;
            keys.insert(key_id, key);
        }

        if !keys.contains_key(current_id) {
            return Err(KeyError::CurrentKeyMissing);
        }

        Ok(Self {
            current_id: current_id.to_owned(),
            keys,
        })
    }

    pub fn current_id(&self) -> &str {
        &self.current_id
    }

    /// The key used for new rows.
    pub fn current(&self) -> &HmacKey {
        self.keys
            .get(&self.current_id)
            .expect("validated keyring contains the current key")
    }

    /// Look up a historical key by the `hmac_key_id` recorded on a row.
    pub fn get(&self, key_id: &str) -> Option<&HmacKey> {
        self.keys.get(key_id)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

impl fmt::Debug for AuditKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuditKeyRing")
            .field("current_id", &self.current_id)
            .field("key_ids", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// A validated Ed25519 signing key loaded from PKCS#8 PEM.
#[derive(Clone)]
pub struct JwtSigningKey(EncodingKey);

impl JwtSigningKey {
    pub fn from_pem(pem: &str) -> Result<Self, KeyError> {
        EncodingKey::from_ed_pem(pem.as_bytes())
            .map(Self)
            .map_err(|_| KeyError::InvalidEd25519Pem)
    }

    /// The underlying key, for the JWT signer.
    pub fn encoding_key(&self) -> &EncodingKey {
        &self.0
    }

    /// Derive the Ed25519 public key that verifies tokens signed with this
    /// private key.
    pub fn verification_key(&self) -> Result<JwtVerificationKey, KeyError> {
        let signing_key = ed25519_dalek::SigningKey::from_pkcs8_der(self.0.as_bytes())
            .map_err(|_| KeyError::InvalidEd25519Pem)?;
        Ok(JwtVerificationKey(DecodingKey::from_ed_der(
            &signing_key.verifying_key().to_bytes(),
        )))
    }
}

impl fmt::Debug for JwtSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JwtSigningKey([REDACTED])")
    }
}

/// A validated Ed25519 public key loaded from SPKI PEM, used to verify tokens
/// signed with a rotated-out key.
#[derive(Clone)]
pub struct JwtVerificationKey(DecodingKey);

impl JwtVerificationKey {
    pub fn from_public_pem(pem: &str) -> Result<Self, KeyError> {
        DecodingKey::from_ed_pem(pem.as_bytes())
            .map(Self)
            .map_err(|_| KeyError::InvalidEd25519Pem)
    }

    /// The underlying key, for the JWT verifier added in a later change.
    pub fn decoding_key(&self) -> &DecodingKey {
        &self.0
    }
}

impl fmt::Debug for JwtVerificationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JwtVerificationKey([REDACTED])")
    }
}
