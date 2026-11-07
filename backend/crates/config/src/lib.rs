//! Configuration loading and startup validation.
//!
//! Secrets are read from `*_FILE` first and fall back to the corresponding
//! environment variable. Error messages name the key but never echo values.

mod keys;
mod secret;
mod settings;

pub use keys::{AuditChainConfig, JwtConfig, RotationKeys, SecurityConfig};
pub use settings::{AuditSettings, AuthSettings};

use ipnet::IpNet;
use secrecy::{ExposeSecret, SecretString};
use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;
use url::Url;

/// Runtime environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEnv {
    Development,
    Test,
    Production,
}

impl AppEnv {
    pub fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

impl FromStr for AppEnv {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "development" => Ok(Self::Development),
            "test" => Ok(Self::Test),
            "production" => Ok(Self::Production),
            _ => Err(ConfigError::Invalid("APP_ENV")),
        }
    }
}

/// Errors produced while loading configuration. Values are never included.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required configuration: {0}")]
    Missing(&'static str),
    #[error("invalid configuration: {0}")]
    Invalid(&'static str),
    #[error("failed to read secret file configured by {0}")]
    UnreadableSecretFile(&'static str),
}

/// Database connection and pool settings.
#[derive(Clone)]
pub struct DatabaseConfig {
    pub url: SecretString,
    pub pool: PoolConfig,
}

impl DatabaseConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    pub fn from_source<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let app_env = parse_app_env(&lookup)?;
        let url = secret::required_secret(&lookup, "DATABASE_URL", "DATABASE_URL_FILE")?;
        validate_database_url(&url, app_env)?;
        let pool = PoolConfig::from_source(&lookup)?;

        Ok(Self {
            url: SecretString::from(url),
            pool,
        })
    }

    /// Expose the connection string to the database layer only.
    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatabaseConfig")
            .field("url", &"[REDACTED]")
            .field("pool", &self.pool)
            .finish()
    }
}

/// Connection-pool settings; defaults follow the implementation spec.
#[derive(Clone, Debug)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
}

impl PoolConfig {
    fn from_source<F>(lookup: &F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let max_connections = parse_u32(lookup, "DATABASE_MAX_CONNECTIONS", 10)?;
        let min_connections = parse_u32(lookup, "DATABASE_MIN_CONNECTIONS", 1)?;
        let acquire_seconds = parse_u64(lookup, "DATABASE_ACQUIRE_TIMEOUT_SECONDS", 3)?;
        let idle_seconds = parse_u64(lookup, "DATABASE_IDLE_TIMEOUT_SECONDS", 600)?;
        let lifetime_seconds = parse_u64(lookup, "DATABASE_MAX_LIFETIME_SECONDS", 1800)?;

        if max_connections == 0 {
            return Err(ConfigError::Invalid("DATABASE_MAX_CONNECTIONS"));
        }
        if min_connections > max_connections {
            return Err(ConfigError::Invalid("DATABASE_MIN_CONNECTIONS"));
        }
        if acquire_seconds == 0 {
            return Err(ConfigError::Invalid("DATABASE_ACQUIRE_TIMEOUT_SECONDS"));
        }
        if idle_seconds == 0 {
            return Err(ConfigError::Invalid("DATABASE_IDLE_TIMEOUT_SECONDS"));
        }
        if lifetime_seconds == 0 {
            return Err(ConfigError::Invalid("DATABASE_MAX_LIFETIME_SECONDS"));
        }

        Ok(Self {
            max_connections,
            min_connections,
            acquire_timeout: Duration::from_secs(acquire_seconds),
            idle_timeout: Duration::from_secs(idle_seconds),
            max_lifetime: Duration::from_secs(lifetime_seconds),
        })
    }
}

/// Listener addresses.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub api_addr: SocketAddr,
    pub metrics_addr: SocketAddr,
}

/// Browser origins, canonicalized without a trailing slash.
#[derive(Clone, Debug)]
pub struct OriginConfig {
    pub public_api_origin: String,
    pub admin_origin: String,
    pub cors_allowed_origins: Vec<String>,
}

/// Fully validated server configuration.
#[derive(Clone, Debug)]
pub struct Config {
    pub app_env: AppEnv,
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub origins: OriginConfig,
    pub trusted_proxy_cidrs: Vec<IpNet>,
    pub security: SecurityConfig,
    pub auth: AuthSettings,
    pub audit: AuditSettings,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    pub fn from_source<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let app_env = parse_app_env(&lookup)?;
        let database = DatabaseConfig::from_source(&lookup)?;
        let server = ServerConfig {
            api_addr: parse_addr(&lookup, "API_ADDR", "0.0.0.0:8080")?,
            metrics_addr: parse_addr(&lookup, "METRICS_ADDR", "0.0.0.0:9100")?,
        };

