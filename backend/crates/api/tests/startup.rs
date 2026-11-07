//! Startup validation is observable through the process exit code and output.

use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use std::collections::BTreeMap;
use std::process::{Command, Output};

const KEY_B64: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

fn private_pem() -> String {
    SigningKey::from_bytes(&[7u8; 32])
        .to_pkcs8_pem(LineEnding::LF)
        .expect("PKCS#8 encoding")
        .to_string()
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("APP_ENV".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://rs_admin:supersecret@127.0.0.1:1/rs_admin".into(),
        ),
        ("PUBLIC_API_ORIGIN".into(), "http://localhost:8080".into()),
        ("ADMIN_ORIGIN".into(), "http://localhost:5173".into()),
        (
            "CORS_ALLOWED_ORIGINS".into(),
            "http://localhost:5173".into(),
        ),
        ("TRUSTED_PROXY_CIDRS".into(), "127.0.0.0/8".into()),
        ("JWT_ISSUER".into(), "rs-admin".into()),
        ("JWT_AUDIENCE".into(), "rs-admin-client".into()),
        ("JWT_EDDSA_KID".into(), "jwt-2026-11".into()),
        ("JWT_EDDSA_PRIVATE_KEY".into(), private_pem()),
        ("REFRESH_DERIVATION_KEY".into(), KEY_B64.into()),
        ("THROTTLE_KEY".into(), KEY_B64.into()),
        ("AUDIT_CHAIN_KEY_ID".into(), "audit-2026-11".into()),
        (
            "AUDIT_CHAIN_KEYS".into(),
            format!("{{\"audit-2026-11\":\"{KEY_B64}\"}}"),
        ),
    ])
}

fn run(environment: &BTreeMap<String, String>) -> Output {
    // A directory without a `.env` keeps the test independent of local setup.
    // Keep the handle alive until the process has started.
    let working_directory = tempfile::tempdir().expect("temp dir");

    let mut command = Command::new(env!("CARGO_BIN_EXE_rs-admin-api"));
    command.env_clear();
    command.current_dir(working_directory.path());
    for (key, value) in environment {
        command.env(key, value);
    }
    command.output().expect("run api binary")
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn missing_configuration_exits_nonzero_and_names_the_key() {
    let mut environment = environment();
    environment.remove("JWT_ISSUER");

    let output = run(&environment);
    assert!(!output.status.success(), "process must exit non-zero");

    let text = combined_output(&output);
    assert!(
        text.contains("JWT_ISSUER"),
        "output must name the key: {text}"
    );
}

#[test]
fn invalid_secret_files_exit_nonzero_without_echoing_values() {
    let mut environment = environment();
    environment.insert(
        "JWT_EDDSA_PRIVATE_KEY".into(),
        "-----BEGIN PRIVATE KEY-----\nSUPER_SECRET_KEY_MATERIAL\n-----END PRIVATE KEY-----".into(),
    );

    let output = run(&environment);
    assert!(!output.status.success(), "process must exit non-zero");

    let text = combined_output(&output);
    assert!(text.contains("JWT_EDDSA_PRIVATE_KEY"));
    assert!(
        !text.contains("SUPER_SECRET_KEY_MATERIAL"),
        "secret material leaked: {text}"
    );
    assert!(!text.contains("supersecret"), "database URL leaked: {text}");
}

#[test]
fn audit_keyring_without_the_current_key_exits_nonzero() {
    let mut environment = environment();
    environment.insert(
        "AUDIT_CHAIN_KEYS".into(),
        format!("{{\"audit-2026-10\":\"{KEY_B64}\"}}"),
    );

    let output = run(&environment);
    assert!(!output.status.success(), "process must exit non-zero");

    let text = combined_output(&output);
    assert!(text.contains("AUDIT_CHAIN_KEY_ID"), "output: {text}");
    assert!(!text.contains(KEY_B64), "key material leaked: {text}");
}

#[test]
fn unreachable_database_exits_nonzero_without_leaking_the_url() {
    let output = run(&environment());
    assert!(!output.status.success(), "process must exit non-zero");

    let text = combined_output(&output);
    assert!(!text.contains("supersecret"), "database URL leaked: {text}");
    assert!(
        !text.to_ascii_lowercase().contains("connection refused"),
        "internal connection detail leaked: {text}"
    );
}
