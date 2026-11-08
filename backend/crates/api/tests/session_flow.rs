//! Logout, logout-all and the per-user session cap over HTTP with a real
//! PostgreSQL schema (docs/specs/login-system.md §5.3, §5.7, §5.9).

mod auth_support;

use auth_support::{
    NewUser, PASSWORD, app, authorized_request, insert_user, json, login, logout_request,
    problem_code, refresh_cookie, refresh_request, send, token_verifier,
};
use axum::http::{StatusCode, header};
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

async fn session_state(pool: &PgPool, session_id: Uuid) -> (bool, Option<String>) {
    let (revoked_at, revoke_reason): (Option<DateTime<Utc>>, Option<String>) =
        sqlx::query_as("SELECT revoked_at, revoke_reason FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .expect("session state");
    (revoked_at.is_some(), revoke_reason)
}

async fn session_id_of(body: &serde_json::Value) -> Uuid {
    token_verifier()
        .verify(body["access_token"].as_str().expect("access token"))
        .expect("verify access token")
        .sid
}

#[derive(sqlx::FromRow)]
struct LogoutAllAudit {
    action: String,
    severity: String,
    result: String,
    target_type: String,
    target_id: Option<Uuid>,
    target_label: Option<String>,
    actor_user_id: Option<Uuid>,
    details: serde_json::Value,
}

#[sqlx::test(migrations = "../../migrations")]
async fn logout_revokes_only_the_current_session_and_is_idempotent(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, first_headers, first_body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{first_body}");
    let first_cookie = refresh_cookie(&first_headers);
    let first_sid = session_id_of(&first_body).await;

    let (status, second_headers, second_body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{second_body}");
    let second_sid = session_id_of(&second_body).await;

    let (status, headers, body) = send(app.clone(), logout_request(Some(&first_cookie))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{:?}", json(&body));
    assert!(body.is_empty(), "204 must have an empty body");
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("clearing Set-Cookie");
    assert!(cookie.starts_with("__Host-refresh=;"), "{cookie}");
    assert!(cookie.contains("Max-Age=0"), "{cookie}");

    let (revoked, reason) = session_state(&pool, first_sid).await;
    assert!(revoked, "the presented session is revoked");
    assert_eq!(reason.as_deref(), Some("logout"));
    let (revoked, _) = session_state(&pool, second_sid).await;
    assert!(!revoked, "logout must not revoke other sessions");

    let tokens: Vec<(Option<DateTime<Utc>>, Option<String>)> = sqlx::query_as(
        "SELECT revoked_at, revoked_reason FROM auth_refresh_tokens WHERE session_id = $1",
    )
    .bind(first_sid)
    .fetch_all(&pool)
    .await
    .expect("token rows");
    assert!(!tokens.is_empty());
    for (revoked_at, reason) in tokens {
        assert!(revoked_at.is_some(), "the whole family is revoked");
        assert_eq!(reason.as_deref(), Some("logout"));
    }

    let audits: Vec<(Option<Uuid>, Option<Uuid>, serde_json::Value)> = sqlx::query_as(
        "SELECT target_id, actor_user_id, details FROM audit_logs WHERE action = 'authn_logout'",
    )
    .fetch_all(&pool)
    .await
    .expect("logout audit");
    assert_eq!(audits.len(), 1, "one event for one revoked session");
    assert_eq!(audits[0].0, Some(first_sid));
    assert_eq!(audits[0].1, Some(user_id));
    assert_eq!(audits[0].2["session_id"], first_sid.to_string());

    let audit_count_after_logout: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .expect("audit count");

    // A repeated logout, a missing cookie and an unknown token are all
    // idempotent 204s that write no further audit event.
    for raw in [Some(first_cookie.as_str()), None, Some("unknown-token")] {
        let (status, headers, body) = send(app.clone(), logout_request(raw)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{:?}", json(&body));
        assert!(
            headers.get(header::SET_COOKIE).is_some(),
            "every logout clears the cookie"
        );
    }
    let audit_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .expect("audit count");
    assert_eq!(
        audit_count, audit_count_after_logout,
        "repeated or unknown logouts write no audit event"
    );

    // The logged-out session can no longer refresh.
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&first_cookie))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");

    // The other session still refreshes normally.
    let (status, _headers, body) =
        send(app, refresh_request(Some(&refresh_cookie(&second_headers)))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
}

#[sqlx::test(migrations = "../../migrations")]
async fn logout_all_revokes_every_session_and_bumps_the_token_version(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, first_headers, first_body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{first_body}");
    let first_cookie = refresh_cookie(&first_headers);
    let first_sid = session_id_of(&first_body).await;

    let (status, second_headers, second_body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{second_body}");
    let second_cookie = refresh_cookie(&second_headers);
    let second_sid = session_id_of(&second_body).await;
    let access_token = second_body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();

    let (status, headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/logout-all",
            &access_token,
            Some(&serde_json::json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{:?}", json(&body));
    assert!(body.is_empty(), "204 must have an empty body");
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("clearing Set-Cookie");
    assert!(cookie.starts_with("__Host-refresh=;"), "{cookie}");
    assert!(cookie.contains("Max-Age=0"), "{cookie}");

    for session_id in [first_sid, second_sid] {
        let (revoked, reason) = session_state(&pool, session_id).await;
        assert!(revoked, "every session is revoked");
        assert_eq!(reason.as_deref(), Some("logout_all"));
    }
    let tokens: Vec<(Option<DateTime<Utc>>, Option<String>)> = sqlx::query_as(
        "SELECT revoked_at, revoked_reason FROM auth_refresh_tokens
         WHERE session_id = ANY($1) ORDER BY session_id",
    )
    .bind(&[first_sid, second_sid][..])
    .fetch_all(&pool)
    .await
    .expect("token rows");
    assert_eq!(tokens.len(), 2);
    for (revoked_at, reason) in tokens {
        assert!(revoked_at.is_some(), "the refresh family is revoked");
        assert_eq!(reason.as_deref(), Some("logout_all"));
    }

    let token_version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(
        token_version, 1,
        "logout-all invalidates every access token"
    );

    let audit: LogoutAllAudit = sqlx::query_as(
        "SELECT action, severity, result, target_type, target_id, target_label,
                actor_user_id, details
         FROM audit_logs WHERE action = 'authn_logout_all'",
    )
    .fetch_one(&pool)
    .await
    .expect("logout-all audit");
    assert_eq!(audit.action, "authn_logout_all");
    assert_eq!(audit.severity, "info");
    assert_eq!(audit.result, "success");
    assert_eq!(audit.target_type, "user");
    assert_eq!(audit.target_id, Some(user_id));
    assert_eq!(audit.target_label.as_deref(), Some("alice"));
    assert_eq!(audit.actor_user_id, Some(user_id));
    assert_eq!(audit.details["sessions_revoked_count"], 2);

    // The old access token is rejected on the next request, and no refresh
    // cookie can mint a new one.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &access_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");

    for cookie in [first_cookie, second_cookie] {
        let (status, _headers, body) = send(app.clone(), refresh_request(Some(&cookie))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(problem_code(&json(&body)), "session_invalid");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn logout_all_is_refused_for_a_restricted_session(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(
        &pool,
        NewUser {
            must_change_password: true,
            temporary_password_expires_at: Some(Utc::now() + Duration::hours(72)),
            ..NewUser::active("alice", "alice@example.com")
        },
    )
    .await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session_id = session_id_of(&body).await;
    let access_token = body["access_token"].as_str().expect("access token");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/logout-all",
            access_token,
            Some(&serde_json::json!({})),
        ),
    )
    .await;
    let body = json(&body);
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&body), "password_change_required");

    let (revoked, _) = session_state(&pool, session_id).await;
    assert!(!revoked, "a refused logout-all changes nothing");
    let token_version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(token_version, 0);
    let audits: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = 'authn_logout_all'")
            .fetch_one(&pool)
            .await
            .expect("audit count");
    assert_eq!(audits, 0);
    assert!(
        !refresh_cookie(&headers).is_empty(),
        "a restricted session received a cookie in the first place"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn sixth_login_evicts_the_least_recently_active_session(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let mut sessions: Vec<(Uuid, String)> = Vec::new();
    for _ in 0..5 {
        let (status, headers, body) = login(&app, "alice", PASSWORD).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        sessions.push((session_id_of(&body).await, refresh_cookie(&headers)));
    }

    // Give every session a distinct activity time so eviction is
    // deterministic: the first login is the least recently active.
    let base = Utc::now() - Duration::minutes(10);
    for (index, (session_id, _)) in sessions.iter().enumerate() {
        sqlx::query("UPDATE auth_sessions SET last_seen_at = $2 WHERE id = $1")
            .bind(*session_id)
            .bind(base + Duration::minutes(index as i64 + 1))
            .execute(&pool)
            .await
            .expect("age session");
    }

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let newest_sid = session_id_of(&body).await;
    let newest_cookie = refresh_cookie(&headers);

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("session count");
    assert_eq!(total, 6, "the sixth login still creates a session");
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("active session count");
    assert_eq!(active, 5, "at most five sessions stay active");

    let (evicted_sid, evicted_cookie) = sessions[0].clone();
    let (revoked, reason) = session_state(&pool, evicted_sid).await;
    assert!(revoked, "the least recently active session is evicted");
    assert_eq!(reason.as_deref(), Some("excess_sessions"));
    for (session_id, _) in &sessions[1..] {
        let (revoked, _) = session_state(&pool, *session_id).await;
        assert!(!revoked, "more recently active sessions are kept");
    }

    let tokens: Vec<(Option<DateTime<Utc>>, Option<String>)> = sqlx::query_as(
        "SELECT revoked_at, revoked_reason FROM auth_refresh_tokens WHERE session_id = $1",
    )
    .bind(evicted_sid)
    .fetch_all(&pool)
    .await
    .expect("token rows");
    assert!(!tokens.is_empty());
    for (revoked_at, reason) in tokens {
        assert!(revoked_at.is_some(), "the evicted family is revoked");
        assert_eq!(reason.as_deref(), Some("excess_sessions"));
    }

    let (action, severity, target_type, target_id, actor_id, details): (
        String,
        String,
        String,
        Option<Uuid>,
        Option<Uuid>,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT action, severity, target_type, target_id, actor_user_id, details
         FROM audit_logs WHERE action = 'authn_session_evicted'",
    )
    .fetch_one(&pool)
    .await
    .expect("eviction audit");
    assert_eq!(action, "authn_session_evicted");
    assert_eq!(severity, "info");
    assert_eq!(target_type, "session");
    assert_eq!(target_id, Some(evicted_sid));
    assert_eq!(actor_id, Some(user_id));
    assert_eq!(details["evicted_session_id"], evicted_sid.to_string());
    assert_eq!(details["new_session_id"], newest_sid.to_string());

    // The evicted device cannot refresh; the new session can.
    let (status, _headers, body) = send(app.clone(), refresh_request(Some(&evicted_cookie))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");
    let (status, _headers, body) = send(app, refresh_request(Some(&newest_cookie))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_logins_never_exceed_the_session_cap(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    // Eight logins race on the same user row; the row lock must serialize the
    // count-and-evict step so the cap holds under contention.
    let responses = tokio::join!(
        login(&app, "alice", PASSWORD),
        login(&app, "alice", PASSWORD),
        login(&app, "alice", PASSWORD),
        login(&app, "alice", PASSWORD),
        login(&app, "alice", PASSWORD),
        login(&app, "alice", PASSWORD),
        login(&app, "alice", PASSWORD),
        login(&app, "alice", PASSWORD),
    );
    for (status, _headers, body) in [
        responses.0,
        responses.1,
        responses.2,
        responses.3,
        responses.4,
        responses.5,
        responses.6,
        responses.7,
    ] {
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("session count");
    assert_eq!(total, 8, "every login created a session");
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("active session count");
    assert_eq!(active, 5, "concurrent logins must respect the cap");
    let evicted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions
         WHERE user_id = $1 AND revoke_reason = 'excess_sessions'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("evicted session count");
    assert_eq!(evicted, 3);

    let eviction_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs WHERE action = 'authn_session_evicted'",
    )
    .fetch_one(&pool)
    .await
    .expect("eviction audit count");
    assert_eq!(eviction_audits, 3);
    let login_audits: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = 'authn_login_success'")
            .fetch_one(&pool)
            .await
            .expect("login audit count");
    assert_eq!(login_audits, 8);
}

#[sqlx::test(migrations = "../../migrations")]
async fn expired_sessions_do_not_consume_session_slots(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let mut sessions: Vec<Uuid> = Vec::new();
    for _ in 0..5 {
        let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        sessions.push(session_id_of(&body).await);
    }

    // The least recently active session has already timed out, so it is not an
    // "active session" and must not force another login to evict a usable one.
    sqlx::query(
        "UPDATE auth_sessions
         SET last_seen_at = now() - interval '1 hour',
             idle_expires_at = now() - interval '1 second',
             absolute_expires_at = now() - interval '1 second'
         WHERE id = $1",
    )
    .bind(sessions[0])
    .execute(&pool)
    .await
    .expect("expire session");

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let evictions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs WHERE action = 'authn_session_evicted'",
    )
    .fetch_one(&pool)
    .await
    .expect("eviction audit count");
    assert_eq!(evictions, 0, "an expired session is not evicted");
    for session_id in &sessions {
        let (revoked, _) = session_state(&pool, *session_id).await;
        assert!(!revoked, "no usable session may be revoked");
    }
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions
         WHERE user_id = $1 AND revoked_at IS NULL
           AND idle_expires_at > now() AND absolute_expires_at > now()",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("active session count");
    assert_eq!(active, 5);
}

#[sqlx::test(migrations = "../../migrations")]
async fn logout_of_an_expired_session_writes_no_audit(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cookie = refresh_cookie(&headers);
    let session_id = session_id_of(&body).await;

    sqlx::query(
        "UPDATE auth_sessions
         SET idle_expires_at = now() - interval '1 second',
             absolute_expires_at = now() - interval '1 second'
         WHERE id = $1",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("expire session");

    let (status, headers, body) = send(app, logout_request(Some(&cookie))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{:?}", json(&body));
    assert!(
        headers.get(header::SET_COOKIE).is_some(),
        "the cookie is still cleared"
    );

    let (revoked, _) = session_state(&pool, session_id).await;
    assert!(!revoked, "an expired session is left untouched");
    let audits: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = 'authn_logout'")
            .fetch_one(&pool)
            .await
            .expect("logout audit count");
    assert_eq!(audits, 0, "logout audits only a session it actually ended");
}
