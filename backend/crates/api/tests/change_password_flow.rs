//! Self-service password change and the temporary-password restricted session
//! over HTTP with a real PostgreSQL schema (docs/specs/login-system.md §5.4,
//! §5.7, §5.9, §8.2).

mod auth_support;

use auth_support::{
    NewUser, PASSWORD, app, authorized_request, change_password_request, insert_user, json, login,
    password_service, problem_code, refresh_cookie, refresh_request, send, token_verifier,
};
use axum::http::{StatusCode, header};
use chrono::{DateTime, Utc};
use rs_admin_audit::verify_chain;
use rs_admin_security::hash_refresh_token;
use sqlx::PgPool;
use uuid::Uuid;

const NEW_PASSWORD: &str = "new correct horse battery staple";

#[derive(sqlx::FromRow)]
struct StoredToken {
    token_hash: Vec<u8>,
    rotation_counter: i64,
    used_at: Option<DateTime<Utc>>,
    replaced_by_hash: Option<Vec<u8>>,
    revoked_at: Option<DateTime<Utc>>,
}

async fn session_state(pool: &PgPool, session_id: Uuid) -> (bool, Option<String>) {
    let (revoked_at, revoke_reason): (Option<DateTime<Utc>>, Option<String>) =
        sqlx::query_as("SELECT revoked_at, revoke_reason FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .expect("session state");
    (revoked_at.is_some(), revoke_reason)
}

#[sqlx::test(migrations = "../../migrations")]
async fn change_password_keeps_the_current_session_and_revokes_the_others(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, first_headers, first_body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{first_body}");
    let original_cookie = refresh_cookie(&first_headers);
    let first_token = first_body["access_token"]
        .as_str()
        .expect("first access token")
        .to_owned();
    let first_sid = token_verifier()
        .verify(&first_token)
        .expect("first claims")
        .sid;

    let (status, second_headers, second_body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{second_body}");
    let second_cookie = refresh_cookie(&second_headers);
    let second_token = second_body["access_token"]
        .as_str()
        .expect("second access token")
        .to_owned();
    let second_sid = token_verifier()
        .verify(&second_token)
        .expect("second claims")
        .sid;
    assert_ne!(first_sid, second_sid);

    let (status, headers, body) = send(
        app.clone(),
        change_password_request(&first_token, Some(&original_cookie), PASSWORD, NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let body = json(&body);
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 900);

    // The current device receives a rotated refresh cookie.
    let rotated_cookie = refresh_cookie(&headers);
    assert_ne!(
        rotated_cookie, original_cookie,
        "the refresh token must rotate"
    );
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("rotated Set-Cookie");
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

    // The user row no longer requires a change and carries a new version.
    let (token_version, must_change, temporary_expires): (i64, bool, Option<DateTime<Utc>>) =
        sqlx::query_as(
            "SELECT token_version, must_change_password, temporary_password_expires_at
         FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("user state");
    assert_eq!(token_version, 1);
    assert!(!must_change);
    assert!(temporary_expires.is_none());

    let stored_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("password hash");
    assert!(
        password_service()
            .verify(NEW_PASSWORD, &stored_hash)
            .await
            .expect("verify new password")
            .valid
    );
    assert!(
        !password_service()
            .verify(PASSWORD, &stored_hash)
            .await
            .expect("verify old password")
            .valid
    );

    // The current session survives; every other session is revoked with the
    // password-change reason, including its refresh family.
    let (current_revoked, _) = session_state(&pool, first_sid).await;
    assert!(!current_revoked, "the current session must stay active");
    let (other_revoked, other_reason) = session_state(&pool, second_sid).await;
    assert!(other_revoked, "other sessions must be revoked");
    assert_eq!(other_reason.as_deref(), Some("password_change"));
    let other_tokens: Vec<(Option<DateTime<Utc>>, Option<String>)> = sqlx::query_as(
        "SELECT revoked_at, revoked_reason FROM auth_refresh_tokens WHERE session_id = $1",
    )
    .bind(second_sid)
    .fetch_all(&pool)
    .await
    .expect("other token rows");
    assert!(!other_tokens.is_empty());
    for (revoked_at, reason) in other_tokens {
        assert!(revoked_at.is_some(), "the whole family is revoked");
        assert_eq!(reason.as_deref(), Some("password_change"));
    }

    // The current session rotated its token exactly once.
    let rows: Vec<StoredToken> = sqlx::query_as(
        "SELECT token_hash, rotation_counter, used_at, replaced_by_hash, revoked_at
         FROM auth_refresh_tokens WHERE session_id = $1 ORDER BY rotation_counter",
    )
    .bind(first_sid)
    .fetch_all(&pool)
    .await
    .expect("current token rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].token_hash,
        hash_refresh_token(&original_cookie).to_vec()
    );
    assert!(rows[0].used_at.is_some(), "the old token must be used");
    assert_eq!(
        rows[0].replaced_by_hash.as_deref(),
        Some(hash_refresh_token(&rotated_cookie).as_slice()),
        "the old row must point at its successor"
    );
    assert_eq!(
        rows[1].token_hash,
        hash_refresh_token(&rotated_cookie).to_vec()
    );
    assert_eq!(rows[1].rotation_counter, 1);
    assert!(rows[1].used_at.is_none());
    assert!(rows[1].revoked_at.is_none());

    // The response access token is signed for the same session with the new
    // token version and the refreshed authentication time.
    let new_token = body["access_token"].as_str().expect("new access token");
    let claims = token_verifier().verify(new_token).expect("new claims");
    let (authenticated_at, idle_expires_at, absolute_expires_at): (
        DateTime<Utc>,
        DateTime<Utc>,
        DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT authenticated_at, idle_expires_at, absolute_expires_at
         FROM auth_sessions WHERE id = $1",
    )
    .bind(first_sid)
    .fetch_one(&pool)
    .await
    .expect("session times");
    assert_eq!(claims.sub, user_id);
    assert_eq!(claims.sid, first_sid);
    assert_eq!(claims.ver, 1);
    assert_eq!(claims.auth_time, authenticated_at.timestamp());
    assert!(idle_expires_at > Utc::now());
    assert!(absolute_expires_at > Utc::now());

    // Old access tokens on every device stop working on the next request.
    for token in [&first_token, &second_token] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request("GET", "/api/v1/auth/me", token, None),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{:?}", json(&body));
        assert_eq!(problem_code(&json(&body)), "session_invalid");
    }
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", new_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    assert_eq!(json(&body)["must_change_password"], false);

    // The rotated cookie refreshes; the revoked device cannot.
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&rotated_cookie))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    let (status, _headers, body) = send(app, refresh_request(Some(&second_cookie))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");

    // One info-level audit event records the count of ended sessions.
    let audit: (
        String,
        String,
        String,
        String,
        Option<Uuid>,
        Option<Uuid>,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT action, severity, result, target_type, target_id, actor_user_id, details
             FROM audit_logs WHERE action = 'authn_password_changed'",
    )
    .fetch_one(&pool)
    .await
    .expect("password change audit");
    assert_eq!(audit.0, "authn_password_changed");
    assert_eq!(audit.1, "info");
    assert_eq!(audit.2, "success");
    assert_eq!(audit.3, "user");
    assert_eq!(audit.4, Some(user_id));
    assert_eq!(audit.5, Some(user_id));
    assert_eq!(audit.6["sessions_revoked_count"], 1);

    let verification = verify_chain(&pool, &auth_support::security_config().audit_chain.keys)
        .await
        .expect("audit chain must verify");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .expect("audit count");
    assert_eq!(verification.verified_rows, stored as u64);
}

#[sqlx::test(migrations = "../../migrations")]
async fn change_password_rotates_the_session_token_without_a_presented_cookie(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let original_cookie = refresh_cookie(&headers);
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let session_id = token_verifier().verify(&token).expect("claims").sid;

    let (status, headers, body) = send(
        app.clone(),
        change_password_request(&token, None, PASSWORD, NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));

    let rotated_cookie = refresh_cookie(&headers);
    assert_ne!(rotated_cookie, original_cookie);
    let (revoked, _) = session_state(&pool, session_id).await;
    assert!(!revoked, "the current session is kept");

    // The replaced family is retired; stale values fail as invalid without
    // being classified as replay.
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&original_cookie))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "refresh_token_invalid");
    let (status, _headers, body) = send(app, refresh_request(Some(&rotated_cookie))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
}

