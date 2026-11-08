use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
use rs_admin_config::{AppEnv, Config, ConfigError, DatabaseConfig, OriginConfig};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

const KEY_B64: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
const OTHER_KEY_B64: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
// Base64 of 31 bytes: one byte short of the required minimum.
const SHORT_KEY_B64: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==";

fn private_pem() -> String {
    SigningKey::from_bytes(&[7u8; 32])
        .to_pkcs8_pem(LineEnding::LF)
        .expect("PKCS#8 encoding")
        .to_string()
}

fn public_pem() -> String {
    SigningKey::from_bytes(&[7u8; 32])
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .expect("SPKI encoding")
}

fn audit_keyring() -> String {
    format!("{{\"audit-2026-11\":\"{KEY_B64}\"}}")
}

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
        ("JWT_ISSUER".into(), "rs-admin".into()),
        ("JWT_AUDIENCE".into(), "rs-admin-client".into()),
        ("JWT_EDDSA_KID".into(), "jwt-2026-11".into()),
        ("JWT_EDDSA_PRIVATE_KEY".into(), private_pem()),
        ("REFRESH_DERIVATION_KEY".into(), KEY_B64.into()),
        ("THROTTLE_KEY".into(), KEY_B64.into()),
        ("AUDIT_CHAIN_KEY_ID".into(), "audit-2026-11".into()),
        ("AUDIT_CHAIN_KEYS".into(), audit_keyring()),
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
fn security_keys_are_loaded_and_validated() {
    let config = load(&base()).expect("valid security config");

    assert_eq!(config.security.jwt.issuer, "rs-admin");
    assert_eq!(config.security.jwt.audience, "rs-admin-client");
    assert_eq!(config.security.jwt.kid, "jwt-2026-11");
    assert!(config.security.jwt.previous_verification_key.is_none());

    assert_eq!(config.security.refresh_derivation.current.len(), 32);
    assert!(config.security.refresh_derivation.previous.is_none());
    assert_eq!(config.security.throttle_key.len(), 32);

    assert_eq!(config.security.audit_chain.key_id, "audit-2026-11");
    assert_eq!(
        config.security.audit_chain.keys.current_id(),
        "audit-2026-11"
    );
    assert_eq!(config.security.audit_chain.keys.len(), 1);
}

#[test]
fn auth_and_audit_settings_use_spec_defaults() {
    let config = load(&base()).expect("valid settings");

    assert!((1..=4).contains(&config.auth.password_hash_concurrency));
    assert_eq!(config.auth.login_ip_failure_limit, 20);
    assert_eq!(config.auth.login_ip_window, Duration::from_secs(600));
    assert_eq!(config.auth.login_ip_lock, Duration::from_secs(900));
    assert_eq!(config.auth.login_global_alert_limit, 200);
    assert_eq!(config.auth.login_global_circuit_limit, 1000);
    assert_eq!(config.auth.login_global_window, Duration::from_secs(300));
    assert_eq!(config.auth.login_global_lock, Duration::from_secs(60));
    assert_eq!(
        config.auth.account_failure_decay,
        Duration::from_secs(24 * 3600)
    );
    assert_eq!(
        config.auth.temp_password_ttl,
        Duration::from_secs(72 * 3600)
    );
    assert_eq!(config.auth.reauth_max_age, Duration::from_secs(300));

    assert_eq!(config.audit.retention_info_days, 180);
    assert_eq!(config.audit.retention_critical_days, 365);
}

#[test]
fn auth_and_audit_settings_can_be_overridden() {
    let mut map = base();
    map.extend([
        ("PASSWORD_HASH_CONCURRENCY".into(), "2".into()),
        ("LOGIN_IP_FAILURE_LIMIT".into(), "5".into()),
        ("LOGIN_IP_WINDOW_SECONDS".into(), "60".into()),
        ("LOGIN_IP_LOCK_SECONDS".into(), "120".into()),
        ("LOGIN_GLOBAL_ALERT_LIMIT".into(), "10".into()),
        ("LOGIN_GLOBAL_CIRCUIT_LIMIT".into(), "50".into()),
        ("LOGIN_GLOBAL_WINDOW_SECONDS".into(), "30".into()),
        ("LOGIN_GLOBAL_LOCK_SECONDS".into(), "15".into()),
        ("ACCOUNT_FAILURE_DECAY_HOURS".into(), "12".into()),
        ("TEMP_PASSWORD_TTL_HOURS".into(), "24".into()),
        ("REAUTH_MAX_AGE_SECONDS".into(), "60".into()),
        ("AUDIT_RETENTION_INFO_DAYS".into(), "200".into()),
        ("AUDIT_RETENTION_CRITICAL_DAYS".into(), "400".into()),
    ]);

    let config = load(&map).expect("valid overrides");
    assert_eq!(config.auth.password_hash_concurrency, 2);
    assert_eq!(config.auth.login_ip_failure_limit, 5);
    assert_eq!(config.auth.login_ip_window, Duration::from_secs(60));
    assert_eq!(config.auth.login_ip_lock, Duration::from_secs(120));
    assert_eq!(config.auth.login_global_alert_limit, 10);
    assert_eq!(config.auth.login_global_circuit_limit, 50);
    assert_eq!(config.auth.login_global_window, Duration::from_secs(30));
    assert_eq!(config.auth.login_global_lock, Duration::from_secs(15));
    assert_eq!(
        config.auth.account_failure_decay,
        Duration::from_secs(12 * 3600)
    );
    assert_eq!(
        config.auth.temp_password_ttl,
        Duration::from_secs(24 * 3600)
    );
    assert_eq!(config.auth.reauth_max_age, Duration::from_secs(60));
    assert_eq!(config.audit.retention_info_days, 200);
    assert_eq!(config.audit.retention_critical_days, 400);
}

