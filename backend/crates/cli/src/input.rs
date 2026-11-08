//! Password and confirmation input.
//!
//! Passwords are wrapped in [`SecretString`] as soon as they are read, are
//! never echoed, trimmed or included in an error, and are exposed only to the
//! policy check, the hasher and the one-time success output. The prompt is
//! read from the terminal; `--password-stdin` exists for automation and reads
//! exactly one line.

use secrecy::SecretString;
use std::io::{self, BufRead, Read};

/// Upper bound for one line read from stdin. The password policy rejects
/// anything above 512 bytes; the cap prevents an unbounded allocation from a
/// hostile or broken pipe.
const MAX_INPUT_BYTES: u64 = 1024;

/// Input failures. Messages never contain the value that was read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error("could not read from stdin")]
    Stdin,
    #[error("could not read the temporary password from the terminal; use --password-stdin")]
    InteractiveRead,
    #[error("the temporary password must not be empty")]
    Empty,
    #[error("the temporary passwords do not match")]
    Mismatch,
    #[error("recovery requires an interactive terminal for confirmation")]
    ConfirmationRequiresTerminal,
    #[error("recovery was not confirmed")]
    ConfirmationMismatch,
}

/// Read the temporary password. With `from_stdin` the first line of stdin is
/// used (`--password-stdin`); otherwise the operator is prompted twice on the
/// terminal without echo and the two entries must match.
pub fn read_password(from_stdin: bool) -> Result<SecretString, InputError> {
    if from_stdin {
        let password = strip_line_ending(read_stdin_line()?);
        if password.is_empty() {
            return Err(InputError::Empty);
        }
        return Ok(SecretString::from(password));
    }

    let password = rpassword::prompt_password("Temporary password: ")
        .map_err(|_| InputError::InteractiveRead)?;
    if password.is_empty() {
        return Err(InputError::Empty);
    }
    let confirmation = rpassword::prompt_password("Repeat temporary password: ")
        .map_err(|_| InputError::InteractiveRead)?;
    if password != confirmation {
        return Err(InputError::Mismatch);
    }
    Ok(SecretString::from(password))
}

/// Ask the operator to type the target email before a break-glass recovery.
/// The confirmation is deliberately read from the terminal, never from a pipe:
/// pressing Enter on the target email is the interactive check ADR-0003 §6
/// requires, even when the password arrives through `--password-stdin`. A
/// masked echo confirms that something was typed without printing the value.
pub fn confirm_break_glass(email: &str) -> Result<(), InputError> {
    eprintln!("Break-glass recovery re-issues the temporary password, enables the account,");
    eprintln!("clears its login lock, revokes every session and invalidates every access token.");

    let config = rpassword::ConfigBuilder::new()
        .password_feedback_mask('*')
        .build();
    let confirmation =
        rpassword::prompt_password_with_config("Type the target email to confirm: ", config)
            .map_err(|_| InputError::ConfirmationRequiresTerminal)?;

    if confirmation.trim().eq_ignore_ascii_case(email) {
        Ok(())
    } else {
        Err(InputError::ConfirmationMismatch)
    }
}

fn read_stdin_line() -> Result<String, InputError> {
    let mut line = String::new();
    io::stdin()
        .lock()
        .take(MAX_INPUT_BYTES)
        .read_line(&mut line)
        .map_err(|_| InputError::Stdin)?;
    Ok(line)
}

fn strip_line_ending(mut line: String) -> String {
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::strip_line_ending;

    #[test]
    fn only_the_line_ending_is_removed() {
        assert_eq!(strip_line_ending(" secret \n".to_owned()), " secret ");
        assert_eq!(strip_line_ending("secret\r\n".to_owned()), "secret");
        assert_eq!(strip_line_ending("secret".to_owned()), "secret");
    }
}
