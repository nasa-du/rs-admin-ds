//! Wire-format helpers shared by response DTOs (docs/specs/login-system.md
//! §7.1).

use chrono::{DateTime, SecondsFormat, Utc};

/// RFC3339 UTC with millisecond precision and a `Z` suffix.
pub(crate) fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}
