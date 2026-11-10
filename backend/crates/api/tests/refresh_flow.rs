//! Refresh rotation, grace-window retries, replay detection and session expiry
//! over HTTP with a real PostgreSQL schema (docs/specs/login-system.md §5.2–§5.3).

mod auth_support;

use auth_support::{
    NewUser, PASSWORD, app, insert_user, json, login, problem_code, refresh_cookie,
    refresh_request, refresh_token_rows, send, token_verifier,
};
use axum::http::{StatusCode, header};
use chrono::{DateTime, Utc};
use metrics_exporter_prometheus::PrometheusHandle;
use rs_admin_api::install_metrics_recorder;
use rs_admin_security::hash_refresh_token;
use sqlx::PgPool;
use std::sync::OnceLock;
use uuid::Uuid;

fn recorder() -> &'static PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| install_metrics_recorder().expect("install recorder"))
}

#[derive(sqlx::FromRow)]
struct StoredToken {
    token_hash: Vec<u8>,
    rotation_counter: i64,
    used_at: Option<DateTime<Utc>>,
    replaced_by_hash: Option<Vec<u8>>,
}

async fn audit_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(pool)
        .await
        .expect("audit count")
}

async fn session_id(pool: &PgPool, user_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM auth_sessions WHERE user_id = $1 ORDER BY id DESC LIMIT 1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("session")
}

