//! End-to-end authentication behaviour over HTTP with a real PostgreSQL schema
//! (docs/specs/login-system.md §5.1–§5.8, §7.6, §14.1).

mod auth_support;

use auth_support::{
    AUDIENCE, ISSUER, NewUser, PASSWORD, app, authorized_request, insert_user, json, json_request,
    login, password_service, problem_code, refresh_cookie, refresh_token_rows, send, session_count,
    token_verifier,
};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::{DateTime, Duration, Utc};
use rs_admin_audit::verify_chain;
use rs_admin_security::hash_refresh_token;
use sqlx::PgPool;
use uuid::Uuid;

fn without_request_id(mut body: serde_json::Value) -> serde_json::Value {
    body.as_object_mut().expect("object").remove("request_id");
    body
}

async fn access_token(pool: &PgPool, app: &Router, user: &NewUser<'_>) -> (String, Uuid) {
    let user_id = insert_user(pool, NewUser { ..*user }).await;
    let (status, _headers, body) = login(app, user.username, user.password).await;
    assert_eq!(status, StatusCode::OK, "login failed: {body}");
    let token = body["access_token"].as_str().expect("token").to_owned();
    (token, user_id)
}

#[sqlx::test(migrations = "../../migrations")]
async fn username_login_issues_a_fifteen_minute_token_and_host_only_cookie(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );

    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("Set-Cookie");
    assert!(cookie.starts_with("__Host-refresh="), "{cookie}");
    for attribute in [
        "Path=/",
        "Secure",
        "HttpOnly",
        "SameSite=Strict",
        "Max-Age=43200",
    ] {
        assert!(cookie.contains(attribute), "missing {attribute}: {cookie}");
    }
    assert!(
        !cookie.contains("Domain"),
        "Domain must be omitted: {cookie}"
    );

    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 900);
    assert_eq!(body["user"]["id"], user_id.to_string());
    assert_eq!(body["user"]["username"], "alice");
    assert_eq!(body["user"]["email"], "alice@example.com");
    assert_eq!(body["user"]["status"], "active");
    assert_eq!(body["user"]["must_change_password"], false);
    assert_eq!(body["user"]["is_super_admin"], false);
    assert_eq!(body["user"]["permissions"], serde_json::json!([]));
    assert_eq!(body["user"]["roles"][0]["name"], "user");
    assert_eq!(body["user"]["roles"][0]["is_builtin"], true);
    assert!(body["user"]["temporary_password_expires_at"].is_null());

    let claims = token_verifier()
        .verify(body["access_token"].as_str().expect("token"))
        .expect("verify token");
    let session_id: Uuid = sqlx::query_scalar("SELECT id FROM auth_sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("session");

    assert_eq!(claims.sub, user_id);
    assert_eq!(claims.sid, session_id);
    assert_eq!(claims.ver, 0);
    assert_eq!(claims.iss, ISSUER);
    assert_eq!(claims.aud, AUDIENCE);
    assert_eq!(claims.exp - claims.iat, 900);
    assert_eq!(claims.nbf, claims.iat);
    assert_eq!(claims.typ, "at+jwt");
    assert!(!claims.jti.is_nil());
    assert!(claims.auth_time > 0);

    let (idle_expires_at, absolute_expires_at, authenticated_at): (
        DateTime<Utc>,
        DateTime<Utc>,
        DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT idle_expires_at, absolute_expires_at, authenticated_at
         FROM auth_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("session times");
    assert_eq!((idle_expires_at - authenticated_at).num_seconds(), 1800);
    assert_eq!(
        (absolute_expires_at - authenticated_at).num_seconds(),
        43_200
    );

    let raw = refresh_cookie(&headers);
    let (stored, counter, used_at): (Vec<u8>, i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT token_hash, rotation_counter, used_at
         FROM auth_refresh_tokens WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("refresh token");
    assert_eq!(stored, hash_refresh_token(&raw).to_vec());
    assert_eq!(counter, 0);
    assert!(used_at.is_none());

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
    assert_eq!(action, "authn_login_success");
    assert_eq!(severity, "info");
    assert_eq!(result, "success");
    assert_eq!(target_type, "session");
    assert_eq!(target_id, Some(session_id));
    assert_eq!(actor_id, Some(user_id));
    assert_eq!(details["auth_method"], "password");
    assert_eq!(details["session_id"], session_id.to_string());
    assert_eq!(details["must_change_password"], false);
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_accepts_a_normalized_email_identifier(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, _headers, body) = login(&app, "  ALICE@Example.COM  ", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["username"], "alice");
}

#[sqlx::test(migrations = "../../migrations")]
async fn wrong_password_and_unknown_identifier_are_indistinguishable(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (bad_status, _bad_headers, bad_body) = login(&app, "alice", "wrong password").await;
    let (unknown_status, _unknown_headers, unknown_body) =
        login(&app, "nobody@example.com", "wrong password").await;

    assert_eq!(bad_status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&unknown_body), "invalid_credentials");
    assert_eq!(
        without_request_id(bad_body),
        without_request_id(unknown_body),
        "responses must not reveal whether the identifier exists"
    );

    let failed_count: i32 =
        sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("counter");
    assert_eq!(failed_count, 1, "known identifiers increment the counter");

    let (known_actor, known_target, known_reason): (String, Option<Uuid>, String) = sqlx::query_as(
        "SELECT actor_label, target_id, details->>'reason_code'
         FROM audit_logs WHERE action = 'authn_login_fail' AND target_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("known failure audit");
    assert_eq!(known_actor, "alice");
    assert_eq!(known_target, Some(user_id));
    assert_eq!(known_reason, "bad_credentials");

    let (unknown_actor, unknown_target, identifier_hmac, details): (
        String,
        Option<Uuid>,
        String,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT actor_label, target_id, details->>'identifier_hmac', details
         FROM audit_logs
         WHERE action = 'authn_login_fail' AND target_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("unknown failure audit");
    assert_eq!(unknown_actor, "anonymous");
    assert!(unknown_target.is_none());
    assert_eq!(identifier_hmac.len(), 64);
    assert!(!details.to_string().contains("nobody@example.com"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_disabled_account_is_refused_only_after_a_correct_password(pool: PgPool) {
    let app = app(&pool).await;
    let user_id = insert_user(
        &pool,
        NewUser {
            status: "disabled",
            ..NewUser::active("alice", "alice@example.com")
        },
    )
    .await;

    let (status, _headers, body) = login(&app, "alice", "wrong password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&body), "invalid_credentials");

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&body), "account_disabled");

    let failed_count: i32 =
        sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("counter");
    assert_eq!(failed_count, 1, "account-state refusals do not count");

    let (result, reason): (String, String) = sqlx::query_as(
        "SELECT result, details->>'reason_code' FROM audit_logs
         WHERE action = 'authn_login_fail' AND details->>'reason_code' = 'account_disabled'",
    )
    .fetch_one(&pool)
    .await
    .expect("audit");
    assert_eq!(result, "denied");
    assert_eq!(reason, "account_disabled");
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_expired_temporary_password_is_refused(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            must_change_password: true,
            temporary_password_expires_at: Some(Utc::now() - Duration::hours(1)),
            ..NewUser::active("alice", "alice@example.com")
        },
    )
    .await;

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&body), "temporary_password_expired");

    let reason: String = sqlx::query_scalar(
        "SELECT details->>'reason_code' FROM audit_logs
         WHERE action = 'authn_login_fail'",
    )
    .fetch_one(&pool)
    .await
    .expect("audit");
    assert_eq!(reason, "temporary_password_expired");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_temporary_password_creates_a_restricted_session(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            must_change_password: true,
            temporary_password_expires_at: Some(Utc::now() + Duration::hours(72)),
            ..NewUser::active("alice", "alice@example.com")
        },
    )
    .await;

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["must_change_password"], true);
    assert!(body["user"]["temporary_password_expires_at"].is_string());
    let token = body["access_token"].as_str().expect("token");

    let (status, _headers, me) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me:?}");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            token,
            Some(&serde_json::json!({ "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&json(&body)), "password_change_required");
}

#[sqlx::test(migrations = "../../migrations")]
async fn me_returns_roles_and_effective_permissions(pool: PgPool) {
    let app = app(&pool).await;
    let super_admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("superuser", "superuser@example.com")
        },
    )
    .await;
    let admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("manager", "manager@example.com")
        },
    )
    .await;

    let (status, _headers, body) = login(&app, "superuser", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["access_token"].as_str().expect("token");
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let me = json(&body);
    assert_eq!(me["id"], super_admin_id.to_string());
    assert_eq!(me["is_super_admin"], true);
    assert_eq!(me["permissions"].as_array().expect("permissions").len(), 13);
    let roles: Vec<&str> = me["roles"]
        .as_array()
        .expect("roles")
        .iter()
        .filter_map(|role| role["name"].as_str())
        .collect();
    assert!(roles.contains(&"super_admin"));
    assert!(roles.contains(&"user"));

    let (status, _headers, body) = login(&app, "manager", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["access_token"].as_str().expect("token");
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let me = json(&body);
    assert_eq!(me["id"], admin_id.to_string());
    assert_eq!(me["is_super_admin"], false);
    assert_eq!(me["permissions"].as_array().expect("permissions").len(), 10);
    assert!(
        me["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .any(|p| p == "users.read")
    );
    assert!(
        !me["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .any(|p| p == "roles.create")
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn me_rejects_missing_invalid_or_revoked_credentials(pool: PgPool) {
    let app = app(&pool).await;
    let user = NewUser::active("alice", "alice@example.com");
    let (token, user_id) = access_token(&pool, &app, &user).await;

    let (status, headers, body) =
        send(app.clone(), auth_support::request("GET", "/api/v1/auth/me")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "access_token_invalid");
    assert_eq!(
        headers
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer realm=\"rs-admin\"")
    );

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", "not-a-jwt", None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "access_token_invalid");

    sqlx::query(
        "UPDATE auth_sessions SET revoked_at = now(), revoke_reason = 'logout' WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("revoke");
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");

    // A fresh login, then a token-version bump invalidates the access token.
    let (_status, _headers, body) = login(&app, "alice", PASSWORD).await;
    let token = body["access_token"].as_str().expect("token").to_owned();
    sqlx::query("UPDATE users SET token_version = token_version + 1 WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("bump version");
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "session_invalid");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_disabled_user_with_a_valid_access_token_gets_403(pool: PgPool) {
    let app = app(&pool).await;
    let (token, user_id) =
        access_token(&pool, &app, &NewUser::active("alice", "alice@example.com")).await;

    sqlx::query("UPDATE users SET status = 'disabled' WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("disable");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&json(&body)), "account_disabled");
}

#[sqlx::test(migrations = "../../migrations")]
async fn reauthentication_refreshes_auth_time_without_rotating_the_session(pool: PgPool) {
    let app = app(&pool).await;
    let user = NewUser::active("alice", "alice@example.com");
    let user_id = insert_user(&pool, NewUser { ..user }).await;
    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["access_token"].as_str().expect("token").to_owned();

    let session_id: Uuid = sqlx::query_scalar("SELECT id FROM auth_sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("session");
    let old_auth_time = Utc::now() - Duration::minutes(10);
    sqlx::query("UPDATE auth_sessions SET authenticated_at = $2 WHERE id = $1")
        .bind(session_id)
        .bind(old_auth_time)
        .execute(&pool)
        .await
        .expect("age session");

    let tokens_before = refresh_token_rows(&pool, session_id).await;
    let sessions_before = session_count(&pool, user_id).await;
    let last_login_before: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT last_login_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("last login");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            &token,
            Some(&serde_json::json!({ "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let body = json(&body);
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 900);
    let auth_time = body["auth_time"].as_i64().expect("auth_time");
    assert!(auth_time > old_auth_time.timestamp());
    assert!(auth_time <= Utc::now().timestamp() + 1);

    let new_token = body["access_token"].as_str().expect("token");
    let old_claims = token_verifier().verify(&token).expect("old token");
    let new_claims = token_verifier().verify(new_token).expect("new token");
    assert_eq!(new_claims.sub, old_claims.sub);
    assert_eq!(new_claims.sid, old_claims.sid);
    assert_eq!(new_claims.sid, session_id);
    assert_eq!(new_claims.ver, old_claims.ver);
    assert_eq!(new_claims.auth_time, auth_time);
    assert_ne!(new_claims.jti, old_claims.jti);

    assert_eq!(refresh_token_rows(&pool, session_id).await, tokens_before);
    assert_eq!(session_count(&pool, user_id).await, sessions_before);
    let last_login_after: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT last_login_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("last login");
    assert_eq!(last_login_after, last_login_before);
    let stored_auth_time: DateTime<Utc> =
        sqlx::query_scalar("SELECT authenticated_at FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("auth time");
    assert_eq!(stored_auth_time.timestamp(), auth_time);

    let (action, result): (String, String) =
        sqlx::query_as("SELECT action, result FROM audit_logs ORDER BY chain_seq DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("audit");
    assert_eq!(action, "authn_reauth_success");
    assert_eq!(result, "success");
}

#[sqlx::test(migrations = "../../migrations")]
async fn reauthentication_with_a_wrong_password_counts_the_failure(pool: PgPool) {
    let app = app(&pool).await;
    let user = NewUser::active("alice", "alice@example.com");
    let user_id = insert_user(&pool, NewUser { ..user }).await;
    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["access_token"].as_str().expect("token").to_owned();
    let session_id: Uuid = sqlx::query_scalar("SELECT id FROM auth_sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("session");
    let auth_time_before: DateTime<Utc> =
        sqlx::query_scalar("SELECT authenticated_at FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("auth time");
    let tokens_before = refresh_token_rows(&pool, session_id).await;

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            &token,
            Some(&serde_json::json!({ "password": "wrong password" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(problem_code(&json(&body)), "invalid_current_password");

    let failed_count: i32 =
        sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("counter");
    assert_eq!(failed_count, 1);
    let auth_time_after: DateTime<Utc> =
        sqlx::query_scalar("SELECT authenticated_at FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("auth time");
    assert_eq!(auth_time_after, auth_time_before);
    assert_eq!(refresh_token_rows(&pool, session_id).await, tokens_before);

    let (action, result, reason): (String, String, String) = sqlx::query_as(
        "SELECT action, result, details->>'reason_code' FROM audit_logs
         WHERE action = 'authn_reauth_fail'",
    )
    .fetch_one(&pool)
    .await
    .expect("audit");
    assert_eq!(result, "failure");
    assert_eq!(reason, "bad_credentials");
    assert_eq!(action, "authn_reauth_fail");
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_rehashes_a_hash_below_the_current_parameters(pool: PgPool) {
    let app = app(&pool).await;
    let weak = weak_hash(PASSWORD);
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (username, email, display_name, password_hash)
         VALUES ('alice', 'alice@example.com', 'alice', $1)
         RETURNING id",
    )
    .bind(&weak)
    .fetch_one(&pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id)
         SELECT $1, id FROM roles WHERE name = 'user'",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("baseline role");

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let upgraded: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("hash");
    assert!(
        upgraded.starts_with("$argon2id$v=19$m=65536,t=3,p=4$"),
        "hash was not upgraded: {upgraded}"
    );
    assert_ne!(upgraded, weak);
    assert!(
        password_service()
            .verify(PASSWORD, &upgraded)
            .await
            .expect("verify")
            .valid
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn authentication_events_keep_the_audit_chain_verifiable(pool: PgPool) {
    let app = app(&pool).await;
    let user = NewUser::active("alice", "alice@example.com");
    insert_user(&pool, NewUser { ..user }).await;

    let _ = login(&app, "alice", "wrong password").await;
    let _ = login(&app, "nobody@example.com", "wrong password").await;
    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["access_token"].as_str().expect("token").to_owned();
    let (status, _headers, _body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            &token,
            Some(&serde_json::json!({ "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let verification = verify_chain(&pool, &auth_support::security_config().audit_chain.keys)
        .await
        .expect("chain must verify");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(verification.verified_rows, stored as u64);
    assert_eq!(verification.head_seq, stored);
}

#[sqlx::test(migrations = "../../migrations")]
async fn request_validation_follows_the_problem_contract(pool: PgPool) {
    let app = app(&pool).await;

    let (status, _headers, body) = send(
        app.clone(),
        json_request(
            "POST",
            "/api/v1/auth/login",
            &serde_json::json!({
                "identifier": "alice",
                "password": "whatever",
                "unexpected": true
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(&json(&body)), "unknown_field");

    let (status, _headers, body) = send(
        app.clone(),
        json_request(
            "POST",
            "/api/v1/auth/login",
            &serde_json::json!({ "identifier": "alice" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(&body);
    assert_eq!(problem_code(&body), "validation_failed");
    assert_eq!(body["errors"][0]["pointer"], "/password");
    assert_eq!(body["errors"][0]["code"], "required");

    let (status, _headers, body) = send(
        app.clone(),
        json_request(
            "POST",
            "/api/v1/auth/login",
            &serde_json::json!({ "identifier": "alice", "password": 123 }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(problem_code(&json(&body)), "validation_failed");

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(r#"{"identifier":"alice","password":"x"}"#))
        .expect("request");
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(problem_code(&json(&body)), "unsupported_media_type");
}

#[sqlx::test(migrations = "../../migrations")]
async fn authentication_bodies_are_capped_at_one_kibibyte(pool: PgPool) {
    let app = app(&pool).await;
    let oversized = "x".repeat(2048);
    let (status, _headers, body) = send(
        app,
        json_request(
            "POST",
            "/api/v1/auth/login",
            &serde_json::json!({ "identifier": "alice", "password": oversized }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(problem_code(&json(&body)), "payload_too_large");
}

fn weak_hash(password: &str) -> String {
    use argon2::password_hash::PasswordHasher as _;
    let params = argon2::Params::new(19 * 1024, 2, 1, Some(32)).expect("params");
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password(password.as_bytes())
        .expect("hash")
        .to_string()
}
