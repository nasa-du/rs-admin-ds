//! Operational authentication and audit settings (spec §8.7).

use crate::{ConfigError, parse_u32, parse_u64};
use std::time::Duration;

/// Minimum audit retention floors; configuration may raise but never lower
/// them.
pub const MIN_AUDIT_RETENTION_INFO_DAYS: u32 = 180;
pub const MIN_AUDIT_RETENTION_CRITICAL_DAYS: u32 = 365;

/// Authentication limits and windows.
#[derive(Clone, Debug)]
pub struct AuthSettings {
    pub password_hash_concurrency: u32,

    pub login_ip_failure_limit: u32,
    pub login_ip_window: Duration,
    pub login_ip_lock: Duration,

    pub login_global_alert_limit: u32,
    pub login_global_circuit_limit: u32,
    pub login_global_window: Duration,
    pub login_global_lock: Duration,

    pub account_failure_decay: Duration,
    pub temp_password_ttl: Duration,
    pub reauth_max_age: Duration,
}

impl AuthSettings {
    pub fn from_source<F>(lookup: &F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let password_hash_concurrency = positive_u32(
            lookup,
            "PASSWORD_HASH_CONCURRENCY",
            default_password_hash_concurrency(),
        )?;

        let login_ip_failure_limit = positive_u32(lookup, "LOGIN_IP_FAILURE_LIMIT", 20)?;
        let login_ip_window = positive_duration(lookup, "LOGIN_IP_WINDOW_SECONDS", 600)?;
        let login_ip_lock = positive_duration(lookup, "LOGIN_IP_LOCK_SECONDS", 900)?;

        let login_global_alert_limit = positive_u32(lookup, "LOGIN_GLOBAL_ALERT_LIMIT", 200)?;
        let login_global_circuit_limit = positive_u32(lookup, "LOGIN_GLOBAL_CIRCUIT_LIMIT", 1000)?;
        let login_global_window = positive_duration(lookup, "LOGIN_GLOBAL_WINDOW_SECONDS", 300)?;
        let login_global_lock = positive_duration(lookup, "LOGIN_GLOBAL_LOCK_SECONDS", 60)?;
        if login_global_alert_limit > login_global_circuit_limit {
            return Err(ConfigError::Invalid("LOGIN_GLOBAL_ALERT_LIMIT"));
        }

        let account_failure_decay =
            positive_duration_hours(lookup, "ACCOUNT_FAILURE_DECAY_HOURS", 24)?;
        let temp_password_ttl = positive_duration_hours(lookup, "TEMP_PASSWORD_TTL_HOURS", 72)?;
        let reauth_max_age = positive_duration(lookup, "REAUTH_MAX_AGE_SECONDS", 300)?;

        Ok(Self {
            password_hash_concurrency,
            login_ip_failure_limit,
            login_ip_window,
            login_ip_lock,
            login_global_alert_limit,
            login_global_circuit_limit,
            login_global_window,
            login_global_lock,
            account_failure_decay,
            temp_password_ttl,
            reauth_max_age,
        })
    }
}

/// Audit retention; values may not fall below the spec floors.
#[derive(Clone, Debug)]
pub struct AuditSettings {
    pub retention_info_days: u32,
    pub retention_critical_days: u32,
}

impl AuditSettings {
    pub fn from_source<F>(lookup: &F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let retention_info_days = positive_u32(lookup, "AUDIT_RETENTION_INFO_DAYS", 180)?;
        if retention_info_days < MIN_AUDIT_RETENTION_INFO_DAYS {
            return Err(ConfigError::Invalid("AUDIT_RETENTION_INFO_DAYS"));
        }

        let retention_critical_days = positive_u32(lookup, "AUDIT_RETENTION_CRITICAL_DAYS", 365)?;
        if retention_critical_days < MIN_AUDIT_RETENTION_CRITICAL_DAYS {
            return Err(ConfigError::Invalid("AUDIT_RETENTION_CRITICAL_DAYS"));
        }

        Ok(Self {
            retention_info_days,
            retention_critical_days,
        })
    }
}

/// `max(1, min(4, cores / p))` with `p = 4`, per spec §8.1.
fn default_password_hash_concurrency() -> u32 {
    let cores = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    u32::try_from(cores / 4).unwrap_or(4).clamp(1, 4)
}

fn positive_u32<F>(lookup: &F, key: &'static str, default: u32) -> Result<u32, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let value = parse_u32(lookup, key, default)?;
    if value == 0 {
        return Err(ConfigError::Invalid(key));
    }
    Ok(value)
}

fn positive_duration<F>(
    lookup: &F,
    key: &'static str,
    default_seconds: u64,
) -> Result<Duration, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let seconds = parse_u64(lookup, key, default_seconds)?;
    if seconds == 0 {
        return Err(ConfigError::Invalid(key));
    }
    Ok(Duration::from_secs(seconds))
}

fn positive_duration_hours<F>(
    lookup: &F,
    key: &'static str,
    default_hours: u64,
) -> Result<Duration, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let hours = parse_u64(lookup, key, default_hours)?;
    if hours == 0 {
        return Err(ConfigError::Invalid(key));
    }
    let seconds = hours.checked_mul(3600).ok_or(ConfigError::Invalid(key))?;
    Ok(Duration::from_secs(seconds))
}