#[sqlx::test(migrations = "../../migrations")]
async fn refresh_rotates_the_token_and_issues_a_new_access_token(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, login_body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{login_body}");
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;
    let audit_before = audit_count(&pool).await;

    let (status, headers, body) = send(app.clone(), refresh_request(Some(&original))).await;
    let body = json(&body);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 900);

    let rotated = refresh_cookie(&headers);
    assert_ne!(rotated, original, "rotation must issue a new raw token");
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("Set-Cookie");
    for attribute in [
        "Path=/",
        "Secure",
        "HttpOnly",
        "SameSite=Strict",
        "Max-Age=",
    ] {
        assert!(cookie.contains(attribute), "missing {attribute}: {cookie}");
    }
    assert!(
        !cookie.contains("Domain"),
        "Domain must be omitted: {cookie}"
    );
    let max_age: i64 = cookie
        .split("Max-Age=")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .and_then(|value| value.trim().parse().ok())
        .expect("numeric Max-Age");
    assert!(
        (1..=43_200).contains(&max_age),
        "Max-Age is capped by the session absolute lifetime: {max_age}"
    );

    let claims = token_verifier()
        .verify(body["access_token"].as_str().expect("access token"))
        .expect("verify access token");
    let (authenticated_at, absolute_expires_at): (DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
        "SELECT authenticated_at, absolute_expires_at FROM auth_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("session times");
    assert_eq!(claims.sub, user_id);
    assert_eq!(claims.sid, session_id);
    assert_eq!(claims.ver, 0);
    assert_eq!(claims.auth_time, authenticated_at.timestamp());
    assert_eq!(claims.exp - claims.iat, 900);
    assert!(absolute_expires_at > Utc::now());

    // The database holds only SHA-256 digests of each raw token and the
    // replaced-by pointer to the successor.
    let rows: Vec<StoredToken> = sqlx::query_as(
        "SELECT token_hash, rotation_counter, used_at, replaced_by_hash
         FROM auth_refresh_tokens WHERE session_id = $1 ORDER BY rotation_counter",
    )
    .bind(session_id)
    .fetch_all(&pool)
    .await
    .expect("refresh token rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].token_hash, hash_refresh_token(&original).to_vec());
    assert!(
        rows[0].used_at.is_some(),
        "the old token must be marked used"
    );
    assert_eq!(
        rows[0].replaced_by_hash.as_deref(),
        Some(hash_refresh_token(&rotated).as_slice()),
        "the old row must point at its successor"
    );
    assert_eq!(rows[1].token_hash, hash_refresh_token(&rotated).to_vec());
    assert_eq!(rows[1].rotation_counter, 1);
    assert!(rows[1].used_at.is_none(), "the successor must stay unused");
    for raw in [&original, &rotated] {
        assert!(
            !rows.iter().any(|row| row.token_hash == raw.as_bytes()),
            "raw tokens must never be persisted"
        );
    }

    assert_eq!(
        audit_count(&pool).await,
        audit_before,
        "a successful refresh does not write an audit event"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_retry_within_grace_returns_the_same_successor_without_a_rotation(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;

    let (status, headers, _body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK);
    let successor = refresh_cookie(&headers);
    let audit_before = audit_count(&pool).await;
    let used_before: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT used_at FROM auth_refresh_tokens WHERE token_hash = $1")
            .bind(hash_refresh_token(&original).to_vec())
            .fetch_one(&pool)
            .await
            .expect("used_at");

    // A retry inside the 30-second window returns the exact same successor
    // and performs no write at all.
    let (status, headers, body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    assert_eq!(refresh_cookie(&headers), successor);
    assert_eq!(refresh_token_rows(&pool, session_id).await, 2);
    assert_eq!(audit_count(&pool).await, audit_before);
    let used_after: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT used_at FROM auth_refresh_tokens WHERE token_hash = $1")
            .bind(hash_refresh_token(&original).to_vec())
            .fetch_one(&pool)
            .await
            .expect("used_at");
    assert_eq!(used_after, used_before, "grace must not re-stamp used_at");

    // Once the successor has itself rotated, an in-grace retry of the original
    // walks the chain and returns the current tail.
    let (status, headers, _body) = send(app.clone(), refresh_request(Some(&successor))).await;
    assert_eq!(status, StatusCode::OK);
    let tail = refresh_cookie(&headers);
    assert_ne!(tail, successor);
    assert_eq!(refresh_token_rows(&pool, session_id).await, 3);

    let (status, headers, _body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        refresh_cookie(&headers),
        tail,
        "the grace chain must return the current valid token"
    );
    assert_eq!(refresh_token_rows(&pool, session_id).await, 3);
    assert_eq!(audit_count(&pool).await, audit_before);
}

#[sqlx::test(migrations = "../../migrations")]
async fn reuse_outside_grace_revokes_the_family_and_writes_a_critical_audit(pool: PgPool) {
    let handle = recorder();
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;

    let (status, headers, _body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK);
    let successor = refresh_cookie(&headers);

    // Move the old token's rotation outside the 30-second grace window.
    sqlx::query(
        "UPDATE auth_refresh_tokens SET used_at = now() - interval '31 seconds'
         WHERE token_hash = $1",
    )
    .bind(hash_refresh_token(&original).to_vec())
    .execute(&pool)
    .await
    .expect("age rotation");

    let (status, headers, body) = send(app.clone(), refresh_request(Some(&original))).await;
    let body = json(&body);
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(problem_code(&body), "refresh_reuse_detected");
    assert!(
        handle
            .render()
            .contains("auth_refresh_reuse_detected_total"),
        "reuse detection must increment its counter"
    );
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("clearing Set-Cookie");
    assert!(cookie.starts_with("__Host-refresh=;"), "{cookie}");
    assert!(cookie.contains("Max-Age=0"), "{cookie}");

    // The entire family is revoked, not just the presented token.
    let tokens: Vec<(Option<DateTime<Utc>>, Option<String>)> = sqlx::query_as(
        "SELECT revoked_at, revoked_reason FROM auth_refresh_tokens WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_all(&pool)
    .await
    .expect("token rows");
    assert_eq!(tokens.len(), 2);
    for (revoked_at, reason) in tokens {
        assert!(revoked_at.is_some(), "every token in the family is revoked");
        assert_eq!(reason.as_deref(), Some("token_reuse"));
    }

    let (revoked_at, revoke_reason): (Option<DateTime<Utc>>, Option<String>) =
        sqlx::query_as("SELECT revoked_at, revoke_reason FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("session row");
    assert!(revoked_at.is_some(), "the session itself is revoked");
    assert_eq!(revoke_reason.as_deref(), Some("token_reuse"));

    let token_version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(
        token_version, 0,
        "reuse detection must not bump the version"
    );

    let (action, severity, result, target_type, target_id, actor_id, details): (
        String,
        String,
        String,
        String,
        Option<Uuid>,
        Option<Uuid>,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT action, severity, result, target_type, target_id, actor_user_id, details
         FROM audit_logs ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("audit row");
    assert_eq!(action, "authn_refresh_reuse_detected");
    assert_eq!(severity, "critical");
    assert_eq!(result, "failure");
    assert_eq!(target_type, "session");
    assert_eq!(target_id, Some(session_id));
    assert_eq!(actor_id, Some(user_id));
    assert_eq!(details["session_id"], session_id.to_string());
    assert_eq!(details["rotation_counter"], 0);
    assert_eq!(details["family_revoked"], true);

    // The successor is dead too: its session is gone.
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&successor))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_broken_grace_chain_is_treated_as_reuse(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;

    let (status, headers, _body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK);
    let successor = refresh_cookie(&headers);

    // Revoke the successor row while the session is still active; a grace
    // retry of the original must not treat the chain as healthy.
    sqlx::query(
        "UPDATE auth_refresh_tokens SET revoked_at = now(), revoked_reason = 'logout'
         WHERE token_hash = $1",
    )
    .bind(hash_refresh_token(&successor).to_vec())
    .execute(&pool)
    .await
    .expect("revoke successor");

    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "refresh_reuse_detected");

    let (revoked_at, revoke_reason): (Option<DateTime<Utc>>, Option<String>) =
        sqlx::query_as("SELECT revoked_at, revoke_reason FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("session row");
    assert!(revoked_at.is_some());
    assert_eq!(revoke_reason.as_deref(), Some("token_reuse"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn refresh_refuses_a_disabled_account_without_rotating(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;

    // Disabling also revokes sessions in the full lifecycle; the refresh
    // endpoint must still prefer 403 account_disabled over 401 session_invalid.
    sqlx::query("UPDATE users SET status = 'disabled' WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("disable");
    sqlx::query(
        "UPDATE auth_sessions SET revoked_at = now(), revoke_reason = 'account_disabled'
         WHERE id = $1",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("revoke session");

    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&json(&body)), "account_disabled");

    let used_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT used_at FROM auth_refresh_tokens WHERE token_hash = $1")
            .bind(hash_refresh_token(&original).to_vec())
            .fetch_one(&pool)
            .await
            .expect("used_at");
    assert!(used_at.is_none(), "a refused refresh must not rotate");
}

#[sqlx::test(migrations = "../../migrations")]
async fn refresh_rejects_missing_unknown_and_revoked_tokens(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, _headers, body) = send(app.clone(), refresh_request(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "refresh_token_invalid");

    let (status, _headers, body) = send(app.clone(), refresh_request(Some("unknown-token"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "refresh_token_invalid");

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let raw = refresh_cookie(&login_headers);
    let first_session_id = session_id(&pool, user_id).await;

    sqlx::query(
        "UPDATE auth_refresh_tokens SET revoked_at = now(), revoked_reason = 'logout'
         WHERE session_id = $1",
    )
    .bind(first_session_id)
    .execute(&pool)
    .await
    .expect("revoke token");
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&raw))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "refresh_token_invalid");

    // A revoked session masks any token state as session_invalid.
    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let raw = refresh_cookie(&login_headers);
    let renewed_session_id = session_id(&pool, user_id).await;
    sqlx::query(
        "UPDATE auth_sessions SET revoked_at = now(), revoke_reason = 'logout' WHERE id = $1",
    )
    .bind(renewed_session_id)
    .execute(&pool)
    .await
    .expect("revoke session");
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&raw))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");
}

#[sqlx::test(migrations = "../../migrations")]
async fn refresh_enforces_idle_and_absolute_session_expiry(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;

    sqlx::query(
        "UPDATE auth_sessions SET idle_expires_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("expire idle");
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");

    sqlx::query(
        "UPDATE auth_sessions
         SET idle_expires_at = now() + interval '30 minutes',
             absolute_expires_at = now() - interval '1 second'
         WHERE id = $1",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("expire absolute");
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");
}

#[sqlx::test(migrations = "../../migrations")]
async fn refresh_updates_activity_at_most_once_per_minute(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;
    let (last_seen_before, idle_before, absolute_before): (
        DateTime<Utc>,
        DateTime<Utc>,
        DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT last_seen_at, idle_expires_at, absolute_expires_at
         FROM auth_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("session times");

    // A refresh right after login is inside the 60-second throttle window.
    let (status, headers, _body) = send(app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK);
    let successor = refresh_cookie(&headers);
    let (last_seen, idle, absolute): (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as(
            "SELECT last_seen_at, idle_expires_at, absolute_expires_at
             FROM auth_sessions WHERE id = $1",
        )
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("session times");
    assert_eq!(last_seen, last_seen_before);
    assert_eq!(idle, idle_before);
    assert_eq!(absolute, absolute_before);

    // Once the throttle window has elapsed, activity slides the idle
    // deadline but never the absolute deadline.
    sqlx::query(
        "UPDATE auth_sessions
         SET last_seen_at = now() - interval '61 seconds',
             idle_expires_at = now() + interval '5 minutes'
         WHERE id = $1",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("age activity");

    let (status, headers, _body) = send(app.clone(), refresh_request(Some(&successor))).await;
    assert_eq!(status, StatusCode::OK);
    let rotated = refresh_cookie(&headers);
    let (last_seen, idle, absolute): (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as(
            "SELECT last_seen_at, idle_expires_at, absolute_expires_at
             FROM auth_sessions WHERE id = $1",
        )
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("session times");
    assert!(
        (Utc::now() - last_seen).num_seconds().abs() <= 2,
        "last_seen_at is stamped with the database clock: {last_seen}"
    );
    assert!(
        (idle - last_seen).num_seconds() == 1800,
        "idle expiry slides to 30 minutes from the touch: {idle}"
    );
    assert_eq!(absolute, absolute_before);
    assert_ne!(rotated, successor);
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_refreshes_share_one_rotation(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;

    let (first, second) = tokio::join!(
        send(app.clone(), refresh_request(Some(&original))),
        send(app.clone(), refresh_request(Some(&original))),
    );

    assert_eq!(
        first.0,
        StatusCode::OK,
        "first refresh: {:?}",
        json(&first.2)
    );
    assert_eq!(
        second.0,
        StatusCode::OK,
        "concurrent retry must use the grace window: {:?}",
        json(&second.2)
    );
    let first_successor = refresh_cookie(&first.1);
    let second_successor = refresh_cookie(&second.1);
    assert_eq!(
        first_successor, second_successor,
        "both concurrent callers must receive the same successor"
    );
    assert_eq!(
        refresh_token_rows(&pool, session_id).await,
        2,
        "the family must contain one rotation, not a branch"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn grace_retries_survive_a_derivation_key_rotation(pool: PgPool) {
    use rs_admin_api::{AppState, AuthServices, build_router};
    use rs_admin_security::{HmacKey, TrustedProxies};
    use rs_admin_store::Database;

    rs_admin_store::rbac::sync_catalog(&pool)
        .await
        .expect("catalog sync");
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    // The first process rotates a token with its current derivation key.
    let old_config = auth_support::security_config();
    let old_app = build_router(
        AppState::with_auth(
            Database::from_pool(pool.clone()),
            AuthServices::with_password_service(
                &old_config,
                auth_support::password_service().clone(),
                auth_support::auth_settings(),
                TrustedProxies::default(),
            )
            .expect("old services"),
        )
        .with_allowed_origins(auth_support::allowed_origins()),
    );
    let (status, login_headers, _body) = login(&old_app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let (status, headers, _body) = send(old_app.clone(), refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK);
    let successor = refresh_cookie(&headers);
    let session_id = session_id(&pool, user_id).await;

    // A redeploy moves the old key to `previous`; the in-grace retry must
    // still recompute the same successor.
    let mut new_config = auth_support::security_config();
    new_config.refresh_derivation.current = HmacKey::from_bytes(vec![9u8; 32]).expect("new key");
    new_config.refresh_derivation.previous =
        Some(auth_support::security_config().refresh_derivation.current);
    let new_app = build_router(
        AppState::with_auth(
            Database::from_pool(pool.clone()),
            AuthServices::with_password_service(
                &new_config,
                auth_support::password_service().clone(),
                auth_support::auth_settings(),
                TrustedProxies::default(),
            )
            .expect("new services"),
        )
        .with_allowed_origins(auth_support::allowed_origins()),
    );

    let (status, headers, body) = send(new_app, refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    assert_eq!(refresh_cookie(&headers), successor);
    assert_eq!(refresh_token_rows(&pool, session_id).await, 2);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_long_in_grace_chain_returns_its_tail_instead_of_replay(pool: PgPool) {
    use rs_admin_security::derive_refresh_token;
    use rs_admin_store::auth::{
        NewRefreshToken, database_clock_now, insert_refresh_token, mark_refresh_token_rotated,
    };

    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let original = refresh_cookie(&login_headers);
    let session_id = session_id(&pool, user_id).await;
    let absolute_expires_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT absolute_expires_at FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("absolute expiry");

    // Build a healthy chain longer than any fixed defensive bound. All
    // rotations happen "now", so the presented token stays inside its grace
    // window while the chain is walked.
    let key = auth_support::security_config().refresh_derivation.current;
    let mut transaction = pool.begin().await.expect("begin");
    let now = database_clock_now(&mut transaction)
        .await
        .expect("database clock");
    let mut current_raw = original.clone();
    let mut tail = original.clone();
    for counter in 1..=100 {
        let next_raw = derive_refresh_token(&key, &current_raw, counter);
        let next_hash = hash_refresh_token(&next_raw);
        mark_refresh_token_rotated(
            &mut transaction,
            &hash_refresh_token(&current_raw),
            &next_hash,
            now,
        )
        .await
        .expect("rotate");
        insert_refresh_token(
            &mut transaction,
            NewRefreshToken {
                token_hash: &next_hash,
                session_id,
                rotation_counter: counter,
                expires_at: absolute_expires_at,
            },
        )
        .await
        .expect("insert successor");
        current_raw = next_raw.clone();
        tail = next_raw;
    }
    transaction.commit().await.expect("commit chain");

    let (status, headers, body) = send(app, refresh_request(Some(&original))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    assert_eq!(
        refresh_cookie(&headers),
        tail,
        "an intact in-grace chain must return the current tail, not replay"
    );
}
