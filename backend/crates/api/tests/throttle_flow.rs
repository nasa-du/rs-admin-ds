//! Three-layer login throttling and trusted-proxy client IP over HTTP
//! (docs/specs/login-system.md §8.3–§8.4, AC-AUTH-03/04).

mod auth_support;

use auth_support::{
    PASSWORD, app, app_with, auth_settings, authorized_request, insert_user, json, login,
    login_from, problem_code, refresh_cookie, refresh_request, send,
};
use axum::Router;
use axum::http::{HeaderMap, StatusCode, header};
use rs_admin_security::TrustedProxies;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::time::Duration;

fn retry_after(headers: &HeaderMap) -> i64 {
    headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .expect("Retry-After header")
        .parse()
        .expect("Retry-After seconds")
}

fn assert_retry_after(headers: &HeaderMap, maximum: i64) {
    let seconds = retry_after(headers);
    assert!(
        (1..=maximum).contains(&seconds),
        "Retry-After {seconds} must be within 1..={maximum}"
    );
}

async fn reauthenticate(
    app: &Router,
    token: &str,
    password: &str,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let (status, headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            token,
            Some(&serde_json::json!({ "password": password })),
        ),
    )
    .await;
    (status, headers, json(&body))
}

#[sqlx::test(migrations = "../../migrations")]
async fn five_failures_lock_the_account_and_locked_attempts_do_not_count(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(
        &pool,
        auth_support::NewUser::active("alice", "alice@example.com"),
    )
    .await;

    for _ in 0..4 {
        let (status, _headers, body) = login(&app, "alice", "wrong password").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(problem_code(&body), "invalid_credentials");
    }

    let (status, headers, body) = login(&app, "alice", "wrong password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "login_locked");
    assert_retry_after(&headers, 30);

    // Locked attempts are refused even with the correct password, and the
    // counter does not move (spec §8.3).
    for password in ["wrong password", PASSWORD] {
        let (status, headers, body) = login(&app, "alice", password).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(problem_code(&body), "login_locked");
        assert_retry_after(&headers, 30);
    }

    let count: i32 = sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("failed count");
    assert_eq!(count, 5);

    let (action, source): (String, String) = sqlx::query_as(
        "SELECT action, details->>'source' FROM audit_logs
         WHERE action = 'authn_login_lock' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("lock audit");
    assert_eq!(action, "authn_login_lock");
    assert_eq!(source, "login");

    sqlx::query("UPDATE users SET locked_until = now() - interval '1 second' WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("expire lock");

    let (status, _headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let count: i32 = sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("failed count");
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn unknown_identifiers_lock_at_the_account_layer_too(pool: PgPool) {
    let app = app(&pool).await;

    for _ in 0..4 {
        let (status, _headers, body) = login(&app, "ghost", "wrong password").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(problem_code(&body), "invalid_credentials");
    }

    let (status, headers, body) = login(&app, "ghost", "wrong password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "login_locked");
    assert_retry_after(&headers, 30);

    // A locked unknown identifier is refused with a dummy verification and
    // does not accumulate another failure.
    let (status, headers, body) = login(&app, "ghost", "wrong password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "login_locked");
    assert_retry_after(&headers, 30);

    let count: i32 = sqlx::query_scalar("SELECT failed_count FROM auth_login_failures")
        .fetch_one(&pool)
        .await
        .expect("failure count");
    assert_eq!(count, 5);

    // A successful login for a *different* identifier does not clear it.
    insert_user(
        &pool,
        auth_support::NewUser::active("alice", "alice@example.com"),
    )
    .await;
    let (status, _headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let count: i32 = sqlx::query_scalar("SELECT failed_count FROM auth_login_failures")
        .fetch_one(&pool)
        .await
        .expect("failure count");
    assert_eq!(count, 5);
}

#[sqlx::test(migrations = "../../migrations")]
async fn twenty_ip_failures_lock_the_address_and_success_keeps_the_ip_counter(pool: PgPool) {
    let mut settings = auth_settings();
    settings.login_ip_failure_limit = 3;
    let app = app_with(&pool, settings, TrustedProxies::default()).await;
    insert_user(
        &pool,
        auth_support::NewUser::active("alice", "alice@example.com"),
    )
    .await;

    for identifier in ["ghost1", "ghost2"] {
        let (status, _headers, body) = login(&app, identifier, "wrong password").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(problem_code(&body), "invalid_credentials");
    }

    let (status, headers, body) = login(&app, "ghost3", "wrong password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "rate_limited");
    assert_retry_after(&headers, 900);

    let failures: i32 =
        sqlx::query_scalar("SELECT failures FROM auth_login_throttle WHERE scope = 'ip'")
            .fetch_one(&pool)
            .await
            .expect("ip failures");
    assert_eq!(failures, 3);

    sqlx::query(
        "UPDATE auth_login_throttle
         SET locked_until = now() - interval '1 second'
         WHERE scope = 'ip'",
    )
    .execute(&pool)
    .await
    .expect("expire ip lock");

    // A successful login resets only the account layer (spec §8.3).
    let (status, _headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let failures: i32 =
        sqlx::query_scalar("SELECT failures FROM auth_login_throttle WHERE scope = 'ip'")
            .fetch_one(&pool)
            .await
            .expect("ip failures");
    assert_eq!(failures, 3);

    let (status, _headers, body) = login(&app, "ghost4", "wrong password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "rate_limited");
    let failures: i32 =
        sqlx::query_scalar("SELECT failures FROM auth_login_throttle WHERE scope = 'ip'")
            .fetch_one(&pool)
            .await
            .expect("ip failures");
    assert_eq!(failures, 4);

    let (action, scope): (String, String) = sqlx::query_as(
        "SELECT action, details->>'scope' FROM audit_logs
         WHERE action = 'authn_login_rate_limited' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("rate-limit audit");
    assert_eq!(action, "authn_login_rate_limited");
    assert_eq!(scope, "ip");
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_global_circuit_blocks_login_and_reauth_but_not_refresh_or_business_api(pool: PgPool) {
    let mut settings = auth_settings();
    settings.login_ip_failure_limit = 100;
    settings.login_global_alert_limit = 1;
    settings.login_global_circuit_limit = 2;
    settings.login_global_window = Duration::from_secs(300);
    settings.login_global_lock = Duration::from_secs(60);
    let app = app_with(&pool, settings, TrustedProxies::default()).await;
    insert_user(
        &pool,
        auth_support::NewUser::active("alice", "alice@example.com"),
    )
    .await;

    // Establish a session before the circuit opens.
    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["access_token"].as_str().expect("token").to_owned();
    let cookie = refresh_cookie(&headers);

    let (status, _headers, _body) = login(&app, "ghost1", "wrong password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, headers, body) = login(&app, "ghost2", "wrong password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "rate_limited");
    assert_retry_after(&headers, 60);

    // Even valid credentials are refused while the circuit is open.
    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "rate_limited");
    assert_retry_after(&headers, 60);

    // Refresh and an already-authenticated business request are unaffected.
    let (status, _headers, _body) = send(app.clone(), refresh_request(Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _headers, _body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Reauthentication is covered by the circuit.
    let (status, headers, body) = reauthenticate(&app, &token, PASSWORD).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "rate_limited");
    assert_retry_after(&headers, 60);

    let scope: String = sqlx::query_scalar(
        "SELECT details->>'scope' FROM audit_logs
         WHERE action = 'authn_login_rate_limited' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("rate-limit audit");
    assert_eq!(scope, "global");
}

#[sqlx::test(migrations = "../../migrations")]
async fn untrusted_peers_cannot_spoof_forwarded_for_and_audit_uses_the_resolved_ip(pool: PgPool) {
    let mut settings = auth_settings();
    settings.login_ip_failure_limit = 2;
    let proxies = TrustedProxies::new(vec!["10.0.0.0/8".parse().expect("trusted proxy cidr")]);
    let app = app_with(&pool, settings, proxies).await;
    insert_user(
        &pool,
        auth_support::NewUser::active("alice", "alice@example.com"),
    )
    .await;

    let proxy: SocketAddr = "10.0.0.5:40000".parse().expect("peer");
    let direct: SocketAddr = "203.0.113.9:40000".parse().expect("peer");
    let forwarded = [("x-forwarded-for", "198.51.100.7")];

    let (status, _headers, _body) =
        login_from(&app, "ghost1", "wrong password", proxy, &forwarded).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, headers, body) =
        login_from(&app, "ghost2", "wrong password", proxy, &forwarded).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "rate_limited");
    assert_retry_after(&headers, 900);

    // A different forwarded address is a different throttle bucket.
    let (status, _headers, _body) = login_from(
        &app,
        "ghost3",
        "wrong password",
        proxy,
        &[("x-forwarded-for", "198.51.100.8")],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The same spoofed header from an untrusted peer is ignored.
    let (status, _headers, _body) =
        login_from(&app, "ghost4", "wrong password", direct, &forwarded).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let audited_ip: String = sqlx::query_scalar(
        "SELECT host(ip) FROM audit_logs
         WHERE action = 'authn_login_fail' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("audit ip");
    assert_eq!(audited_ip, "203.0.113.9");

    // A malformed chain from a trusted peer falls back to the peer address.
    let (status, _headers, _body) = login_from(
        &app,
        "ghost5",
        "wrong password",
        proxy,
        &[("x-forwarded-for", "not-an-ip")],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let audited_ip: String = sqlx::query_scalar(
        "SELECT host(ip) FROM audit_logs
         WHERE action = 'authn_login_fail' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("audit ip");
    assert_eq!(audited_ip, "10.0.0.5");

    // The forwarded address of a trusted request is what gets audited.
    let (status, _headers, _body) = login_from(
        &app,
        "ghost6",
        "wrong password",
        proxy,
        &[("x-forwarded-for", "198.51.100.9")],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let audited_ip: String = sqlx::query_scalar(
        "SELECT host(ip) FROM audit_logs
         WHERE action = 'authn_login_fail' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("audit ip");
    assert_eq!(audited_ip, "198.51.100.9");
}

#[sqlx::test(migrations = "../../migrations")]
async fn reauthentication_failures_count_toward_the_account_layer(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(
        &pool,
        auth_support::NewUser::active("alice", "alice@example.com"),
    )
    .await;
    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["access_token"].as_str().expect("token").to_owned();

    for _ in 0..4 {
        let (status, _headers, body) = reauthenticate(&app, &token, "wrong password").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem_code(&body), "invalid_current_password");
    }

    let (status, headers, body) = reauthenticate(&app, &token, "wrong password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&body), "login_locked");
    assert_retry_after(&headers, 30);

    let count: i32 = sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("failed count");
    assert_eq!(count, 5);

    let source: String = sqlx::query_scalar(
        "SELECT details->>'source' FROM audit_logs
         WHERE action = 'authn_login_lock' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("lock audit");
    assert_eq!(source, "reauth");

    sqlx::query("UPDATE users SET locked_until = now() - interval '1 second' WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("expire lock");

    let (status, _headers, _body) = reauthenticate(&app, &token, PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let count: i32 = sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("failed count");
    assert_eq!(count, 0);
}
