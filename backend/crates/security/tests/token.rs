//! Refresh-token and throttle-key helpers (docs/specs/login-system.md §5.2, §8.3).

use rs_admin_security::{
    HmacKey, REFRESH_TOKEN_BYTES, derive_refresh_token, generate_refresh_token, hash_refresh_token,
    identifier_hmac_hex,
};
use std::collections::HashSet;

#[test]
fn refresh_tokens_are_256_bit_url_safe_and_unique() {
    let mut seen = HashSet::new();
    for _ in 0..64 {
        let token = generate_refresh_token().expect("token");
        assert_eq!(token.len(), 43, "32 bytes base64url is 43 characters");
        assert!(
            token.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            ),
            "token must be URL-safe: {token}"
        );
        assert!(seen.insert(token), "tokens must not repeat");
    }
    assert_eq!(REFRESH_TOKEN_BYTES, 32);
}

#[test]
fn refresh_tokens_are_hashed_with_sha256() {
    let token = generate_refresh_token().expect("token");
    let first = hash_refresh_token(&token);
    let second = hash_refresh_token(&token);

    assert_eq!(first, second);
    assert_eq!(first.len(), 32);
    assert_ne!(first, hash_refresh_token("another token"));
    // The raw token must not appear in its hash.
    assert!(!String::from_utf8_lossy(&first).contains(&token));
}

#[test]
fn refresh_derivation_is_a_stable_keyed_chain() {
    let key = HmacKey::from_bytes(vec![1u8; 32]).expect("key");
    let other_key = HmacKey::from_bytes(vec![2u8; 32]).expect("key");

    let first = derive_refresh_token(&key, "initial-token", 1);
    assert_eq!(
        first,
        derive_refresh_token(&key, "initial-token", 1),
        "derivation must be deterministic for a retry"
    );
    assert_eq!(first.len(), 43, "HMAC-SHA256 base64url is 43 characters");
    assert!(
        first
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')),
        "derived tokens must be URL-safe: {first}"
    );

    assert_ne!(first, derive_refresh_token(&key, "initial-token", 2));
    assert_ne!(first, derive_refresh_token(&key, "other-token", 1));
    assert_ne!(first, derive_refresh_token(&other_key, "initial-token", 1));

    // A second hop derives from the first successor, not the initial token.
    let second = derive_refresh_token(&key, &first, 2);
    assert_ne!(second, first);
    assert_ne!(second, derive_refresh_token(&key, "initial-token", 2));
}

#[test]
fn identifier_hmac_is_keyed_and_stable() {
    let key = HmacKey::from_bytes(vec![7u8; 32]).expect("key");
    let other = HmacKey::from_bytes(vec![8u8; 32]).expect("key");

    let first = identifier_hmac_hex(&key, "alice@example.com");
    assert_eq!(first.len(), 64, "HMAC-SHA256 hex is 64 characters");
    assert_eq!(first, identifier_hmac_hex(&key, "alice@example.com"));
    assert_ne!(first, identifier_hmac_hex(&key, "bob@example.com"));
    assert_ne!(first, identifier_hmac_hex(&other, "alice@example.com"));
    assert!(!first.contains("alice"));
}
