//! Command-line argument parsing.
//!
//! `--password` is deliberately unsupported so a password never appears in
//! shell history or the process list (ADR-0003 §6). Parsing errors never echo
//! an argument value, because that value could be a password typed by
//! mistake.

use crate::key_rotation::{KeyType, RotationReason};

/// Outcome of parsing a command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseOutcome<T> {
    Parsed(T),
    Help(&'static str),
}

/// A command-line usage error. Values are never included in the message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArgError {
    #[error("missing required option {0}")]
    MissingOption(&'static str),
    #[error("option {0} requires a value")]
    MissingValue(&'static str),
    #[error("unknown option {0}")]
    UnknownOption(String),
    #[error("option {0} does not take a value")]
    UnexpectedValue(&'static str),
    #[error(
        "the --password option is not supported; use the interactive prompt or --password-stdin"
    )]
    PasswordFlagForbidden,
    #[error("unexpected positional argument")]
    UnexpectedPositional,
    #[error("option {0} has an invalid value")]
    InvalidValue(&'static str),
}

pub const BOOTSTRAP_USAGE: &str = "\
bootstrap-super-admin - create or promote the first super administrator

Usage:
  bootstrap-super-admin --email <email> --username <username> [--password-stdin]

The temporary password is read twice from an interactive prompt without echo,
or from the first line of stdin with --password-stdin. The --password option is
not supported. The password must satisfy the password policy and is shown once
on success; the user must change it at first login.";

pub const RECOVER_USAGE: &str = "\
recover-super-admin - break-glass recovery for an existing super administrator

Usage:
  recover-super-admin --email <email> [--password-stdin]

The target must already hold the built-in super_admin role. Recovery re-issues
a temporary password, enables the account, clears its login lock, revokes every
session and invalidates every access token. The temporary password is read
twice from an interactive prompt without echo, or from the first line of stdin
with --password-stdin. Confirmation is always read from a terminal: type the
target email when prompted.";

pub const RECORD_KEY_ROTATION_USAGE: &str = "\
record-key-rotation - append the critical crypto_key_rotated audit event

Usage:
  record-key-rotation --key-type <jwt|refresh_derivation|throttle|audit_chain> \
    --old-kid <id> --new-kid <id> --reason <scheduled|compromise>

The command writes one critical crypto_key_rotated event with the key type,
old/new key ids and reason. It never reads or prints key material. Run it after
the new key is active and the rotation's verification has passed.";

/// Parsed `bootstrap-super-admin` arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapArgs {
    pub email: String,
    pub username: String,
    pub password_stdin: bool,
}

/// Parsed `recover-super-admin` arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverArgs {
    pub email: String,
    pub password_stdin: bool,
}

/// Parsed `record-key-rotation` arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRotationArgs {
    pub key_type: KeyType,
    pub old_kid: String,
    pub new_kid: String,
    pub reason: RotationReason,
}

pub fn parse_bootstrap(
    arguments: impl IntoIterator<Item = String>,
) -> Result<ParseOutcome<BootstrapArgs>, ArgError> {
    let mut email = None;
    let mut username = None;
    let mut password_stdin = false;

    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match option_of(argument)? {
            Some(ParsedOption::Help) => return Ok(ParseOutcome::Help(BOOTSTRAP_USAGE)),
            Some(ParsedOption::Password) => return Err(ArgError::PasswordFlagForbidden),
            Some(ParsedOption::PasswordStdin) => password_stdin = true,
            Some(ParsedOption::Email(inline)) => {
                email = Some(inline_value_or_next(inline, &mut arguments, "--email")?);
            }
            Some(ParsedOption::Username(inline)) => {
                username = Some(inline_value_or_next(inline, &mut arguments, "--username")?);
            }
            Some(_) => return Err(ArgError::UnexpectedPositional),
            None => return Err(ArgError::UnexpectedPositional),
        }
    }

    Ok(ParseOutcome::Parsed(BootstrapArgs {
        email: email.ok_or(ArgError::MissingOption("--email"))?,
        username: username.ok_or(ArgError::MissingOption("--username"))?,
        password_stdin,
    }))
}