        let public_api_origin = required(&lookup, "PUBLIC_API_ORIGIN")?;
        let public_api_origin = parse_origin(&public_api_origin, "PUBLIC_API_ORIGIN", app_env)?;
        let admin_origin = required(&lookup, "ADMIN_ORIGIN")?;
        let admin_origin = parse_origin(&admin_origin, "ADMIN_ORIGIN", app_env)?;
        let cors_allowed_origins = parse_origins(
            &required(&lookup, "CORS_ALLOWED_ORIGINS")?,
            "CORS_ALLOWED_ORIGINS",
            app_env,
        )?;
        let trusted_proxy_cidrs = parse_cidrs(&required(&lookup, "TRUSTED_PROXY_CIDRS")?)?;

        let security = SecurityConfig::from_source(&lookup)?;
        let auth = AuthSettings::from_source(&lookup)?;
        let audit = AuditSettings::from_source(&lookup)?;

        Ok(Self {
            app_env,
            database,
            server,
            origins: OriginConfig {
                public_api_origin,
                admin_origin,
                cors_allowed_origins,
            },
            trusted_proxy_cidrs,
            security,
            auth,
            audit,
        })
    }
}

fn parse_app_env<F>(lookup: &F) -> Result<AppEnv, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    AppEnv::from_str(&required(lookup, "APP_ENV")?)
}

pub(crate) fn required<F>(lookup: &F, key: &'static str) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(key)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::Missing(key))
}

fn validate_database_url(raw: &str, app_env: AppEnv) -> Result<(), ConfigError> {
    let url = Url::parse(raw).map_err(|_| ConfigError::Invalid("DATABASE_URL"))?;

    if !matches!(url.scheme(), "postgres" | "postgresql") || url.host_str().is_none() {
        return Err(ConfigError::Invalid("DATABASE_URL"));
    }

    if app_env.is_production() {
        let ssl_mode = url
            .query_pairs()
            .find(|(key, _)| key == "sslmode")
            .map(|(_, value)| value.into_owned());
        if ssl_mode.as_deref() != Some("verify-full") {
            return Err(ConfigError::Invalid("DATABASE_URL"));
        }
    }

    Ok(())
}

fn parse_addr<F>(lookup: &F, key: &'static str, default: &str) -> Result<SocketAddr, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let raw = lookup(key)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned());
    raw.parse().map_err(|_| ConfigError::Invalid(key))
}

fn parse_origin(raw: &str, key: &'static str, app_env: AppEnv) -> Result<String, ConfigError> {
    if raw.eq_ignore_ascii_case("null") || raw == "*" {
        return Err(ConfigError::Invalid(key));
    }

    let url = Url::parse(raw).map_err(|_| ConfigError::Invalid(key))?;
    let production_ok = !app_env.is_production() || url.scheme() == "https";

    if !matches!(url.scheme(), "http" | "https")
        || !production_ok
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(ConfigError::Invalid(key));
    }

    let origin = url.origin().ascii_serialization();
    if origin == "null" {
        return Err(ConfigError::Invalid(key));
    }
    Ok(origin)
}

fn parse_origins(
    raw: &str,
    key: &'static str,
    app_env: AppEnv,
) -> Result<Vec<String>, ConfigError> {
    let mut origins = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(ConfigError::Invalid(key));
        }
        let origin = parse_origin(part, key, app_env)?;
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }

    if origins.is_empty() {
        return Err(ConfigError::Invalid(key));
    }
    Ok(origins)
}

fn parse_cidrs(raw: &str) -> Result<Vec<IpNet>, ConfigError> {
    let mut cidrs = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(ConfigError::Invalid("TRUSTED_PROXY_CIDRS"));
        }
        let cidr = part
            .parse()
            .map_err(|_| ConfigError::Invalid("TRUSTED_PROXY_CIDRS"))?;
        if !cidrs.contains(&cidr) {
            cidrs.push(cidr);
        }
    }

    if cidrs.is_empty() {
        return Err(ConfigError::Invalid("TRUSTED_PROXY_CIDRS"));
    }
    Ok(cidrs)
}

pub(crate) fn parse_u32<F>(lookup: &F, key: &'static str, default: u32) -> Result<u32, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(key) {
        None => Ok(default),
        Some(raw) => raw.trim().parse().map_err(|_| ConfigError::Invalid(key)),
    }
}

pub(crate) fn parse_u64<F>(lookup: &F, key: &'static str, default: u64) -> Result<u64, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(key) {
        None => Ok(default),
        Some(raw) => raw.trim().parse().map_err(|_| ConfigError::Invalid(key)),
    }
}
