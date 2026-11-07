//! Mounted-file-first secret loading.
//!
//! Each secret has a logical key (for example `THROTTLE_KEY`) and an optional
//! `*_FILE` key. When the file key is present it wins, matching the deployment
//! contract in spec §8.7.

use crate::ConfigError;

/// Read a secret from `<file_key>` (a path) or fall back to `<key>`.
///
/// An empty environment variable counts as absent so optional secrets can be
/// left blank. An empty mounted file is invalid: it usually means a broken
/// deployment rather than an intentionally absent secret.
pub(crate) fn read_secret<F>(
    lookup: &F,
    key: &'static str,
    file_key: &'static str,
) -> Result<Option<String>, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(path) = lookup(file_key) {
        let path = path.trim();
        if path.is_empty() {
            return Err(ConfigError::UnreadableSecretFile(file_key));
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|_| ConfigError::UnreadableSecretFile(file_key))?;
        let contents = contents.trim().to_owned();
        if contents.is_empty() {
            return Err(ConfigError::Invalid(key));
        }
        return Ok(Some(contents));
    }

    Ok(lookup(key)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty()))
}

pub(crate) fn required_secret<F>(
    lookup: &F,
    key: &'static str,
    file_key: &'static str,
) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    read_secret(lookup, key, file_key)?.ok_or(ConfigError::Missing(key))
}

pub(crate) fn optional_secret<F>(
    lookup: &F,
    key: &'static str,
    file_key: &'static str,
) -> Result<Option<String>, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    read_secret(lookup, key, file_key)
}