pub fn parse_recover(
    arguments: impl IntoIterator<Item = String>,
) -> Result<ParseOutcome<RecoverArgs>, ArgError> {
    let mut email = None;
    let mut password_stdin = false;

    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match option_of(argument)? {
            Some(ParsedOption::Help) => return Ok(ParseOutcome::Help(RECOVER_USAGE)),
            Some(ParsedOption::Password) => return Err(ArgError::PasswordFlagForbidden),
            Some(ParsedOption::PasswordStdin) => password_stdin = true,
            Some(ParsedOption::Email(inline)) => {
                email = Some(inline_value_or_next(inline, &mut arguments, "--email")?);
            }
            Some(ParsedOption::Username(_)) => {
                return Err(ArgError::UnknownOption("--username".to_owned()));
            }
            Some(_) => return Err(ArgError::UnexpectedPositional),
            None => return Err(ArgError::UnexpectedPositional),
        }
    }

    Ok(ParseOutcome::Parsed(RecoverArgs {
        email: email.ok_or(ArgError::MissingOption("--email"))?,
        password_stdin,
    }))
}

pub fn parse_record_key_rotation(
    arguments: impl IntoIterator<Item = String>,
) -> Result<ParseOutcome<KeyRotationArgs>, ArgError> {
    let mut key_type = None;
    let mut old_kid = None;
    let mut new_kid = None;
    let mut reason = None;

    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match option_of(argument)? {
            Some(ParsedOption::Help) => return Ok(ParseOutcome::Help(RECORD_KEY_ROTATION_USAGE)),
            Some(ParsedOption::Password) => return Err(ArgError::PasswordFlagForbidden),
            Some(ParsedOption::KeyType(inline)) => {
                let value = inline_value_or_next(inline, &mut arguments, "--key-type")?;
                key_type =
                    Some(KeyType::parse(&value).ok_or(ArgError::InvalidValue("--key-type"))?);
            }
            Some(ParsedOption::OldKid(inline)) => {
                old_kid = Some(inline_value_or_next(inline, &mut arguments, "--old-kid")?);
            }
            Some(ParsedOption::NewKid(inline)) => {
                new_kid = Some(inline_value_or_next(inline, &mut arguments, "--new-kid")?);
            }
            Some(ParsedOption::Reason(inline)) => {
                let value = inline_value_or_next(inline, &mut arguments, "--reason")?;
                reason =
                    Some(RotationReason::parse(&value).ok_or(ArgError::InvalidValue("--reason"))?);
            }
            Some(_) => return Err(ArgError::UnexpectedPositional),
            None => return Err(ArgError::UnexpectedPositional),
        }
    }

    Ok(ParseOutcome::Parsed(KeyRotationArgs {
        key_type: key_type.ok_or(ArgError::MissingOption("--key-type"))?,
        old_kid: old_kid.ok_or(ArgError::MissingOption("--old-kid"))?,
        new_kid: new_kid.ok_or(ArgError::MissingOption("--new-kid"))?,
        reason: reason.ok_or(ArgError::MissingOption("--reason"))?,
    }))
}

enum ParsedOption {
    Help,
    Password,
    PasswordStdin,
    Email(Option<String>),
    Username(Option<String>),
    KeyType(Option<String>),
    OldKid(Option<String>),
    NewKid(Option<String>),
    Reason(Option<String>),
}