#[sqlx::test(migrations = "../../migrations")]
async fn change_password_follows_a_token_rotated_by_a_concurrent_refresh(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let login_cookie = refresh_cookie(&headers);
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();

    // Another tab refreshes first; the cookie this request carries is now a
    // used token inside its grace window.
    let (status, headers, _body) = send(app.clone(), refresh_request(Some(&login_cookie))).await;
    assert_eq!(status, StatusCode::OK);
    let concurrent_cookie = refresh_cookie(&headers);

    let (status, headers, body) = send(
        app.clone(),
        change_password_request(&token, Some(&login_cookie), PASSWORD, NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    let rotated_cookie = refresh_cookie(&headers);
    assert_ne!(rotated_cookie, concurrent_cookie);

    // The grace retry with the concurrently issued token still resolves to
    // the current tail instead of being classified as replay.
    let (status, _headers, body) =
        send(app.clone(), refresh_request(Some(&concurrent_cookie))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    let (status, _headers, body) = send(app, refresh_request(Some(&rotated_cookie))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_wrong_current_password_counts_the_failure_and_writes_an_audit(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let session_id = token_verifier().verify(&token).expect("claims").sid;

    let (status, _headers, body) = send(
        app,
        change_password_request(&token, None, "wrong password", NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(problem_code(&json(&body)), "invalid_current_password");

    // Nothing about the credential or the sessions changed.
    let stored_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("password hash");
    assert!(
        password_service()
            .verify(PASSWORD, &stored_hash)
            .await
            .expect("verify password")
            .valid
    );
    let token_version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(token_version, 0);
    let (revoked, _) = session_state(&pool, session_id).await;
    assert!(!revoked, "a failed attempt must not revoke the session");

    // The account and IP layers count the failure; the global circuit is
    // deliberately untouched by password changes (spec §8.3).
    let failed_count: i32 =
        sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("account counter");
    assert_eq!(failed_count, 1);
    let ip_failures: i32 =
        sqlx::query_scalar("SELECT failures FROM auth_login_throttle WHERE scope = 'ip'")
            .fetch_one(&pool)
            .await
            .expect("ip counter");
    assert_eq!(ip_failures, 1);
    let global_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM auth_login_throttle WHERE scope = 'global'")
            .fetch_one(&pool)
            .await
            .expect("global rows");
    assert_eq!(global_rows, 0);

    let audit: (
        String,
        String,
        String,
        String,
        Option<Uuid>,
        Option<Uuid>,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT action, severity, result, target_type, target_id, actor_user_id, details
             FROM audit_logs WHERE action = 'authn_password_change_fail'",
    )
    .fetch_one(&pool)
    .await
    .expect("failure audit");
    assert_eq!(audit.0, "authn_password_change_fail");
    assert_eq!(audit.1, "warning");
    assert_eq!(audit.2, "failure");
    assert_eq!(audit.3, "user");
    assert_eq!(audit.4, Some(user_id));
    assert_eq!(audit.5, Some(user_id));
    assert_eq!(audit.6["reason_code"], "bad_credentials");

    let changed_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs WHERE action = 'authn_password_changed'",
    )
    .fetch_one(&pool)
    .await
    .expect("changed events");
    assert_eq!(changed_events, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn policy_failures_and_an_unchanged_password_are_rejected(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let session_id = token_verifier().verify(&token).expect("claims").sid;
    let too_long = "a".repeat(129);

    let cases: [(&str, &str, &str, Option<&str>); 5] = [
        ("unchanged", PASSWORD, "password_unchanged", None),
        (
            "too short",
            "short",
            "password_policy_violation",
            Some("too_short"),
        ),
        (
            "equals identifier",
            "alice@example.com",
            "password_policy_violation",
            Some("equals_identifier"),
        ),
        (
            "blocklisted",
            "demon1q2w3e4r5t",
            "password_policy_violation",
            Some("blocklisted"),
        ),
        (
            "too long",
            &too_long,
            "password_policy_violation",
            Some("too_long"),
        ),
    ];

    for (label, new_password, expected_code, expected_reason) in cases {
        let (status, _headers, body) = send(
            app.clone(),
            change_password_request(&token, None, PASSWORD, new_password),
        )
        .await;
        let body = json(&body);
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{label}: {body}");
        assert_eq!(problem_code(&body), expected_code, "{label}");
        if let Some(reason) = expected_reason {
            assert_eq!(body["errors"][0]["pointer"], "/new_password", "{label}");
            assert_eq!(
                body["errors"][0]["code"], "password_policy_violation",
                "{label}"
            );
            assert_eq!(body["errors"][0]["params"]["reason"], reason, "{label}");
        }
    }

    // Neither the password nor any session changed, and these semantic
    // refusals do not feed the failure counters.
    let stored_hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("password hash");
    assert!(
        password_service()
            .verify(PASSWORD, &stored_hash)
            .await
            .expect("verify password")
            .valid
    );
    let (token_version, failed_count): (i64, i32) =
        sqlx::query_as("SELECT token_version, failed_login_count FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("user state");
    assert_eq!(token_version, 0);
    assert_eq!(failed_count, 0);
    let (revoked, _) = session_state(&pool, session_id).await;
    assert!(!revoked);
    let failure_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs WHERE action = 'authn_password_change_fail'",
    )
    .fetch_one(&pool)
    .await
    .expect("failure events");
    assert_eq!(failure_events, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn repeated_wrong_current_passwords_lock_the_account(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();

    for _ in 0..4 {
        let (status, _headers, body) = send(
            app.clone(),
            change_password_request(&token, None, "wrong password", NEW_PASSWORD),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem_code(&json(&body)), "invalid_current_password");
    }

    let (status, headers, body) = send(
        app.clone(),
        change_password_request(&token, None, "wrong password", NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&json(&body)), "login_locked");
    let retry_after: i64 = headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .expect("Retry-After")
        .parse()
        .expect("numeric Retry-After");
    assert!((1..=30).contains(&retry_after));

    // The lock also refuses a correct current password until it expires.
    let (status, _headers, body) = send(
        app,
        change_password_request(&token, None, PASSWORD, NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(problem_code(&json(&body)), "login_locked");

    let source: String = sqlx::query_scalar(
        "SELECT details->>'source' FROM audit_logs
         WHERE action = 'authn_login_lock' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("lock audit");
    assert_eq!(source, "change_password");
    let count: i32 = sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("failed count");
    assert_eq!(count, 5);
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_existing_restricted_session_can_change_an_expired_temporary_password(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::restricted("alice", "alice@example.com")).await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["must_change_password"], true);
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let cookie = refresh_cookie(&headers);

    // A temporary password expiring only blocks new logins; the session that
    // already exists may still change it (ADR-0003).
    sqlx::query(
        "UPDATE users SET temporary_password_expires_at = now() - interval '1 hour'
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("expire temporary password");

    let (status, headers, body) = send(
        app.clone(),
        change_password_request(&token, Some(&cookie), PASSWORD, NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    let rotated_cookie = refresh_cookie(&headers);
    assert_ne!(rotated_cookie, cookie);

    let (must_change, temporary_expires): (bool, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT must_change_password, temporary_password_expires_at FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("user state");
    assert!(!must_change, "the temporary-password state must clear");
    assert!(temporary_expires.is_none());

    // The session regained full access.
    let new_token = json(&body)["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let (status, _headers, body) = send(
        app,
        authorized_request("GET", "/api/v1/auth/me", &new_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    assert_eq!(json(&body)["must_change_password"], false);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_restricted_session_is_limited_to_the_allowlist(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::restricted("alice", "alice@example.com")).await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let cookie = refresh_cookie(&headers);

    // GET /auth/me is allowed and reports the restriction.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    assert_eq!(json(&body)["must_change_password"], true);

    // POST /auth/refresh is allowed.
    let (status, headers, body) = send(app.clone(), refresh_request(Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    let cookie = refresh_cookie(&headers);

    // Every endpoint outside the allowlist returns 403
    // `password_change_required`.
    for (uri, payload) in [
        (
            "/api/v1/auth/reauthenticate",
            serde_json::json!({ "password": PASSWORD }),
        ),
        ("/api/v1/auth/logout-all", serde_json::json!({})),
    ] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request("POST", uri, &token, Some(&payload)),
        )
        .await;
        let body = json(&body);
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {body}");
        assert_eq!(problem_code(&body), "password_change_required", "{uri}");
    }

    // POST /auth/change-password is allowed.
    let (status, _headers, body) = send(
        app.clone(),
        change_password_request(&token, None, "wrong password", NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(problem_code(&json(&body)), "invalid_current_password");

    // POST /auth/logout is allowed and ends the session.
    let (status, _headers, body) =
        send(app.clone(), auth_support::logout_request(Some(&cookie))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{:?}", json(&body));
    let (revoked, reason) =
        session_state(&pool, token_verifier().verify(&token).expect("claims").sid).await;
    assert!(revoked);
    assert_eq!(reason.as_deref(), Some("logout"));

    // The user still needs to change the password.
    let must_change: bool =
        sqlx::query_scalar("SELECT must_change_password FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("user state");
    assert!(must_change);
}
