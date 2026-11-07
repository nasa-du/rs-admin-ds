//! Password hashing and policy (docs/specs/login-system.md §8.1–§8.2).

use argon2::password_hash::PasswordHasher as _;
use argon2::{Algorithm, Argon2, Params, Version};
use rs_admin_security::{PasswordError, PasswordPolicy, PasswordService, PolicyViolation};

const CURRENT_PREFIX: &str = "$argon2id$v=19$m=65536,t=3,p=4$";

fn service() -> PasswordService {
    PasswordService::new(1).expect("password service")
}

#[tokio::test]
async fn hashes_use_argon2id_phc_with_the_policy_parameters() {
    let hash = service()
        .hash("correct horse battery staple")
        .await
        .expect("hash");

    assert!(
        hash.starts_with(CURRENT_PREFIX),
        "unexpected PHC prefix: {hash}"
    );
    // Salt is random per hash; the same password must not produce the same PHC.
    let other = service()
        .hash("correct horse battery staple")
        .await
        .expect("hash");
    assert_ne!(hash, other);
}

#[tokio::test]
async fn verification_accepts_only_the_matching_password() {
    let service = service();
    let hash = service
        .hash("correct horse battery staple")
        .await
        .expect("hash");

    let good = service
        .verify("correct horse battery staple", &hash)
        .await
        .expect("verify");
    assert!(good.valid);
    assert!(!good.needs_rehash);
    assert!(good.replacement_hash.is_none());

    let bad = service
        .verify("wrong password", &hash)
        .await
        .expect("verify");
    assert!(!bad.valid);
    assert!(!bad.needs_rehash);
    assert!(bad.replacement_hash.is_none());
}

#[tokio::test]
async fn hashes_below_the_current_parameters_are_replaced_after_verification() {
    // Independent weak PHC created with the same algorithm but lower cost.
    let weak_params = Params::new(19 * 1024, 2, 1, Some(32)).expect("params");
    let weak = Argon2::new(Algorithm::Argon2id, Version::V0x13, weak_params)
        .hash_password(b"correct horse battery staple")
        .expect("weak hash")
        .to_string();

    let outcome = service()
        .verify("correct horse battery staple", &weak)
        .await
        .expect("verify");

    assert!(outcome.valid);
    assert!(outcome.needs_rehash);
    let replacement = outcome.replacement_hash.expect("replacement hash");
    assert!(replacement.starts_with(CURRENT_PREFIX));
    assert!(
        service()
            .verify("correct horse battery staple", &replacement)
            .await
            .expect("verify replacement")
            .valid
    );
}

#[tokio::test]
async fn passwords_are_nfc_normalized_before_hashing_and_verification() {
    let service = service();
    let composed = "pässwörd-pässwörd";
    let decomposed = "pa\u{0308}sswo\u{0308}rd-pa\u{0308}sswo\u{0308}rd";

    let hash = service.hash(composed).await.expect("hash");
    assert!(
        service
            .verify(decomposed, &hash)
            .await
            .expect("verify")
            .valid
    );

    let hash = service.hash(decomposed).await.expect("hash");
    assert!(service.verify(composed, &hash).await.expect("verify").valid);
}

#[tokio::test]
async fn a_corrupt_stored_hash_is_a_failed_verification_not_an_error() {
    let outcome = service()
        .verify("anything", "not-a-phc-string")
        .await
        .expect("verify");
    assert!(!outcome.valid);
    assert!(!outcome.needs_rehash);
}

#[test]
fn password_policy_accepts_a_long_non_blocked_value() {
    let policy = PasswordPolicy::new();
    // A deterministic, obviously non-blocklisted and non-identifier value.
    let password = "Qz7!vLm2#Pw9$Rt4&Xc6";
    assert!(
        policy
            .validate(password, &["alice", "alice@example.com"])
            .is_ok()
    );
}

#[test]
fn password_policy_counts_unicode_code_points_without_trimming() {
    let policy = PasswordPolicy::new();

    assert_eq!(
        policy.validate("short", &[]),
        Err(PolicyViolation::TooShort)
    );
    assert_eq!(
        policy.validate(&"密".repeat(14), &[]),
        Err(PolicyViolation::TooShort)
    );
    // 15 code points pass the length rule (they are not on the blocklist).
    assert!(policy.validate(&"密".repeat(15), &[]).is_ok());
    assert_eq!(
        policy.validate(&"a".repeat(129), &[]),
        Err(PolicyViolation::TooLong)
    );

    // Surrounding whitespace is part of the password, never trimmed.
    let padded = format!(" {} ", "x".repeat(15));
    assert!(policy.validate(&padded, &[]).is_ok());
}

#[test]
fn password_policy_rejects_blocklisted_passwords_case_insensitively() {
    let policy = PasswordPolicy::new();

    assert_eq!(
        policy.validate("1q2w3e4r5t6y7u8i", &[]),
        Err(PolicyViolation::Blocklisted)
    );
    assert_eq!(
        policy.validate("1Q2W3E4R5T6Y7U8I", &[]),
        Err(PolicyViolation::Blocklisted)
    );
}

#[test]
fn password_policy_compares_nfc_normalized_identifiers() {
    let policy = PasswordPolicy::new();
    // "é" composed versus decomposed must be treated as the same value.
    let password = "pässwörd-pässwörd";
    assert_eq!(
        policy.validate(
            password,
            &["pa\u{0308}sswo\u{0308}rd-pa\u{0308}sswo\u{0308}rd"]
        ),
        Err(PolicyViolation::EqualsIdentifier)
    );
    assert_eq!(
        policy.validate(password, &["PÄSSWÖRD-PÄSSWÖRD"]),
        Err(PolicyViolation::EqualsIdentifier)
    );
}

#[test]
fn policy_violation_reasons_are_stable() {
    assert_eq!(PolicyViolation::TooShort.as_reason(), "too_short");
    assert_eq!(PolicyViolation::TooLong.as_reason(), "too_long");
    assert_eq!(PolicyViolation::Blocklisted.as_reason(), "blocklisted");
    assert_eq!(
        PolicyViolation::EqualsIdentifier.as_reason(),
        "equals_identifier"
    );
}

#[test]
fn password_service_rejects_zero_concurrency() {
    assert!(matches!(
        PasswordService::new(0),
        Err(PasswordError::InvalidConfiguration)
    ));
}
