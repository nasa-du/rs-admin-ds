//! User identity value objects and their normalization rules
//! (docs/specs/login-system.md §4.3 and §7.5).
//!
//! Normalization happens once, before persistence: usernames are
//! ASCII-lowercased (surrounding whitespace is a format error, not silently
//! trimmed), emails are trimmed, NFC-normalized and lowercased. The database
//! repeats these checks as a backstop.

use std::fmt;
use unicode_normalization::UnicodeNormalization;

pub const USERNAME_MIN_CHARS: usize = 3;
pub const USERNAME_MAX_CHARS: usize = 32;
pub const EMAIL_MAX_CHARS: usize = 254;
pub const EMAIL_LOCAL_MAX_CHARS: usize = 64;
pub const EMAIL_DOMAIN_MAX_CHARS: usize = 253;
pub const EMAIL_LABEL_MAX_CHARS: usize = 63;
pub const DISPLAY_NAME_MAX_CHARS: usize = 64;
pub const PHONE_MAX_CHARS: usize = 32;

/// Names that would be confused with privileged accounts.
pub const RESERVED_USERNAMES: &[&str] =
    &["admin", "administrator", "root", "super_admin", "system"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("username must be between 3 and 32 characters")]
    UsernameLength,
    #[error("username must match ^[a-z0-9][a-z0-9._-]*$")]
    UsernameFormat,
    #[error("username is reserved")]
    UsernameReserved,
    #[error("email must be at most 254 characters")]
    EmailTooLong,
    #[error("email format is invalid")]
    EmailFormat,
    #[error("display name must be between 1 and 64 characters")]
    DisplayNameLength,
    #[error("phone must be between 1 and 32 characters")]
    PhoneLength,
}

/// A normalized, validated login username.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Username(String);

impl Username {
    pub fn parse(input: &str) -> Result<Self, IdentityError> {
        let normalized = input.to_ascii_lowercase();
        let length = normalized.chars().count();

        if !(USERNAME_MIN_CHARS..=USERNAME_MAX_CHARS).contains(&length) {
            return Err(IdentityError::UsernameLength);
        }
        if !is_valid_username(&normalized) {
            return Err(IdentityError::UsernameFormat);
        }
        if RESERVED_USERNAMES.contains(&normalized.as_str()) {
            return Err(IdentityError::UsernameReserved);
        }

        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_username(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    chars.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | '-')
    })
}

/// A normalized, validated login email address.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Email(String);

impl Email {
    pub fn parse(input: &str) -> Result<Self, IdentityError> {
        let normalized: String = input.trim().nfc().collect::<String>().to_lowercase();

        if normalized.chars().count() > EMAIL_MAX_CHARS {
            return Err(IdentityError::EmailTooLong);
        }
        if !is_valid_email(&normalized) {
            return Err(IdentityError::EmailFormat);
        }

        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_email(value: &str) -> bool {
    if value
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return false;
    }

    let mut parts = value.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };

    let local_length = local.chars().count();
    if local_length == 0 || local_length > EMAIL_LOCAL_MAX_CHARS {
        return false;
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return false;
    }
    if !local.chars().all(is_local_part_character) {
        return false;
    }

    let domain_length = domain.chars().count();
    if domain_length == 0 || domain_length > EMAIL_DOMAIN_MAX_CHARS {
        return false;
    }
    let labels: Vec<_> = domain.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    labels.iter().all(|label| {
        let length = label.chars().count();
        (1..=EMAIL_LABEL_MAX_CHARS).contains(&length)
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|character| character.is_alphanumeric() || character == '-')
    })
}

fn is_local_part_character(character: char) -> bool {
    character.is_alphanumeric()
        || matches!(
            character,
            '.' | '!'
                | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '/'
                | '='
                | '?'
                | '^'
                | '_'
                | '`'
                | '{'
                | '|'
                | '}'
                | '~'
        )
}

/// A display name. It is not a login identifier and need not be unique.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DisplayName(String);

impl DisplayName {
    pub fn parse(input: &str) -> Result<Self, IdentityError> {
        let length = input.chars().count();
        if !(1..=DISPLAY_NAME_MAX_CHARS).contains(&length) {
            return Err(IdentityError::DisplayNameLength);
        }
        Ok(Self(input.to_owned()))
    }

    /// The default display name: the user's username.
    pub fn from_username(username: &Username) -> Self {
        Self(username.0.clone())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An optional, admin-maintained phone number.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Phone(String);

impl Phone {
    pub fn parse(input: &str) -> Result<Self, IdentityError> {
        let normalized = input.trim();
        let length = normalized.chars().count();
        if !(1..=PHONE_MAX_CHARS).contains(&length) {
            return Err(IdentityError::PhoneLength);
        }
        Ok(Self(normalized.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! string_value_object_traits {
    ($( $type:ident ),+) => {
        $(
            impl AsRef<str> for $type {
                fn as_ref(&self) -> &str {
                    self.as_str()
                }
            }

            impl fmt::Display for $type {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str(self.as_str())
                }
            }
        )+
    };
}

string_value_object_traits!(Username, Email, DisplayName, Phone);