#[test]
fn invalid_settings_name_the_key_without_values() {
    for (key, value) in [
        ("PASSWORD_HASH_CONCURRENCY", "0"),
        ("LOGIN_IP_FAILURE_LIMIT", "0"),
        ("LOGIN_IP_WINDOW_SECONDS", "0"),
        ("LOGIN_GLOBAL_CIRCUIT_LIMIT", "0"),
        ("ACCOUNT_FAILURE_DECAY_HOURS", "0"),
        ("TEMP_PASSWORD_TTL_HOURS", "0"),
        ("REAUTH_MAX_AGE_SECONDS", "0"),
        ("AUDIT_RETENTION_INFO_DAYS", "179"),
        ("AUDIT_RETENTION_CRITICAL_DAYS", "364"),
    ] {
        let mut map = base();
        map.insert(key.into(), value.into());
        let error = load(&map).expect_err(key);
        assert!(error.to_string().contains(key), "{key}: {error}");
    }

    let mut map = base();
    map.insert("LOGIN_GLOBAL_ALERT_LIMIT".into(), "1001".into());
    let error = load(&map).expect_err("alert above circuit");
    assert!(error.to_string().contains("LOGIN_GLOBAL_ALERT_LIMIT"));
}

#[test]
fn missing_security_configuration_names_the_key() {
    for key in [
        "JWT_ISSUER",
        "JWT_AUDIENCE",
        "JWT_EDDSA_KID",
        "JWT_EDDSA_PRIVATE_KEY",
        "REFRESH_DERIVATION_KEY",
        "THROTTLE_KEY",
        "AUDIT_CHAIN_KEY_ID",
        "AUDIT_CHAIN_KEYS",
    ] {
        let mut map = base();
        map.remove(key);
        let error = load(&map).expect_err(key);
        assert!(error.to_string().contains(key), "{key}: {error}");
    }
}

#[test]
fn blank_jwt_identity_fields_are_rejected() {
    for key in ["JWT_ISSUER", "JWT_AUDIENCE", "JWT_EDDSA_KID"] {
        let mut map = base();
        map.insert(key.into(), "   ".into());
        let error = load(&map).expect_err(key);
        assert!(error.to_string().contains(key), "{key}: {error}");
    }
}

#[test]
fn short_hmac_keys_are_rejected_without_echoing_them() {
    for key in ["REFRESH_DERIVATION_KEY", "THROTTLE_KEY"] {
        let mut map = base();
        map.insert(key.into(), SHORT_KEY_B64.into());
        let error = load(&map).expect_err(key);
        assert!(error.to_string().contains(key), "{key}: {error}");
        assert!(!error.to_string().contains("BwcHBwc"));
    }
}

#[test]
fn audit_chain_keyring_must_contain_the_current_key() {
    let mut map = base();
    map.insert(
        "AUDIT_CHAIN_KEYS".into(),
        format!("{{\"audit-2026-10\":\"{KEY_B64}\"}}"),
    );

    let error = load(&map).expect_err("missing current key");
    assert!(error.to_string().contains("AUDIT_CHAIN_KEY_ID"));
}

#[test]
fn invalid_audit_chain_keys_name_the_key_without_echoing_values() {
    for (value, expected_key) in [
        ("not json", "AUDIT_CHAIN_KEYS"),
        (
            "{\"audit-2026-11\":\"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBw==\"}",
            "AUDIT_CHAIN_KEYS",
        ),
    ] {
        let mut map = base();
        map.insert("AUDIT_CHAIN_KEYS".into(), value.into());
        let error = load(&map).expect_err("invalid keyring");
        assert!(error.to_string().contains(expected_key), "{error}");
        assert!(!error.to_string().contains("BwcHBwc"), "{error}");
    }
}