/// Classify one argument. `None` means a bare positional argument, whose text
/// is intentionally not echoed.
fn option_of(argument: String) -> Result<Option<ParsedOption>, ArgError> {
    if !argument.starts_with('-') || argument == "-" {
        return Ok(None);
    }

    // Split `--option=value`; only the option name may appear in an error.
    let (name, inline) = match argument.split_once('=') {
        Some((name, value)) => (name, Some(value.to_owned())),
        None => (argument.as_str(), None),
    };

    let parsed = match (name, inline) {
        ("-h" | "--help", None) => ParsedOption::Help,
        ("-h" | "--help", Some(_)) => return Err(ArgError::UnexpectedValue("--help")),
        ("--password", _) => ParsedOption::Password,
        ("--password-stdin", None) => ParsedOption::PasswordStdin,
        ("--password-stdin", Some(_)) => {
            return Err(ArgError::UnexpectedValue("--password-stdin"));
        }
        ("--email", inline) => ParsedOption::Email(inline),
        ("--username", inline) => ParsedOption::Username(inline),
        ("--key-type", inline) => ParsedOption::KeyType(inline),
        ("--old-kid", inline) => ParsedOption::OldKid(inline),
        ("--new-kid", inline) => ParsedOption::NewKid(inline),
        ("--reason", inline) => ParsedOption::Reason(inline),
        _ => return Err(ArgError::UnknownOption(name.to_owned())),
    };

    Ok(Some(parsed))
}

