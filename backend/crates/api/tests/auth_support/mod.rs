//! Shared fixtures for the authentication HTTP tests.
//!
//! Each test gets its own `#[sqlx::test]` database, but the keyed services are
//! process-wide: deriving the dummy Argon2 hash and the Ed25519 key once keeps
//! the suite fast and memory-bounded.

#![allow(dead_code)]

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use rs_admin_api::{AppState, AuthServices, build_router};
use rs_admin_config::{AuditChainConfig, JwtConfig, RotationKeys, SecurityConfig};
use rs_admin_security::{AuditKeyRing, HmacKey, JwtIssuer, JwtSigningKey, PasswordService};
use rs_admin_store::Database;
use sqlx::PgPool;
use std::sync::OnceLock;
use tower::ServiceExt;
use uuid::Uuid;

pub const ISSUER: &str = "https://api.example.com";
pub const AUDIENCE: &str = "rs-admin-ds-api";
pub const KID: &str = "test-2026-11";
pub const AUDIT_KEY_ID: &str = "audit-2026-11";
pub const KEY_B64: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
pub const PASSWORD: &str = "correct horse battery staple";

fn private_pem() -> String {
    SigningKey::from_bytes(&[7u8; 32])
        .to_pkcs8_pem(LineEnding::LF)
        .expect("PKCS#8 encoding")
        .to_string()
}

fn signing_key() -> JwtSigningKey {
    JwtSigningKey::from_pem(&private_pem()).expect("signing key")
}

fn audit_keyring() -> AuditKeyRing {
    AuditKeyRing::from_json(
        &format!("{{\"{AUDIT_KEY_ID}\":\"{KEY_B64}\"}}"),
        AUDIT_KEY_ID,
    )
    .expect("keyring")
}

pub fn security_config() -> SecurityConfig {
    SecurityConfig {
        jwt: JwtConfig {
            issuer: ISSUER.to_owned(),
            audience: AUDIENCE.to_owned(),
            kid: KID.to_owned(),
            signing_key: signing_key(),
            previous_verification_key: None,
        },
        refresh_derivation: RotationKeys {
            current: HmacKey::from_bytes(vec![1u8; 32]).expect("key"),
            previous: None,
        },
        throttle_key: HmacKey::from_bytes(vec![2u8; 32]).expect("key"),
        audit_chain: AuditChainConfig {
            key_id: AUDIT_KEY_ID.to_owned(),
            keys: audit_keyring(),
        },
    }
}

/// Process-wide hashing service so tests share the expensive dummy hash and
/// the bounded hashing pool. The queue budget is generous because tests run in
/// parallel in a debug build, where Argon2 is much slower than production.
pub fn password_service() -> &'static PasswordService {
    static SERVICE: OnceLock<PasswordService> = OnceLock::new();
    SERVICE.get_or_init(|| {
        PasswordService::with_queue_timeout(8, std::time::Duration::from_secs(60))
            .expect("password service")
    })
}

fn auth_services() -> &'static AuthServices {
    static SERVICES: OnceLock<AuthServices> = OnceLock::new();
    SERVICES.get_or_init(|| {
        AuthServices::with_password_service(&security_config(), password_service().clone())
            .expect("auth services")
    })
}

/// Verifier equivalent to the API's codec, used to inspect issued tokens.
pub fn token_verifier() -> JwtIssuer {
    JwtIssuer::new(ISSUER, AUDIENCE, KID, &signing_key()).expect("verifier")
}

/// Build a full router over the test database.
pub async fn app(pool: &PgPool) -> Router {
    rs_admin_store::rbac::sync_catalog(pool)
        .await
        .expect("catalog sync");
    build_router(AppState::with_auth(
        Database::from_pool(pool.clone()),
        auth_services().clone(),
    ))
}