#[test]
fn invalid_jwt_pem_never_echoes_key_material() {
    let marker =
        "-----BEGIN PRIVATE KEY-----\nSUPER_SECRET_KEY_MATERIAL\n-----END PRIVATE KEY-----";
    let mut map = base();
    map.insert("JWT_EDDSA_PRIVATE_KEY".into(), marker.into());

    let error = load(&map).expect_err("invalid pem");
    assert!(error.to_string().contains("JWT_EDDSA_PRIVATE_KEY"));
    assert!(!error.to_string().contains("SUPER_SECRET_KEY_MATERIAL"));
}

#[test]
fn previous_rotation_keys_are_optional_and_validated() {
    let mut map = base();
    map.insert("JWT_EDDSA_PREVIOUS_PUBLIC_KEY".into(), public_pem());
    map.insert(
        "REFRESH_DERIVATION_KEY_PREVIOUS".into(),
        OTHER_KEY_B64.into(),
    );
    let config = load(&map).expect("valid rotation keys");
    assert!(config.security.jwt.previous_verification_key.is_some());
    assert_eq!(
        config
            .security
            .refresh_derivation
            .previous
            .expect("previous key")
            .len(),
        32
    );

    let mut bad_public = map.clone();
    bad_public.insert("JWT_EDDSA_PREVIOUS_PUBLIC_KEY".into(), private_pem());
    assert!(load(&bad_public).is_err());

    let mut bad_previous = map;
    bad_previous.insert("REFRESH_DERIVATION_KEY_PREVIOUS".into(), "nope".into());
    let error = load(&bad_previous).expect_err("invalid previous key");
    assert!(
        error
            .to_string()
            .contains("REFRESH_DERIVATION_KEY_PREVIOUS")
    );
}

#[test]
fn secret_files_take_precedence_for_every_loaded_secret() {
    let keyring = audit_keyring();
    let mut files = Vec::new();
    for (env_key, contents) in [
        ("JWT_EDDSA_PRIVATE_KEY_FILE", private_pem()),
        ("JWT_EDDSA_PREVIOUS_PUBLIC_KEY_FILE", public_pem()),
        ("REFRESH_DERIVATION_KEY_FILE", OTHER_KEY_B64.to_string()),
        (
            "REFRESH_DERIVATION_KEY_PREVIOUS_FILE",
            OTHER_KEY_B64.to_string(),
        ),
        ("THROTTLE_KEY_FILE", OTHER_KEY_B64.to_string()),
        ("AUDIT_CHAIN_KEYS_FILE", keyring),
    ] {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        write!(file, "{contents}").expect("write temp file");
        files.push((env_key, file));
    }

    let mut map = base();
    for (env_key, file) in &files {
        map.remove(env_key.trim_end_matches("_FILE"));
        map.insert(
            (*env_key).into(),
            file.path().to_string_lossy().into_owned(),
        );
    }

    let config = load(&map).expect("file-backed secrets");
    assert!(config.security.jwt.previous_verification_key.is_some());
    assert_eq!(config.security.refresh_derivation.current.len(), 32);
    assert_eq!(
        config
            .security
            .refresh_derivation
            .previous
            .expect("previous key")
            .len(),
        32
    );
    assert_eq!(config.security.throttle_key.len(), 32);
    assert_eq!(config.security.audit_chain.keys.len(), 1);
}

#[test]
fn unreadable_secret_files_are_errors_and_name_the_file_key() {
    for key in [
        "JWT_EDDSA_PRIVATE_KEY_FILE",
        "REFRESH_DERIVATION_KEY_FILE",
        "THROTTLE_KEY_FILE",
        "AUDIT_CHAIN_KEYS_FILE",
    ] {
        let mut map = base();
        map.insert(key.into(), "/nonexistent/rs-admin/secret".into());
        let error = load(&map).expect_err(key);
        assert!(error.to_string().contains(key), "{key}: {error}");
    }
}

#[test]
fn config_debug_output_redacts_all_key_material() {
    let mut map = base();
    map.insert(
        "REFRESH_DERIVATION_KEY_PREVIOUS".into(),
        OTHER_KEY_B64.into(),
    );
    map.insert("JWT_EDDSA_PREVIOUS_PUBLIC_KEY".into(), public_pem());
    let config = load(&map).expect("valid config");

    let rendered = format!("{config:?}");
    for forbidden in [
        "supersecret",
        "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=",
        OTHER_KEY_B64,
        "PRIVATE KEY",
    ] {
        assert!(!rendered.contains(forbidden), "leaked {forbidden}");
    }
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
fn unreadable_database_secret_file_is_an_error_and_names_the_key() {
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
fn empty_trusted_proxy_cidrs_disable_forwarded_headers() {
    let mut map = base();
    map.remove("TRUSTED_PROXY_CIDRS");
    assert!(
        load(&map)
            .expect("missing list is allowed")
            .trusted_proxy_cidrs
            .is_empty()
    );

    map.insert("TRUSTED_PROXY_CIDRS".into(), String::new());
    assert!(
        load(&map)
            .expect("empty list is allowed")
            .trusted_proxy_cidrs
            .is_empty()
    );
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
