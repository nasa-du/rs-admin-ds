use rs_admin_config::{AppEnv, Config, ConfigError, DatabaseConfig, OriginConfig};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

fn base() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("APP_ENV".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://rs_admin:supersecret@127.0.0.1:5432/rs_admin".into(),
        ),
        ("PUBLIC_API_ORIGIN".into(), "http://localhost:8080".into()),
        ("ADMIN_ORIGIN".into(), "http://localhost:5173".into()),
        (
            "CORS_ALLOWED_ORIGINS".into(),
            "http://localhost:5173".into(),
        ),
        ("TRUSTED_PROXY_CIDRS".into(), "127.0.0.0/8,::1/128".into()),
    ])
}

fn load(map: &BTreeMap<String, String>) -> Result<Config, ConfigError> {
    Config::from_source(|key| map.get(key).cloned())
}

#[test]
fn development_config_loads_with_spec_defaults() {
    let config = load(&base()).expect("valid development config");

    assert_eq!(config.app_env, AppEnv::Development);
    assert_eq!(config.database.pool.max_connections, 10);
    assert_eq!(config.database.pool.min_connections, 1);
    assert_eq!(config.database.pool.acquire_timeout, Duration::from_secs(3));
    assert_eq!(config.database.pool.idle_timeout, Duration::from_secs(600));
    assert_eq!(config.database.pool.max_lifetime, Duration::from_secs(1800));
    assert_eq!(config.server.api_addr.to_string(), "0.0.0.0:8080");
    assert_eq!(config.server.metrics_addr.to_string(), "0.0.0.0:9100");
    assert_eq!(config.origins.public_api_origin, "http://localhost:8080");
    assert_eq!(config.origins.admin_origin, "http://localhost:5173");
    assert_eq!(
        config.origins.cors_allowed_origins,
        vec!["http://localhost:5173".to_string()]
    );
    assert_eq!(config.trusted_proxy_cidrs.len(), 2);
}

#[test]
fn pool_settings_can_be_overridden() {
    let mut map = base();
    map.extend([
        ("DATABASE_MAX_CONNECTIONS".into(), "7".into()),
        ("DATABASE_MIN_CONNECTIONS".into(), "2".into()),
        ("DATABASE_ACQUIRE_TIMEOUT_SECONDS".into(), "5".into()),
        ("DATABASE_IDLE_TIMEOUT_SECONDS".into(), "120".into()),
        ("DATABASE_MAX_LIFETIME_SECONDS".into(), "60".into()),
    ]);

    let config = load(&map).expect("valid overrides");
    assert_eq!(config.database.pool.max_connections, 7);
    assert_eq!(config.database.pool.min_connections, 2);
    assert_eq!(config.database.pool.acquire_timeout, Duration::from_secs(5));
    assert_eq!(config.database.pool.idle_timeout, Duration::from_secs(120));
    assert_eq!(config.database.pool.max_lifetime, Duration::from_secs(60));
}

#[test]
fn secret_file_takes_precedence_over_environment() {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    writeln!(
        file,
        "postgres://from_file:file_secret@db.internal:5432/rs_admin?sslmode=verify-full"
    )
    .expect("write temp file");

    let mut map = base();
    map.insert(
        "DATABASE_URL_FILE".into(),
        file.path().to_string_lossy().into_owned(),
    );

    let config = load(&map).expect("valid file-backed config");
    assert!(config.database.url().contains("from_file"));
    assert!(!config.database.url().contains("supersecret"));
}

#[test]
fn unreadable_secret_file_is_an_error_and_names_the_key() {
    let mut map = base();
    map.insert(
        "DATABASE_URL_FILE".into(),
        "/nonexistent/rs-admin/database_url".into(),
    );

    let error = load(&map).expect_err("missing secret file must fail");
    assert!(error.to_string().contains("DATABASE_URL_FILE"));
}

