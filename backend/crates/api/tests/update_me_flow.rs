//! Self-service profile updates over HTTP with a real PostgreSQL schema
//! (docs/specs/login-system.md §5.4, §5.8, §7.6, §9.11).
//!
//! `PATCH /auth/me` accepts exactly one field — `display_name` — and is
//! refused for restricted sessions.

mod auth_support;

use auth_support::{
    NewUser, app, authorized_request, insert_user, json as parse_json, login, problem_code, send,
};
use axum::http::{StatusCode, header};
use rs_admin_audit::verify_chain;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const UPDATE_ME: &str = "/api/v1/auth/me";

async fn access_token(pool: &PgPool, user: NewUser<'_>) -> (String, Uuid, axum::Router) {
    let app = app(pool).await;
    let user_id = insert_user(pool, user).await;
    let (status, _headers, body) = login(&app, user.username, user.password).await;
    assert_eq!(status, StatusCode::OK, "login failed: {body}");
    let token = body["access_token"].as_str().expect("token").to_owned();
    (token, user_id, app)
}

async fn display_name(pool: &PgPool, user_id: Uuid) -> String {
    sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("display name")
}

#[sqlx::test(migrations = "../../migrations")]
async fn patch_me_updates_the_display_name_and_writes_audit(pool: PgPool) {
    let (token, user_id, app) =
        access_token(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = send(
        app,
        authorized_request(
            "PATCH",
            UPDATE_ME,
            &token,
            Some(&json!({ "display_name": "Alice Chen" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let body = parse_json(&body);
    assert_eq!(body["display_name"], "Alice Chen");
    // Only the display name is mutable; identity and authorization data are
    // returned unchanged.
    assert_eq!(body["id"], user_id.to_string());
    assert_eq!(body["username"], "alice");
    assert_eq!(body["email"], "alice@example.com");
    assert_eq!(body["status"], "active");
    assert_eq!(body["must_change_password"], false);
    assert_eq!(body["roles"][0]["name"], "user");
    assert_eq!(body["permissions"], json!([]));
    assert_eq!(body["is_super_admin"], false);

    assert_eq!(display_name(&pool, user_id).await, "Alice Chen");

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
    assert_eq!(action, "user_updated");
    assert_eq!(severity, "info");
    assert_eq!(result, "success");
    assert_eq!(target_type, "user");
    assert_eq!(target_id, Some(user_id));
    assert_eq!(actor_id, Some(user_id));
    assert_eq!(details["changed_fields"]["display_name"], true);
    assert_eq!(details["source"], "self");

    verify_chain(&pool, &auth_support::security_config().audit_chain.keys)
        .await
        .expect("audit chain must verify");
}

#[sqlx::test(migrations = "../../migrations")]
async fn patch_me_rejects_an_empty_body(pool: PgPool) {
    let (token, user_id, app) =
        access_token(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, _headers, body) = send(
        app,
        authorized_request("PATCH", UPDATE_ME, &token, Some(&json!({}))),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(problem_code(&parse_json(&body)), "empty_update");
    assert_eq!(display_name(&pool, user_id).await, "alice");
}

#[sqlx::test(migrations = "../../migrations")]
async fn patch_me_rejects_fields_other_than_display_name(pool: PgPool) {
    let (token, _user_id, app) =
        access_token(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, _headers, body) = send(
        app,
        authorized_request(
            "PATCH",
            UPDATE_ME,
            &token,
            Some(&json!({ "username": "mallory" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(&parse_json(&body)), "unknown_field");
}

#[sqlx::test(migrations = "../../migrations")]
async fn patch_me_rejects_null_and_invalid_display_names(pool: PgPool) {
    let (token, user_id, app) =
        access_token(&pool, NewUser::active("alice", "alice@example.com")).await;

    for (payload, code) in [
        (json!({ "display_name": null }), "invalid"),
        (json!({ "display_name": "" }), "invalid_length"),
        (json!({ "display_name": "x".repeat(65) }), "invalid_length"),
    ] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request("PATCH", UPDATE_ME, &token, Some(&payload)),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "payload {payload}"
        );
        let body = parse_json(&body);
        assert_eq!(problem_code(&body), "validation_failed", "{payload}");
        assert_eq!(body["errors"][0]["pointer"], "/display_name", "{payload}");
        assert_eq!(body["errors"][0]["code"], code, "{payload}");
        assert_eq!(display_name(&pool, user_id).await, "alice");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn patch_me_accepts_a_sixty_four_character_display_name(pool: PgPool) {
    let (token, user_id, app) =
        access_token(&pool, NewUser::active("alice", "alice@example.com")).await;
    let name = "名".repeat(64);

    let (status, _headers, body) = send(
        app,
        authorized_request(
            "PATCH",
            UPDATE_ME,
            &token,
            Some(&json!({ "display_name": name })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(parse_json(&body)["display_name"], name);
    assert_eq!(display_name(&pool, user_id).await, name);
}

#[sqlx::test(migrations = "../../migrations")]
async fn patch_me_is_refused_for_a_restricted_session(pool: PgPool) {
    let (token, user_id, app) =
        access_token(&pool, NewUser::restricted("alice", "alice@example.com")).await;

    let (status, _headers, body) = send(
        app,
        authorized_request(
            "PATCH",
            UPDATE_ME,
            &token,
            Some(&json!({ "display_name": "Alice Chen" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&parse_json(&body)), "password_change_required");
    assert_eq!(display_name(&pool, user_id).await, "alice");
}

#[sqlx::test(migrations = "../../migrations")]
async fn patch_me_requires_bearer_authentication(pool: PgPool) {
    let app = app(&pool).await;
    let _user_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = send(
        app,
        authorized_request(
            "PATCH",
            UPDATE_ME,
            "",
            Some(&json!({ "display_name": "Alice Chen" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let body = parse_json(&body);
    assert_eq!(problem_code(&body), "access_token_invalid");
    assert!(
        headers.get(header::WWW_AUTHENTICATE).is_some(),
        "401 must carry the Bearer challenge"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn patch_me_with_the_same_display_name_is_a_no_op(pool: PgPool) {
    let (token, user_id, app) =
        access_token(&pool, NewUser::active("alice", "alice@example.com")).await;

    let before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = 'user_updated'")
            .fetch_one(&pool)
            .await
            .expect("audit count");

    let (status, _headers, body) = send(
        app,
        authorized_request(
            "PATCH",
            UPDATE_ME,
            &token,
            Some(&json!({ "display_name": "alice" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse_json(&body)["display_name"], "alice");
    assert_eq!(display_name(&pool, user_id).await, "alice");
    let after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = 'user_updated'")
            .fetch_one(&pool)
            .await
            .expect("audit count");
    assert_eq!(after, before, "an unchanged value writes no audit event");
}
