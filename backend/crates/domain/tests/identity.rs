//! Username/email normalization and format rules (docs/specs/login-system.md
//! §4.3 and §7.5).

use rs_admin_domain::identity::{DisplayName, Email, IdentityError, Phone, Username};

#[test]
fn username_is_lowercased_without_trimming() {
    let username = Username::parse("Alice.01").expect("valid username");
    assert_eq!(username.as_str(), "alice.01");

    // §4.3 forbids whitespace in usernames; only lowercasing is normalized.
    assert_eq!(
        Username::parse("  alice  "),
        Err(IdentityError::UsernameFormat)
    );
}

#[test]
fn username_accepts_letters_digits_dots_underscores_and_hyphens() {
    for input in [
        "abc",
        "a1b",
        "alice.01",
        "alice_smith",
        "alice-smith",
        "9lives",
    ] {
        assert!(Username::parse(input).is_ok(), "{input} must be accepted");
    }
}

#[test]
fn username_length_is_between_three_and_thirty_two() {
    assert_eq!(Username::parse("ab"), Err(IdentityError::UsernameLength));
    assert_eq!(Username::parse(""), Err(IdentityError::UsernameLength));
    assert!(Username::parse("abc").is_ok());
    assert!(Username::parse(&"a".repeat(32)).is_ok());
    assert_eq!(
        Username::parse(&"a".repeat(33)),
        Err(IdentityError::UsernameLength)
    );
}

#[test]
fn username_format_is_ascii_starting_with_a_letter_or_digit() {
    for input in [
        "-alice",
        ".alice",
        "_alice",
        "alice!",
        "alice smith",
        "alice@example.com",
        "álice",
        "Alice Smith",
    ] {
        assert_eq!(
            Username::parse(input),
            Err(IdentityError::UsernameFormat),
            "{input} must be rejected"
        );
    }
}

#[test]
fn reserved_usernames_are_rejected_case_insensitively() {
    for input in [
        "admin",
        "Admin",
        "ROOT",
        "system",
        "super_admin",
        "administrator",
    ] {
        assert_eq!(
            Username::parse(input),
            Err(IdentityError::UsernameReserved),
            "{input} must be reserved"
        );
    }
}

#[test]
fn email_is_trimmed_nfc_normalized_and_lowercased() {
    let email = Email::parse("  Cafe\u{301}.Owner@Example.COM  ").expect("valid email");

    assert_eq!(email.as_str(), "café.owner@example.com");
}

#[test]
fn email_length_is_at_most_254_characters() {
    let local = "a".repeat(64);
    let domain = format!("{}.com", "b".repeat(60));
    let valid = format!("{local}@{domain}");
    assert!(valid.chars().count() <= 254);
    assert!(Email::parse(&valid).is_ok());

    let local = "a".repeat(64);
    let domain = format!("{}.{}.{}", "b".repeat(63), "b".repeat(63), "c".repeat(62));
    let too_long = format!("{local}@{domain}");
    assert_eq!(too_long.chars().count(), 255);
    assert_eq!(Email::parse(&too_long), Err(IdentityError::EmailTooLong));
}

#[test]
fn email_format_is_validated() {
    for input in [
        "",
        "no-at-sign",
        "a@b",
        "@example.com",
        "alice@",
        "alice@@example.com",
        "alice smith@example.com",
        ".alice@example.com",
        "alice.@example.com",
        "alice..smith@example.com",
        "alice@-example.com",
        "alice@example-.com",
        "alice@example..com",
    ] {
        assert_eq!(
            Email::parse(input),
            Err(IdentityError::EmailFormat),
            "{input} must be rejected"
        );
    }
}

#[test]
fn email_local_part_and_label_limits_are_enforced() {
    let long_local = format!("{}@example.com", "a".repeat(65));
    assert_eq!(Email::parse(&long_local), Err(IdentityError::EmailFormat));

    let long_label = format!("alice@{}.com", "b".repeat(64));
    assert_eq!(Email::parse(&long_label), Err(IdentityError::EmailFormat));
}

#[test]
fn display_name_counts_unicode_characters_and_defaults_to_the_username() {
    let username = Username::parse("alice").expect("valid username");
    let name = DisplayName::from_username(&username);
    assert_eq!(name.as_str(), "alice");

    let chinese = "管理员".repeat(21); // 63 characters
    assert!(DisplayName::parse(&chinese).is_ok());
    assert_eq!(
        DisplayName::parse(&"名".repeat(65)),
        Err(IdentityError::DisplayNameLength)
    );
    assert_eq!(
        DisplayName::parse(""),
        Err(IdentityError::DisplayNameLength)
    );
}

#[test]
fn phone_is_trimmed_and_limited_to_thirty_two_characters() {
    let phone = Phone::parse("  +86 138 0000 0000  ").expect("valid phone");
    assert_eq!(phone.as_str(), "+86 138 0000 0000");

    assert!(Phone::parse(&"1".repeat(32)).is_ok());
    assert_eq!(
        Phone::parse(&"1".repeat(33)),
        Err(IdentityError::PhoneLength)
    );
    assert_eq!(Phone::parse("   "), Err(IdentityError::PhoneLength));
}
