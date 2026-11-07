//! Access-token signing and verification (docs/specs/login-system.md §5.1).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rs_admin_security::{
    AccessTokenClaims, JWT_ACCESS_TOKEN_TTL_SECONDS, JwtError, JwtIssuer, JwtSigningKey,
};
use serde_json::json;
use uuid::Uuid;

const KID: &str = "test-2026-11";
const ISSUER: &str = "https://api.example.com";
const AUDIENCE: &str = "rs-admin-ds-api";

fn signing_key() -> JwtSigningKey {
    let pem = SigningKey::from_bytes(&[7u8; 32])
        .to_pkcs8_pem(LineEnding::LF)
        .expect("PKCS#8")
        .to_string();
    JwtSigningKey::from_pem(&pem).expect("signing key")
}

fn issuer() -> JwtIssuer {
    JwtIssuer::new(ISSUER, AUDIENCE, KID, &signing_key()).expect("issuer")
}

fn claims(issued_at: i64) -> AccessTokenClaims {
    AccessTokenClaims::new(
        Uuid::now_v7(),
        Uuid::now_v7(),
        0,
        issued_at,
        ISSUER,
        AUDIENCE,
        issued_at,
    )
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[test]
fn a_signed_token_round_trips_with_every_required_claim() {
    let issuer = issuer();
    let original = claims(now());
    let token = issuer.sign(&original).expect("sign");
    let verified = issuer.verify(&token).expect("verify");

    assert_eq!(verified, original);
    assert_eq!(verified.exp - verified.iat, JWT_ACCESS_TOKEN_TTL_SECONDS);
    assert_eq!(verified.typ, "at+jwt");
    assert_eq!(verified.iss, ISSUER);
    assert_eq!(verified.aud, AUDIENCE);
    assert!(!verified.jti.is_nil());
}

#[test]
fn the_algorithm_header_and_typ_are_pinned() {
    let issuer = issuer();
    let token = issuer.sign(&claims(now())).expect("sign");
    let header = jsonwebtoken::decode_header(&token).expect("header");

    assert_eq!(header.alg, Algorithm::EdDSA);
    assert_eq!(header.typ.as_deref(), Some("at+jwt"));
    assert_eq!(header.kid.as_deref(), Some(KID));
    assert!(header.jku.is_none());
    assert!(header.jwk.is_none());
    assert!(header.x5u.is_none());
}

#[test]
fn unknown_kids_are_rejected() {
    let other = JwtIssuer::new(ISSUER, AUDIENCE, "other-kid", &signing_key()).expect("issuer");
    let token = other.sign(&claims(now())).expect("sign");

    assert!(matches!(issuer().verify(&token), Err(JwtError::UnknownKid)));
}

#[test]
fn unsigned_none_algorithm_tokens_are_rejected() {
    let header = URL_SAFE_NO_PAD.encode(json!({"alg": "none", "typ": "at+jwt"}).to_string());
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims(now())).expect("claims"));
    let token = format!("{header}.{payload}.");

    assert!(matches!(
        issuer().verify(&token),
        Err(JwtError::InvalidToken)
    ));
}

#[test]
fn key_material_in_the_header_is_rejected() {
    let issuer = issuer();
    let key = issuer.signing_key();

    for header in [
        Header {
            alg: Algorithm::EdDSA,
            typ: Some("at+jwt".into()),
            kid: Some(KID.into()),
            jku: Some("https://attacker.example/jwks.json".into()),
            ..Header::default()
        },
        Header {
            alg: Algorithm::EdDSA,
            typ: Some("at+jwt".into()),
            kid: Some(KID.into()),
            x5u: Some("https://attacker.example/cert.pem".into()),
            ..Header::default()
        },
        serde_json::from_value(json!({
            "alg": "EdDSA",
            "typ": "at+jwt",
            "kid": KID,
            "jwk": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            }
        }))
        .expect("jwk header"),
    ] {
        let token =
            jsonwebtoken::encode(&header, &claims(now()), key.encoding_key()).expect("sign");
        assert!(
            matches!(issuer.verify(&token), Err(JwtError::InvalidHeader)),
            "header {header:?} must be rejected"
        );
    }
}

#[test]
fn the_header_typ_must_be_at_jwt() {
    let issuer = issuer();
    let header = Header {
        alg: Algorithm::EdDSA,
        typ: Some("JWT".into()),
        kid: Some(KID.into()),
        ..Header::default()
    };
    let token = jsonwebtoken::encode(&header, &claims(now()), issuer.signing_key().encoding_key())
        .expect("sign");

    assert!(matches!(
        issuer.verify(&token),
        Err(JwtError::InvalidHeader)
    ));
}

#[test]
fn tokens_with_a_lifetime_above_fifteen_minutes_are_rejected() {
    let issuer = issuer();
    let mut claims = claims(now());
    claims.exp = claims.iat + JWT_ACCESS_TOKEN_TTL_SECONDS + 1;
    let token = issuer.sign(&claims).expect("sign");

    assert!(matches!(
        issuer.verify(&token),
        Err(JwtError::InvalidLifetime)
    ));
}

#[test]
fn expiry_and_nbf_use_a_sixty_second_leeway() {
    let issuer = issuer();
    let current = now();

    let mut just_expired = claims(current - JWT_ACCESS_TOKEN_TTL_SECONDS);
    // exp is 30 seconds in the past; within the 60-second leeway.
    just_expired.exp = current - 30;
    let token = issuer.sign(&just_expired).expect("sign");
    assert!(issuer.verify(&token).is_ok());

    let mut too_old = claims(current - 1_000);
    too_old.exp = current - 120;
    let token = issuer.sign(&too_old).expect("sign");
    assert!(matches!(issuer.verify(&token), Err(JwtError::Expired)));

    let mut not_yet = claims(current);
    not_yet.nbf = current + 30;
    not_yet.iat = current;
    let token = issuer.sign(&not_yet).expect("sign");
    assert!(issuer.verify(&token).is_ok());

    let mut far_future = claims(current);
    far_future.nbf = current + 120;
    let token = issuer.sign(&far_future).expect("sign");
    assert!(matches!(issuer.verify(&token), Err(JwtError::NotYetValid)));
}

#[test]
fn tokens_signed_with_another_key_are_rejected() {
    let other_pem = SigningKey::from_bytes(&[9u8; 32])
        .to_pkcs8_pem(LineEnding::LF)
        .expect("PKCS#8")
        .to_string();
    let other_key = JwtSigningKey::from_pem(&other_pem).expect("key");
    let other = JwtIssuer::new(ISSUER, AUDIENCE, KID, &other_key).expect("issuer");
    let token = other.sign(&claims(now())).expect("sign");

    assert!(matches!(
        issuer().verify(&token),
        Err(JwtError::InvalidToken)
    ));
    let _ = EncodingKey::from_ed_pem(other_pem.as_bytes());
}

#[test]
fn malformed_tokens_are_rejected() {
    let issuer = issuer();
    for token in ["", "one", "one.two", "one.two.three.four", "a.b.c"] {
        assert!(
            matches!(issuer.verify(token), Err(JwtError::InvalidToken)),
            "{token:?} must be rejected"
        );
    }
}
