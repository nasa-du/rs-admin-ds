//! Access-token signing and verification (docs/specs/login-system.md §5.1).
//!
//! Tokens are EdDSA (Ed25519) JWTs with `typ=at+jwt`, a local `kid` and a
//! fixed 15-minute lifetime. Verification only trusts the locally configured
//! key: `jku`, `jwk` and `x5u` headers are rejected outright and an unknown
//! `kid` is an error rather than a key lookup.

use crate::{JwtSigningKey, JwtVerificationKey, KeyError};
use jsonwebtoken::{
    Algorithm, Header, TokenData, Validation, decode, decode_header, encode, errors::ErrorKind,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Access-token lifetime in seconds.
pub const JWT_ACCESS_TOKEN_TTL_SECONDS: i64 = 900;

/// Clock skew applied to `exp` and `nbf`.
pub const JWT_LEEWAY_SECONDS: u64 = 60;

/// Required token type in both the header and the claims.
pub const ACCESS_TOKEN_TYPE: &str = "at+jwt";

/// Claims carried by an access token. Roles and permissions are deliberately
/// absent; they are resolved from PostgreSQL on every request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessTokenClaims {
    pub sub: Uuid,
    pub sid: Uuid,
    pub ver: i64,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub nbf: i64,
    pub jti: Uuid,
    pub typ: String,
    pub auth_time: i64,
}

impl AccessTokenClaims {
    /// Build a claim set with the fixed 15-minute lifetime.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sub: Uuid,
        sid: Uuid,
        ver: i64,
        auth_time: i64,
        issuer: &str,
        audience: &str,
        issued_at: i64,
    ) -> Self {
        Self {
            sub,
            sid,
            ver,
            iss: issuer.to_owned(),
            aud: audience.to_owned(),
            exp: issued_at + JWT_ACCESS_TOKEN_TTL_SECONDS,
            iat: issued_at,
            nbf: issued_at,
            jti: Uuid::now_v7(),
            typ: ACCESS_TOKEN_TYPE.to_owned(),
            auth_time,
        }
    }
}

/// A configured access-token issuer and verifier.
#[derive(Clone)]
pub struct JwtIssuer {
    issuer: String,
    audience: String,
    kid: String,
    signing_key: JwtSigningKey,
    verification_key: JwtVerificationKey,
}

impl JwtIssuer {
    /// Validate the configured identity and derive the public key used to
    /// verify tokens signed by this key.
    pub fn new(
        issuer: &str,
        audience: &str,
        kid: &str,
        signing_key: &JwtSigningKey,
    ) -> Result<Self, KeyError> {
        if issuer.trim().is_empty() || audience.trim().is_empty() || kid.trim().is_empty() {
            return Err(KeyError::InvalidJwtConfiguration);
        }

        Ok(Self {
            issuer: issuer.to_owned(),
            audience: audience.to_owned(),
            kid: kid.to_owned(),
            signing_key: signing_key.clone(),
            verification_key: signing_key.verification_key()?,
        })
    }

    pub fn signing_key(&self) -> &JwtSigningKey {
        &self.signing_key
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// Sign a claim set with the `at+jwt` header and configured `kid`.
    pub fn sign(&self, claims: &AccessTokenClaims) -> Result<String, JwtError> {
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some(ACCESS_TOKEN_TYPE.to_owned());
        header.kid = Some(self.kid.clone());

        encode(&header, claims, self.signing_key.encoding_key()).map_err(|_| JwtError::Signing)
    }

    /// Verify the signature and every claim of an access token.
    pub fn verify(&self, token: &str) -> Result<AccessTokenClaims, JwtError> {
        let header = decode_header(token).map_err(|_| JwtError::InvalidToken)?;

        if header.alg != Algorithm::EdDSA {
            return Err(JwtError::InvalidHeader);
        }
        if header.jku.is_some() || header.jwk.is_some() || header.x5u.is_some() {
            return Err(JwtError::InvalidHeader);
        }
        if header.typ.as_deref() != Some(ACCESS_TOKEN_TYPE) {
            return Err(JwtError::InvalidHeader);
        }
        match header.kid.as_deref() {
            Some(kid) if kid == self.kid => {}
            Some(_) => return Err(JwtError::UnknownKid),
            None => return Err(JwtError::InvalidHeader),
        }

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.leeway = JWT_LEEWAY_SECONDS;
        validation.validate_nbf = true;
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        validation.required_spec_claims = ["exp", "nbf", "iss", "aud", "sub"]
            .into_iter()
            .map(str::to_owned)
            .collect();

        let TokenData { claims, .. } =
            decode::<AccessTokenClaims>(token, self.verification_key.decoding_key(), &validation)
                .map_err(|error| match error.kind() {
                ErrorKind::ExpiredSignature => JwtError::Expired,
                ErrorKind::ImmatureSignature => JwtError::NotYetValid,
                _ => JwtError::InvalidToken,
            })?;

        if claims.typ != ACCESS_TOKEN_TYPE
            || claims.iss != self.issuer
            || claims.aud != self.audience
        {
            return Err(JwtError::InvalidHeader);
        }

        let lifetime = claims.exp - claims.iat;
        if lifetime <= 0 || lifetime > JWT_ACCESS_TOKEN_TTL_SECONDS {
            return Err(JwtError::InvalidLifetime);
        }

        Ok(claims)
    }
}

/// Verification failures. All variants map to a generic 401 in the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum JwtError {
    #[error("access token is invalid")]
    InvalidToken,
    #[error("access token header is invalid")]
    InvalidHeader,
    #[error("access token key id is unknown")]
    UnknownKid,
    #[error("access token lifetime is invalid")]
    InvalidLifetime,
    #[error("access token has expired")]
    Expired,
    #[error("access token is not yet valid")]
    NotYetValid,
    #[error("access token could not be signed")]
    Signing,
}
