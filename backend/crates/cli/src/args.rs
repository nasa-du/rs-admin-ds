//! Command-line argument parsing.
//!
//! `--password` is deliberately unsupported so a password never appears in
//! shell history or the process list (ADR-0003 §6). Parsing errors never echo
//! an argument value, because that value could be a password typed by
//! mistake.

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
            None => return Err(ArgError::UnexpectedPositional),
        }
    }

    Ok(ParseOutcome::Parsed(RecoverArgs {
        email: email.ok_or(ArgError::MissingOption("--email"))?,
        password_stdin,
    }))
}

enum ParsedOption {
    Help,
    Password,
    PasswordStdin,
    Email(Option<String>),
    Username(Option<String>),
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