/// Inline value from `--option=value`, or the next argument. A following
/// option-looking token is treated as a missing value instead of being
/// swallowed.
fn inline_value_or_next(
    inline: Option<String>,
    arguments: &mut impl Iterator<Item = String>,
    name: &'static str,
) -> Result<String, ArgError> {
    let value = match inline {
        Some(value) => value,
        None => arguments
            .next()
            .filter(|next| !next.starts_with('-'))
            .ok_or(ArgError::MissingValue(name))?,
    };

    if value.is_empty() {
        return Err(ArgError::MissingValue(name));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bootstrap(arguments: &[&str]) -> Result<ParseOutcome<BootstrapArgs>, ArgError> {
        parse_bootstrap(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    fn recover(arguments: &[&str]) -> Result<ParseOutcome<RecoverArgs>, ArgError> {
        parse_recover(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    fn key_rotation(arguments: &[&str]) -> Result<ParseOutcome<KeyRotationArgs>, ArgError> {
        parse_record_key_rotation(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    #[test]
    fn bootstrap_accepts_separate_and_inline_values() {
        assert_eq!(
            bootstrap(&["--email", "ops@example.com", "--username", "ops.admin"]),
            Ok(ParseOutcome::Parsed(BootstrapArgs {
                email: "ops@example.com".to_owned(),
                username: "ops.admin".to_owned(),
                password_stdin: false,
            }))
        );
        assert_eq!(
            bootstrap(&[
                "--email=ops@example.com",
                "--username=ops.admin",
                "--password-stdin"
            ]),
            Ok(ParseOutcome::Parsed(BootstrapArgs {
                email: "ops@example.com".to_owned(),
                username: "ops.admin".to_owned(),
                password_stdin: true,
            }))
        );
    }

    #[test]
    fn recover_requires_only_the_email() {
        assert_eq!(
            recover(&["--email", "ops@example.com"]),
            Ok(ParseOutcome::Parsed(RecoverArgs {
                email: "ops@example.com".to_owned(),
                password_stdin: false,
            }))
        );
        assert_eq!(
            recover(&["--email", "ops@example.com", "--username", "ops.admin"]),
            Err(ArgError::UnknownOption("--username".to_owned()))
        );
    }

    #[test]
    fn key_rotation_accepts_separate_and_inline_values() {
        assert_eq!(
            key_rotation(&[
                "--key-type",
                "jwt",
                "--old-kid",
                "prod-2026-11",
                "--new-kid",
                "prod-2027-01",
                "--reason",
                "scheduled",
            ]),
            Ok(ParseOutcome::Parsed(KeyRotationArgs {
                key_type: KeyType::Jwt,
                old_kid: "prod-2026-11".to_owned(),
                new_kid: "prod-2027-01".to_owned(),
                reason: RotationReason::Scheduled,
            }))
        );
        assert_eq!(
            key_rotation(&[
                "--key-type=audit_chain",
                "--old-kid=audit-2026-11",
                "--new-kid=audit-2027-01",
                "--reason=compromise",
            ]),
            Ok(ParseOutcome::Parsed(KeyRotationArgs {
                key_type: KeyType::AuditChain,
                old_kid: "audit-2026-11".to_owned(),
                new_kid: "audit-2027-01".to_owned(),
                reason: RotationReason::Compromise,
            }))
        );
    }

    #[test]
    fn key_rotation_rejects_unknown_values_without_echoing_them() {
        let error = key_rotation(&[
            "--key-type",
            "hmac",
            "--old-kid",
            "a",
            "--new-kid",
            "b",
            "--reason",
            "scheduled",
        ])
        .expect_err("key type");
        assert_eq!(error, ArgError::InvalidValue("--key-type"));
        assert!(!error.to_string().contains("hmac"));

        let error = key_rotation(&[
            "--key-type",
            "jwt",
            "--old-kid",
            "a",
            "--new-kid",
            "b",
            "--reason",
            "because",
        ])
        .expect_err("reason");
        assert_eq!(error, ArgError::InvalidValue("--reason"));
        assert!(!error.to_string().contains("because"));
    }

    #[test]
    fn key_rotation_requires_every_option() {
        assert_eq!(
            key_rotation(&[]),
            Err(ArgError::MissingOption("--key-type"))
        );
        assert_eq!(
            key_rotation(&["--key-type", "jwt"]),
            Err(ArgError::MissingOption("--old-kid"))
        );
        assert_eq!(
            key_rotation(&["--key-type", "jwt", "--old-kid", "a"]),
            Err(ArgError::MissingOption("--new-kid"))
        );
        assert_eq!(
            key_rotation(&["--key-type", "jwt", "--old-kid", "a", "--new-kid", "b"]),
            Err(ArgError::MissingOption("--reason"))
        );
    }

    #[test]
    fn help_is_recognized_without_other_arguments() {
        assert!(matches!(
            bootstrap(&["--help"]),
            Ok(ParseOutcome::Help(BOOTSTRAP_USAGE))
        ));
        assert!(matches!(
            recover(&["-h"]),
            Ok(ParseOutcome::Help(RECOVER_USAGE))
        ));
    }

    #[test]
    fn required_options_are_enforced() {
        assert_eq!(
            bootstrap(&["--email", "ops@example.com"]),
            Err(ArgError::MissingOption("--username"))
        );
        assert_eq!(recover(&[]), Err(ArgError::MissingOption("--email")));
        assert_eq!(
            bootstrap(&["--email", "--username", "ops.admin"]),
            Err(ArgError::MissingValue("--email"))
        );
    }

    #[test]
    fn the_password_flag_is_rejected_without_echoing_its_value() {
        let error = bootstrap(&[
            "--email",
            "ops@example.com",
            "--username",
            "ops.admin",
            "--password=hunter2",
        ])
        .expect_err("password flag");
        assert_eq!(error, ArgError::PasswordFlagForbidden);
        assert!(!error.to_string().contains("hunter2"));
    }

    #[test]
    fn flags_reject_inline_values() {
        assert_eq!(
            bootstrap(&["--password-stdin=yes"]),
            Err(ArgError::UnexpectedValue("--password-stdin"))
        );
        assert_eq!(
            recover(&["--help=yes"]),
            Err(ArgError::UnexpectedValue("--help"))
        );
    }

    #[test]
    fn unknown_and_positional_arguments_do_not_echo_their_value() {
        let error = bootstrap(&["--token", "secret-value"]).expect_err("unknown");
        assert_eq!(error, ArgError::UnknownOption("--token".to_owned()));
        assert!(!error.to_string().contains("secret-value"));

        let error = bootstrap(&["hunter2"]).expect_err("positional");
        assert_eq!(error, ArgError::UnexpectedPositional);
        assert!(!error.to_string().contains("hunter2"));
    }
}