#[derive(Clone, Copy)]
pub struct NewUser<'a> {
    pub username: &'a str,
    pub email: &'a str,
    pub password: &'a str,
    pub status: &'a str,
    pub must_change_password: bool,
    pub temporary_password_expires_at: Option<DateTime<Utc>>,
    /// Built-in role name (`user`, `admin`, `super_admin`) to add in addition
    /// to the baseline `user` role.
    pub role: Option<&'a str>,
}

impl<'a> NewUser<'a> {
    pub fn active(username: &'a str, email: &'a str) -> Self {
        Self {
            username,
            email,
            password: PASSWORD,
            status: "active",
            must_change_password: false,
            temporary_password_expires_at: None,
            role: None,
        }
    }
}

pub async fn insert_user(pool: &PgPool, user: NewUser<'_>) -> Uuid {
    let hash = password_service()
        .hash(user.password)
        .await
        .expect("password hash");
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (
            username, email, display_name, password_hash, status,
            must_change_password, temporary_password_expires_at
         ) VALUES ($1, $2, $1, $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(user.username)
    .bind(user.email)
    .bind(hash)
    .bind(user.status)
    .bind(user.must_change_password)
    .bind(user.temporary_password_expires_at)
    .fetch_one(pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id)
         SELECT $1, id FROM roles WHERE name = 'user'",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("baseline role");

    if let Some(role) = user.role.filter(|role| *role != "user") {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id)
             SELECT $1, id FROM roles WHERE name = $2",
        )
        .bind(id)
        .bind(role)
        .execute(pool)
        .await
        .expect("extra role");
    }

    id
}

pub fn request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

pub fn json_request(method: &str, uri: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(body).expect("json")))
        .expect("request")
}

/// `POST /auth/refresh` with the raw token carried in the cookie header. The
/// access token is intentionally absent: refresh authenticates by cookie only.
pub fn refresh_request(raw: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/refresh")
        .header(header::CONTENT_TYPE, "application/json");
    let builder = match raw {
        Some(raw) => builder.header(header::COOKIE, format!("__Host-refresh={raw}")),
        None => builder,
    };
    builder.body(Body::from("{}")).expect("refresh request")
}

/// `POST /auth/logout` with the raw token carried in the cookie header. Logout
/// authenticates by cookie only; the access token is intentionally absent.
pub fn logout_request(raw: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/logout")
        .header(header::CONTENT_TYPE, "application/json");
    let builder = match raw {
        Some(raw) => builder.header(header::COOKIE, format!("__Host-refresh={raw}")),
        None => builder,
    };
    builder.body(Body::from("{}")).expect("logout request")
}

pub fn authorized_request(
    method: &str,
    uri: &str,
    token: &str,
    body: Option<&serde_json::Value>,
) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).expect("json")))
            .expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    }
}

pub async fn send(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
    let response = app.oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, headers, body)
}

pub fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).expect("json body")
}

/// Stable problem `code` from a problem-details body.
pub fn problem_code(body: &serde_json::Value) -> &str {
    body["code"].as_str().expect("problem code")
}

pub async fn login(
    app: &Router,
    identifier: &str,
    password: &str,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let (status, headers, body) = send(
        app.clone(),
        json_request(
            "POST",
            "/api/v1/auth/login",
            &serde_json::json!({ "identifier": identifier, "password": password }),
        ),
    )
    .await;
    (status, headers, json(&body))
}

/// Raw refresh token from `Set-Cookie`, without attributes.
pub fn refresh_cookie(headers: &HeaderMap) -> String {
    let header = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("Set-Cookie");
    let pair = header.split(';').next().expect("cookie pair");
    let (name, value) = pair.split_once('=').expect("cookie value");
    assert_eq!(name, "__Host-refresh");
    value.to_owned()
}

pub async fn refresh_token_rows(pool: &PgPool, session_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM auth_refresh_tokens WHERE session_id = $1")
        .bind(session_id)
        .fetch_one(pool)
        .await
        .expect("token count")
}

pub async fn session_count(pool: &PgPool, user_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("session count")
}