#[test]
fn missing_required_configuration_names_the_key_without_values() {
    let mut map = base();
    map.remove("DATABASE_URL");

    let error = load(&map).expect_err("missing database url must fail");
    let message = error.to_string();
    assert!(message.contains("DATABASE_URL"));
    assert!(!message.contains("supersecret"));
}

#[test]
fn invalid_database_url_never_echoes_the_secret() {
    let mut map = base();
    map.insert(
        "DATABASE_URL".into(),
        "postgres://user:supersecret@/missing-host".into(),
    );

    let error = load(&map).expect_err("invalid database url must fail");
    assert!(!error.to_string().contains("supersecret"));
}

#[test]
fn production_requires_https_origins_and_verified_database_tls() {
    let production = || {
        let mut map = base();
        map.extend([
            ("APP_ENV".into(), "production".into()),
            (
                "DATABASE_URL".into(),
                "postgres://rs_admin:supersecret@db.internal:5432/rs_admin?sslmode=verify-full"
                    .into(),
            ),
            ("PUBLIC_API_ORIGIN".into(), "https://api.example.com".into()),
            ("ADMIN_ORIGIN".into(), "https://admin.example.com".into()),
            (
                "CORS_ALLOWED_ORIGINS".into(),
                "https://admin.example.com".into(),
            ),
        ]);
        map
    };

    load(&production()).expect("valid production config");

    let mut insecure_origin = production();
    insecure_origin.insert("ADMIN_ORIGIN".into(), "http://admin.example.com".into());
    assert!(load(&insecure_origin).is_err());

    let mut insecure_database = production();
    insecure_database.insert(
        "DATABASE_URL".into(),
        "postgres://rs_admin:supersecret@db.internal:5432/rs_admin".into(),
    );
    assert!(load(&insecure_database).is_err());
}

#[test]
fn cors_origins_are_canonicalized_deduplicated_and_strict() {
    let mut map = base();
    map.insert(
        "CORS_ALLOWED_ORIGINS".into(),
        " https://admin.example.com/ , https://admin.example.com, https://other.example.com/ "
            .into(),
    );
    let config = load(&map).expect("valid cors list");
    assert_eq!(
        config.origins.cors_allowed_origins,
        vec![
            "https://admin.example.com".to_string(),
            "https://other.example.com".to_string()
        ]
    );

    for invalid in ["*", "null", "https://admin.example.com/path"] {
        let mut map = base();
        map.insert("CORS_ALLOWED_ORIGINS".into(), invalid.into());
        assert!(load(&map).is_err(), "{invalid} must be rejected");
    }
}

#[test]
fn invalid_app_env_and_cidrs_are_rejected() {
    let mut map = base();
    map.insert("APP_ENV".into(), "staging".into());
    assert!(load(&map).is_err());

    let mut map = base();
    map.insert("TRUSTED_PROXY_CIDRS".into(), "not-a-cidr".into());
    assert!(load(&map).is_err());
}

#[test]
fn origin_config_type_is_public_and_usable() {
    let mut map = base();
    let config = load(&map).expect("valid config");
    let origins: &OriginConfig = &config.origins;
    assert!(origins.cors_allowed_origins[0].starts_with("http"));

    map.insert("CORS_ALLOWED_ORIGINS".into(), String::new());
    assert!(load(&map).is_err());
}

#[test]
fn debug_output_redacts_the_database_url() {
    let config = load(&base()).expect("valid config");
    let rendered = format!("{config:?}");
    assert!(!rendered.contains("supersecret"));
}

#[test]
fn database_config_can_be_loaded_without_http_origins() {
    let map = BTreeMap::from([
        ("APP_ENV".to_string(), "development".to_string()),
        (
            "DATABASE_URL".to_string(),
            "postgres://rs_admin:supersecret@127.0.0.1:5432/rs_admin".to_string(),
        ),
    ]);

    let database = DatabaseConfig::from_source(|key| map.get(key).cloned())
        .expect("database-only configuration");
    assert!(database.url().contains("rs_admin"));
    assert_eq!(database.pool.max_connections, 10);
}
