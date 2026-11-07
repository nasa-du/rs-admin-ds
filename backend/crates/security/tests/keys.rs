//! Public behaviour of key material parsing and validation.
//!
//! These tests use generated keys; no fixture key material is committed.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
use rs_admin_security::{AuditKeyRing, HmacKey, JwtSigningKey, JwtVerificationKey};

const KEY_BYTES: &[u8] = b"0123456789abcdef0123456789abcdef";

fn private_pem() -> String {
    SigningKey::from_bytes(&[7u8; 32])
        .to_pkcs8_pem(LineEnding::LF)
        .expect("PKCS#8 encoding")
        .to_string()
}

fn public_pem() -> String {
    SigningKey::from_bytes(&[7u8; 32])
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .expect("SPKI encoding")
}

fn encoded_key(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn keyring_json(entries: &[(&str, &str)]) -> String {
    let body = entries
        .iter()
        .map(|(id, key)| format!("\"{id}\":\"{key}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

#[test]
fn hmac_keys_are_base64_and_at_least_32_bytes() {
    let key = HmacKey::from_base64(&encoded_key(KEY_BYTES)).expect("32-byte key");
    assert_eq!(key.expose(), KEY_BYTES);
    assert_eq!(key.len(), 32);

    HmacKey::from_base64(&encoded_key(&[9u8; 31])).expect_err("31-byte key must be rejected");
    HmacKey::from_base64("not base64!").expect_err("invalid base64 must be rejected");
}

#[test]
fn hmac_key_debug_output_redacts_material() {
    let encoded = encoded_key(KEY_BYTES);
    let rendered = format!("{:?}", HmacKey::from_base64(&encoded).expect("key"));
    assert!(!rendered.contains(&encoded));
    assert!(!rendered.contains("0123456789abcdef"));
    assert!(rendered.contains("REDACTED"));
}

#[test]
fn audit_keyring_exposes_current_and_previous_keys() {
    let current = encoded_key(KEY_BYTES);
    let previous = encoded_key(&[8u8; 32]);
    let ring = AuditKeyRing::from_json(
        &keyring_json(&[("2026-11", &current), ("2026-10", &previous)]),
        "2026-11",
    )
    .expect("valid keyring");

    assert_eq!(ring.current_id(), "2026-11");
    assert_eq!(ring.current().expose(), KEY_BYTES);
    assert_eq!(
        ring.get("2026-10").expect("previous key").expose(),
        &[8u8; 32]
    );
    assert!(ring.get("missing").is_none());
    assert_eq!(ring.len(), 2);
}

#[test]
fn audit_keyring_must_contain_the_current_key_id() {
    let json = keyring_json(&[("2026-11", &encoded_key(KEY_BYTES))]);
    let error = AuditKeyRing::from_json(&json, "2026-12").expect_err("missing current key");
    assert!(error.to_string().contains("current key id"));
}

#[test]
fn audit_keyring_rejects_malformed_entries_without_echoing_them() {
    let secret_marker = "super-secret-ring-material";
    for json in [
        "not json".to_string(),
        "{}".to_string(),
        keyring_json(&[("2026-11", "not base64!")]),
        keyring_json(&[("2026-11", &encoded_key(&[1u8; 31]))]),
        keyring_json(&[("", &encoded_key(KEY_BYTES))]),
        format!("{{\"2026-11\":\"{secret_marker}\"}}"),
    ] {
        let error = AuditKeyRing::from_json(&json, "2026-11").expect_err("invalid keyring");
        assert!(!error.to_string().contains(secret_marker));
    }
}

#[test]
fn audit_keyring_debug_output_redacts_material() {
    let encoded = encoded_key(KEY_BYTES);
    let ring = AuditKeyRing::from_json(&keyring_json(&[("2026-11", &encoded)]), "2026-11")
        .expect("valid keyring");
    let rendered = format!("{ring:?}");
    assert!(!rendered.contains(&encoded));
    assert!(rendered.contains("2026-11"));
}

#[test]
fn jwt_signing_key_accepts_pkcs8_ed25519_private_pem_only() {
    JwtSigningKey::from_pem(&private_pem()).expect("valid Ed25519 private key");

    let public = public_pem();
    JwtSigningKey::from_pem(&public).expect_err("public key is not a signing key");
    JwtSigningKey::from_pem("not a pem").expect_err("garbage must be rejected");
}

#[test]
fn jwt_verification_key_accepts_ed25519_public_pem_only() {
    JwtVerificationKey::from_public_pem(&public_pem()).expect("valid Ed25519 public key");

    let private = private_pem();
    JwtVerificationKey::from_public_pem(&private).expect_err("private key is not a public key");
    JwtVerificationKey::from_public_pem("not a pem").expect_err("garbage must be rejected");
}

#[test]
fn key_errors_never_echo_key_material() {
    let private = private_pem();
    let error = JwtSigningKey::from_pem(&public_pem()).expect_err("wrong key kind");
    assert!(!error.to_string().contains(&private));

    let marker = "secret-marker-value";
    let error = HmacKey::from_base64(marker).expect_err("invalid base64");
    assert!(!error.to_string().contains(marker));
}
