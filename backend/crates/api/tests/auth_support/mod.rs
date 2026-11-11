//! Shared fixtures for the authentication HTTP tests.
//!
//! Each test gets its own `#[sqlx::test]` database, but the keyed services are
//! process-wide: deriving the dummy Argon2 hash and the Ed25519 key once keeps
//! the suite fast and memory-bounded.

#![allow(dead_code)]

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode, header};
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use rs_admin_api::{AppState, AuthServices, build_router};
use rs_admin_config::{AuditChainConfig, AuthSettings, JwtConfig, RotationKeys, SecurityConfig};
use rs_admin_security::{
    AuditKeyRing, HmacKey, JwtIssuer, JwtSigningKey, PasswordService, TrustedProxies,
};
use rs_admin_store::Database;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::OnceLock;
use tower::ServiceExt;
use uuid::Uuid;

pub const ISSUER: &str = "https://api.example.com";
pub const AUDIENCE: &str = "rs-admin-ds-api";
pub const KID: &str = "test-2026-11";
pub const AUDIT_KEY_ID: &str = "audit-2026-11";
pub const KEY_B64: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
pub const PASSWORD: &str = "correct horse battery staple";

/// The development admin origin; tests trust exactly this origin.
pub const TEST_ORIGIN: &str = "http://localhost:5173";

/// Header every cookie-authenticated endpoint requires (spec §7.11).
pub const CSRF_HEADER: &str = "x-requested-with";
pub const CSRF_VALUE: &str = "XMLHttpRequest";

pub fn allowed_origins() -> Vec<String> {
    vec![TEST_ORIGIN.to_owned()]
}

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
            previous_kid: None,
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

/// Spec-default throttle settings; tests override individual fields.
pub fn auth_settings() -> AuthSettings {
    AuthSettings::from_source(&|_| None).expect("default auth settings")
}

fn auth_services() -> &'static AuthServices {
    static SERVICES: OnceLock<AuthServices> = OnceLock::new();
    SERVICES.get_or_init(|| {
        AuthServices::with_password_service(
            &security_config(),
            password_service().clone(),
            auth_settings(),
            TrustedProxies::default(),
        )
        .expect("auth services")
    })
}

/// Verifier equivalent to the API's codec, used to inspect issued tokens.
pub fn token_verifier() -> JwtIssuer {
    JwtIssuer::new(ISSUER, AUDIENCE, KID, &signing_key()).expect("verifier")
}

/// Build a full router over the test database with spec-default settings.
pub async fn app(pool: &PgPool) -> Router {
    app_with(pool, auth_settings(), TrustedProxies::default()).await
}

/// Build a router with custom throttle settings and trusted proxies.
pub async fn app_with(pool: &PgPool, settings: AuthSettings, proxies: TrustedProxies) -> Router {
    rs_admin_store::rbac::sync_catalog(pool)
        .await
        .expect("catalog sync");
    let services = AuthServices::with_password_service(
        &security_config(),
        password_service().clone(),
        settings,
        proxies,
    )
    .expect("auth services");
    build_router(
        AppState::with_auth(Database::from_pool(pool.clone()), services)
            .with_allowed_origins(allowed_origins()),
    )
}

#[derive(Clone, Copy)]
pub struct NewUser<'a> {
    pub username: &'a str,
    pub email: &'a str,
    /// Defaults to the username when absent.
    pub display_name: Option<&'a str>,
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
            display_name: None,
            password: PASSWORD,
            status: "active",
            must_change_password: false,
            temporary_password_expires_at: None,
            role: None,
        }
    }

    /// An active user holding a temporary password that expires in 72 hours.
    pub fn restricted(username: &'a str, email: &'a str) -> Self {
        Self {
            must_change_password: true,
            temporary_password_expires_at: Some(Utc::now() + Duration::hours(72)),
            ..Self::active(username, email)
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
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(user.username)
    .bind(user.email)
    .bind(user.display_name.unwrap_or(user.username))
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

/// A JSON request from the trusted admin origin carrying the CSRF header.
pub fn json_request(method: &str, uri: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, TEST_ORIGIN)
        .header(CSRF_HEADER, CSRF_VALUE)
        .body(Body::from(serde_json::to_vec(body).expect("json")))
        .expect("request")
}

/// `POST /auth/refresh` with the raw token carried in the cookie header. The
/// access token is intentionally absent: refresh authenticates by cookie only.
pub fn refresh_request(raw: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/refresh")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, TEST_ORIGIN)
        .header(CSRF_HEADER, CSRF_VALUE);
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
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, TEST_ORIGIN)
        .header(CSRF_HEADER, CSRF_VALUE);
    let builder = match raw {
        Some(raw) => builder.header(header::COOKIE, format!("__Host-refresh={raw}")),
        None => builder,
    };
    builder.body(Body::from("{}")).expect("logout request")
}

/// `POST /auth/change-password` with Bearer authentication and, when given, the
/// refresh cookie the handler rotates on success.
pub fn change_password_request(
    token: &str,
    raw_refresh: Option<&str>,
    current_password: &str,
    new_password: &str,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/change-password")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, TEST_ORIGIN);
    if let Some(raw) = raw_refresh {
        builder = builder.header(header::COOKIE, format!("__Host-refresh={raw}"));
    }
    builder
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "current_password": current_password,
                "new_password": new_password,
            }))
            .expect("json"),
        ))
        .expect("change-password request")
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
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::ORIGIN, TEST_ORIGIN)
        .header(CSRF_HEADER, CSRF_VALUE);
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).expect("json")))
            .expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    }
}

/// Send a request, defaulting the TCP peer to loopback when the test did not
/// choose one. Production always supplies `ConnectInfo` from the listener.
pub async fn send(app: Router, mut request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
    if request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_none()
    {
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
    }
    let response = app.oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, headers, body)
}

/// A JSON request from an explicit TCP peer, with extra headers such as
/// `X-Forwarded-For`.
pub fn request_from(
    method: &str,
    uri: &str,
    body: &serde_json::Value,
    peer: SocketAddr,
    extra_headers: &[(&str, &str)],
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder
        .body(Body::from(serde_json::to_vec(body).expect("json")))
        .expect("request");
    request.extensions_mut().insert(ConnectInfo(peer));
    request
}

/// Log in from an explicit TCP peer with optional extra request headers.
pub async fn login_from(
    app: &Router,
    identifier: &str,
    password: &str,
    peer: SocketAddr,
    extra_headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, serde_json::Value) {
    // Defaults come last so a test that overrides a header in `extra_headers`
    // wins: `HeaderMap::get` returns the first value.
    let mut headers = extra_headers.to_vec();
    for (name, value) in [("origin", TEST_ORIGIN), (CSRF_HEADER, CSRF_VALUE)] {
        if !headers
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
        {
            headers.push((name, value));
        }
    }
    let (status, headers, body) = send(
        app.clone(),
        request_from(
            "POST",
            "/api/v1/auth/login",
            &serde_json::json!({ "identifier": identifier, "password": password }),
            peer,
            &headers,
        ),
    )
    .await;
    (status, headers, json(&body))
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
