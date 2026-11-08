//! Security key material loading (spec §8.7).

use crate::secret::{optional_secret, required_secret};
use crate::{ConfigError, required};
use rs_admin_security::{AuditKeyRing, HmacKey, JwtSigningKey, JwtVerificationKey};
use secrecy::{ExposeSecret, SecretString};

/// JWT identity and signing material.
#[derive(Clone, Debug)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: String,
    pub kid: String,
    pub signing_key: JwtSigningKey,
    /// Public key for tokens signed before the most recent rotation.
    pub previous_verification_key: Option<JwtVerificationKey>,
}

/// A current secret plus the optional predecessor that remains valid during a
/// rotation window.
#[derive(Clone, Debug)]
pub struct RotationKeys {
    pub current: HmacKey,
    pub previous: Option<HmacKey>,
}

impl RotationKeys {
    /// Keys that may have produced an existing derivation, current first.
    pub fn candidates(&self) -> impl Iterator<Item = &HmacKey> {
        std::iter::once(&self.current).chain(self.previous.as_ref())
    }
}

/// Audit chain signing key ring.
#[derive(Clone, Debug)]
pub struct AuditChainConfig {
    pub key_id: String,
    pub keys: AuditKeyRing,
}

impl AuditChainConfig {
    /// Load only the audit chain key material. The management CLI needs this
    /// and nothing else, so bootstrap/recovery do not require JWT or refresh
    /// keys.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_source(&|key| std::env::var(key).ok())
    }

    pub fn from_source<F>(lookup: &F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let key_id = required(lookup, "AUDIT_CHAIN_KEY_ID")?;
        let keyring_json = SecretString::from(required_secret(
            lookup,
            "AUDIT_CHAIN_KEYS",
            "AUDIT_CHAIN_KEYS_FILE",
        )?);
        let keys =
            AuditKeyRing::from_json(keyring_json.expose_secret(), &key_id).map_err(|error| {
                match error {
                    rs_admin_security::KeyError::CurrentKeyMissing => {
                        ConfigError::Invalid("AUDIT_CHAIN_KEY_ID")
                    }
                    _ => ConfigError::Invalid("AUDIT_CHAIN_KEYS"),
                }
            })?;

        Ok(Self { key_id, keys })
    }
}

/// All key material used by authentication and audit.
#[derive(Clone, Debug)]
pub struct SecurityConfig {
    pub jwt: JwtConfig,
    pub refresh_derivation: RotationKeys,
    pub throttle_key: HmacKey,
    pub audit_chain: AuditChainConfig,
}

impl SecurityConfig {
    pub fn from_source<F>(lookup: &F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let issuer = required(lookup, "JWT_ISSUER")?;
        let audience = required(lookup, "JWT_AUDIENCE")?;
        let kid = required(lookup, "JWT_EDDSA_KID")?;

        let signing_pem = SecretString::from(required_secret(
            lookup,
            "JWT_EDDSA_PRIVATE_KEY",
            "JWT_EDDSA_PRIVATE_KEY_FILE",
        )?);
        let signing_key = JwtSigningKey::from_pem(signing_pem.expose_secret())
            .map_err(|_| ConfigError::Invalid("JWT_EDDSA_PRIVATE_KEY"))?;

        let previous_verification_key = optional_secret(
            lookup,
            "JWT_EDDSA_PREVIOUS_PUBLIC_KEY",
            "JWT_EDDSA_PREVIOUS_PUBLIC_KEY_FILE",
        )?
        .map(|pem| {
            JwtVerificationKey::from_public_pem(&pem)
                .map_err(|_| ConfigError::Invalid("JWT_EDDSA_PREVIOUS_PUBLIC_KEY"))
        })
        .transpose()?;

        let refresh_current = hmac_key(
            lookup,
            "REFRESH_DERIVATION_KEY",
            "REFRESH_DERIVATION_KEY_FILE",
        )?;
        let refresh_previous = optional_hmac_key(
            lookup,
            "REFRESH_DERIVATION_KEY_PREVIOUS",
            "REFRESH_DERIVATION_KEY_PREVIOUS_FILE",
        )?;
        let throttle_key = hmac_key(lookup, "THROTTLE_KEY", "THROTTLE_KEY_FILE")?;

        let audit_chain = AuditChainConfig::from_source(lookup)?;

        Ok(Self {
            jwt: JwtConfig {
                issuer,
                audience,
                kid,
                signing_key,
                previous_verification_key,
            },
            refresh_derivation: RotationKeys {
                current: refresh_current,
                previous: refresh_previous,
            },
            throttle_key,
            audit_chain,
        })
    }
}

fn hmac_key<F>(
    lookup: &F,
    key: &'static str,
    file_key: &'static str,
) -> Result<HmacKey, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let encoded = SecretString::from(required_secret(lookup, key, file_key)?);
    HmacKey::from_base64(encoded.expose_secret()).map_err(|_| ConfigError::Invalid(key))
}

fn optional_hmac_key<F>(
    lookup: &F,
    key: &'static str,
    file_key: &'static str,
) -> Result<Option<HmacKey>, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    optional_secret(lookup, key, file_key)?
        .map(|encoded| {
            let encoded = SecretString::from(encoded);
            HmacKey::from_base64(encoded.expose_secret()).map_err(|_| ConfigError::Invalid(key))
        })
        .transpose()
}
