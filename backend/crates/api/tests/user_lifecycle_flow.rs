//! User role assignment and lifecycle actions over HTTP with a real PostgreSQL
//! schema (docs/specs/login-system.md §5.7–§5.9, §6.3, §7.7).

mod auth_support;

use auth_support::{
    NewUser, PASSWORD, app, authorized_request, insert_user, login, problem_code, session_count,
};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

/// Send a request and decode the JSON body, so assertions read from the wire
/// contract rather than raw bytes.
async fn send(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let (status, headers, body) = auth_support::send(app, request).await;
    let value = if body.is_empty() {
        Value::Null
    } else {
        auth_support::json(&body)
    };
    (status, headers, value)
}

async fn login_token(app: &Router, username: &str) -> String {
    let (status, _headers, body) = login(app, username, PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "login failed: {body}");
    body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

async fn insert_custom_role(pool: &PgPool, name: &str, permission_keys: &[&str]) -> Uuid {
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO roles (name, is_builtin) VALUES ($1, false) RETURNING id")
            .bind(name)
            .fetch_one(pool)
            .await
            .expect("insert custom role");

    for key in permission_keys {
        sqlx::query("INSERT INTO role_permissions (role_id, permission_key) VALUES ($1, $2)")
            .bind(id)
            .bind(*key)
            .execute(pool)
            .await
            .expect("role permission");
    }
    id
}

async fn assign_role(pool: &PgPool, user_id: Uuid, role_id: Uuid) {
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await
        .expect("assign role");
}

async fn builtin_role_id(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM roles WHERE name = $1 AND is_builtin")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("builtin role id")
}

async fn user_id(pool: &PgPool, username: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(pool)
        .await
        .expect("user id")
}

fn role_names(body: &Value) -> Vec<&str> {
    body["roles"]
        .as_array()
        .expect("roles")
        .iter()
        .map(|role| role["name"].as_str().expect("role name"))
        .collect()
}

fn added_removed(details: &Value) -> (Vec<&str>, Vec<&str>) {
    let ids = |key: &str| -> Vec<&str> {
        details[key]
            .as_array()
            .expect("role id array")
            .iter()
            .map(|value| value.as_str().expect("role id"))
            .collect()
    };
    (ids("added_role_ids"), ids("removed_role_ids"))
}

#[sqlx::test(migrations = "../../migrations")]
async fn replace_roles_keeps_the_baseline_role_and_audits_the_difference(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let old_role = insert_custom_role(&pool, "support", &["users.read"]).await;
    let new_role = insert_custom_role(&pool, "auditor", &["audit.read"]).await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    assign_role(&pool, alice_id, old_role).await;
    let baseline = builtin_role_id(&pool, "user").await;

    let token = login_token(&app, "root1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PUT",
            &format!("/api/v1/users/{alice_id}/roles"),
            &token,
            Some(&json!({ "role_ids": [new_role.to_string()] })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    // The baseline `user` role is retained even though the request omitted it.
    assert_eq!(role_names(&body), ["user", "auditor"]);
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT r.name FROM user_roles ur JOIN roles r ON r.id = ur.role_id
         WHERE ur.user_id = $1 ORDER BY r.is_builtin DESC, r.name",
    )
    .bind(alice_id)
    .fetch_all(&pool)
    .await
    .expect("stored roles");
    assert_eq!(stored, ["user", "auditor"]);

    let (severity, details): (String, Value) = sqlx::query_as(
        "SELECT severity, details FROM audit_logs WHERE action = 'user_roles_changed'",
    )
    .fetch_one(&pool)
    .await
    .expect("role change audit");
    assert_eq!(severity, "warning");
    let (added, removed) = added_removed(&details);
    let new_role = new_role.to_string();
    let old_role = old_role.to_string();
    assert_eq!(added, [new_role.as_str()]);
    assert_eq!(removed, [old_role.as_str()]);
    assert_eq!(details["super_admin_involved"], false);
    let _ = baseline;

    // A request that names only the baseline removes every other role.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PUT",
            &format!("/api/v1/users/{alice_id}/roles"),
            &token,
            Some(&json!({ "role_ids": [baseline.to_string()] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(role_names(&body), ["user"]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn replace_roles_validates_roles_authorization_and_step_up(pool: PgPool) {
    let app = app(&pool).await;
    let admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let root_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    let alice_id = user_id(&pool, "alice").await;

    let support = insert_custom_role(&pool, "support", &["users.read"]).await;
    let role_maker = insert_custom_role(&pool, "role-maker", &["roles.create"]).await;
    let deleted = insert_custom_role(&pool, "gone", &["users.read"]).await;
    sqlx::query("UPDATE roles SET deleted_at = now() WHERE id = $1")
        .bind(deleted)
        .execute(&pool)
        .await
        .expect("soft delete role");
    let admin_role = builtin_role_id(&pool, "admin").await;

    let admin_token = login_token(&app, "admin1").await;
    let put = |token: String, target: Uuid, role_ids: Vec<String>| {
        let app = app.clone();
        async move {
            send(
                app,
                authorized_request(
                    "PUT",
                    &format!("/api/v1/users/{target}/roles"),
                    &token,
                    Some(&json!({ "role_ids": role_ids })),
                ),
            )
            .await
        }
    };

    // Unknown, soft-deleted and malformed ids never change the target.
    let (status, _headers, problem) = put(
        admin_token.clone(),
        alice_id,
        vec![Uuid::now_v7().to_string()],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "unknown_role");
    assert_eq!(problem["errors"][0]["pointer"], "/role_ids");

    let (status, _headers, problem) =
        put(admin_token.clone(), alice_id, vec![deleted.to_string()]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "unknown_role");

    let (status, _headers, problem) =
        put(admin_token.clone(), alice_id, vec!["not-a-uuid".to_owned()]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "validation_failed");
    assert_eq!(problem["errors"][0]["pointer"], "/role_ids/0");

    // Privileged and broader custom roles violate the grant-subset rule.
    for role_id in [admin_role, role_maker] {
        let (status, _headers, problem) =
            put(admin_token.clone(), alice_id, vec![role_id.to_string()]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem_code(&problem), "permission_subset_violation");
    }
    let role_denials: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied'
           AND details->>'operation' = 'set_user_roles'
           AND details->>'reason_code' = 'permission_subset_violation'
           AND target_type = 'role'",
    )
    .fetch_one(&pool)
    .await
    .expect("role subset denials");
    assert_eq!(role_denials, 2);

    // A role within the operator's permissions is assignable.
    let (status, _headers, body) =
        put(admin_token.clone(), alice_id, vec![support.to_string()]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(role_names(&body), ["user", "support"]);

    // Self actions, out-of-scope targets and missing permissions are refused.
    let (status, _headers, problem) =
        put(admin_token.clone(), admin_id, vec![support.to_string()]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "self_action_forbidden");

    let (status, _headers, problem) =
        put(admin_token.clone(), root_id, vec![support.to_string()]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "out_of_scope");

    let plain_token = login_token(&app, "plain").await;
    let (status, _headers, problem) = put(plain_token, alice_id, vec![support.to_string()]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");

    // Role replacement always requires a fresh password authentication.
    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at = now() - interval '301 seconds' WHERE user_id = $1",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("age session");
    let (status, _headers, problem) =
        put(admin_token.clone(), alice_id, vec![support.to_string()]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "reauth_required");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            &admin_token,
            Some(&json!({ "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fresh_token = body["access_token"].as_str().expect("token").to_owned();
    let (status, _headers, body) = put(fresh_token, alice_id, vec![support.to_string()]).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A super administrator may grant a privileged role; the change is
    // critical, and `super_admin_involved` tracks the super_admin role
    // specifically.
    let root_token = login_token(&app, "root1").await;
    let (status, _headers, body) = put(
        root_token.clone(),
        alice_id,
        vec![admin_role.to_string(), support.to_string()],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(role_names(&body), ["admin", "user", "support"]);

    let (severity, details): (String, Value) = sqlx::query_as(
        "SELECT severity, details FROM audit_logs
         WHERE action = 'user_roles_changed'
         ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("privileged role audit");
    assert_eq!(severity, "critical");
    assert_eq!(details["super_admin_involved"], false);
    let (added, _removed) = added_removed(&details);
    let admin_role = admin_role.to_string();
    assert!(added.contains(&admin_role.as_str()), "{details}");

    let super_admin_role = builtin_role_id(&pool, "super_admin").await;
    let (status, _headers, body) = put(
        root_token,
        alice_id,
        vec![
            admin_role,
            super_admin_role.to_string(),
            support.to_string(),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        role_names(&body),
        ["admin", "super_admin", "user", "support"]
    );
    let (severity, details): (String, Value) = sqlx::query_as(
        "SELECT severity, details FROM audit_logs
         WHERE action = 'user_roles_changed'
         ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("super-admin role audit");
    assert_eq!(severity, "critical");
    assert_eq!(details["super_admin_involved"], true);
}

fn status_audit(details: &Value) -> (&str, &str, i64) {
    (
        details["from"].as_str().expect("from"),
        details["to"].as_str().expect("to"),
        details["sessions_revoked_count"]
            .as_i64()
            .expect("sessions_revoked_count"),
    )
}

#[sqlx::test(migrations = "../../migrations")]
async fn set_status_disables_sessions_and_reenables_login(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let admin_token = login_token(&app, "admin1").await;
    let alice_token = login_token(&app, "alice").await;
    let _second = login_token(&app, "alice").await;

    let version_before: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");

    let put = |status: &'static str| {
        let app = app.clone();
        let token = admin_token.clone();
        async move {
            send(
                app,
                authorized_request(
                    "PUT",
                    &format!("/api/v1/users/{alice_id}/status"),
                    &token,
                    Some(&json!({ "status": status })),
                ),
            )
            .await
        }
    };

    let (status, _headers, body) = put("disabled").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "disabled");

    let version_after: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(version_after, version_before + 1);

    let unrevoked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(alice_id)
    .fetch_one(&pool)
    .await
    .expect("sessions");
    assert_eq!(unrevoked, 0);
    let unrevoked_tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_refresh_tokens t
         JOIN auth_sessions s ON s.id = t.session_id
         WHERE s.user_id = $1 AND t.revoked_at IS NULL",
    )
    .bind(alice_id)
    .fetch_one(&pool)
    .await
    .expect("refresh tokens");
    assert_eq!(unrevoked_tokens, 0);
    // Disabling uses the dedicated revocation reason (spec §5.9).
    let reasons: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT revoke_reason FROM auth_sessions WHERE user_id = $1")
            .bind(alice_id)
            .fetch_all(&pool)
            .await
            .expect("revoke reasons");
    assert_eq!(reasons, ["account_disabled"]);

    // The disabled account is refused by the per-request pipeline even though
    // its access token has not expired (spec §5.9).
    let (status, _headers, problem) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &alice_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "account_disabled");

    let (severity, details): (String, Value) = sqlx::query_as(
        "SELECT severity, details FROM audit_logs WHERE action = 'user_status_changed'",
    )
    .fetch_one(&pool)
    .await
    .expect("status audit");
    assert_eq!(severity, "warning");
    assert_eq!(status_audit(&details), ("active", "disabled", 2));

    // Repeating the same status is idempotent and changes nothing.
    let (status, _headers, body) = put("disabled").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "disabled");
    let version_same: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(version_same, version_after);
    let status_events: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = 'user_status_changed'")
            .fetch_one(&pool)
            .await
            .expect("status events");
    assert_eq!(status_events, 1);

    // Enabling restores login without touching the token version.
    let (status, _headers, body) = put("active").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "active");
    let version_enabled: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(version_enabled, version_after);

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let events: Vec<Value> = sqlx::query_scalar(
        "SELECT details FROM audit_logs
         WHERE action = 'user_status_changed' ORDER BY chain_seq",
    )
    .fetch_all(&pool)
    .await
    .expect("status events");
    assert_eq!(status_audit(&events[1]), ("disabled", "active", 0));
}

#[sqlx::test(migrations = "../../migrations")]
async fn set_status_validates_input_authorization_and_step_up(pool: PgPool) {
    let app = app(&pool).await;
    let admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let root_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;

    let admin_token = login_token(&app, "admin1").await;
    let put = |token: String, target: Uuid, status: &'static str| {
        let app = app.clone();
        async move {
            send(
                app,
                authorized_request(
                    "PUT",
                    &format!("/api/v1/users/{target}/status"),
                    &token,
                    Some(&json!({ "status": status })),
                ),
            )
            .await
        }
    };

    let (status, _headers, problem) = put(admin_token.clone(), alice_id, "suspended").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "validation_failed");
    assert_eq!(problem["errors"][0]["pointer"], "/status");

    let (status, _headers, problem) = put(admin_token.clone(), admin_id, "disabled").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "self_action_forbidden");

    let (status, _headers, problem) = put(admin_token.clone(), root_id, "disabled").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "out_of_scope");

    let plain_token = login_token(&app, "plain").await;
    let (status, _headers, problem) = put(plain_token, alice_id, "disabled").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");

    // Status changes always require step-up.
    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at = now() - interval '301 seconds' WHERE user_id = $1",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("age session");
    let (status, _headers, problem) = put(admin_token.clone(), alice_id, "disabled").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "reauth_required");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            &admin_token,
            Some(&json!({ "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fresh_token = body["access_token"].as_str().expect("token").to_owned();
    let (status, _headers, body) = put(fresh_token, alice_id, "disabled").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "disabled");
}

#[sqlx::test(migrations = "../../migrations")]
async fn reset_password_issues_a_temporary_password_and_revokes_sessions(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let admin_token = login_token(&app, "admin1").await;
    let _first = login_token(&app, "alice").await;
    let _second = login_token(&app, "alice").await;
    sqlx::query(
        "UPDATE users SET failed_login_count = 5,
                          locked_until = now() + interval '10 minutes'
         WHERE id = $1",
    )
    .bind(alice_id)
    .execute(&pool)
    .await
    .expect("lock alice");
    let version_before: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");

    let reset = |token: String, payload: Value| {
        let app = app.clone();
        async move {
            send(
                app,
                authorized_request(
                    "POST",
                    &format!("/api/v1/users/{alice_id}/reset-password"),
                    &token,
                    Some(&payload),
                ),
            )
            .await
        }
    };

    let (status, _headers, body) = reset(admin_token.clone(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let temporary_password = body["temporary_password"]
        .as_str()
        .expect("temporary password");
    assert_eq!(temporary_password.len(), 24, "{temporary_password}");
    assert_eq!(body["user"]["must_change_password"], true);
    assert_eq!(body["user"]["status"], "active");

    let expires_at = body["temporary_password_expires_at"]
        .as_str()
        .expect("expiry");
    let expires_at = chrono::DateTime::parse_from_rfc3339(expires_at)
        .expect("rfc3339")
        .with_timezone(&Utc);
    let ttl = expires_at - Utc::now();
    assert!(
        ttl > Duration::hours(71) && ttl <= Duration::hours(73),
        "expected a 72 hour expiry, got {ttl}"
    );

    let (status, failed_count, locked_until, version_after, must_change): (
        String,
        i32,
        Option<chrono::DateTime<Utc>>,
        i64,
        bool,
    ) = sqlx::query_as(
        "SELECT status, failed_login_count, locked_until, token_version, must_change_password
         FROM users WHERE id = $1",
    )
    .bind(alice_id)
    .fetch_one(&pool)
    .await
    .expect("reset user row");
    assert_eq!(status, "active");
    assert_eq!(failed_count, 0);
    assert_eq!(locked_until, None);
    assert_eq!(version_after, version_before + 1);
    assert!(must_change);

    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("stored hash");
    assert!(stored.starts_with("$argon2id$"));
    assert!(!stored.contains(temporary_password));

    let unrevoked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(alice_id)
    .fetch_one(&pool)
    .await
    .expect("sessions");
    assert_eq!(unrevoked, 0);

    let (severity, details): (String, Value) = sqlx::query_as(
        "SELECT severity, details FROM audit_logs WHERE action = 'user_password_reset'",
    )
    .fetch_one(&pool)
    .await
    .expect("reset audit");
    assert_eq!(severity, "critical");
    assert_eq!(details["method"], "generated");
    assert_eq!(details["sessions_revoked_count"], 2);
    assert!(details["temporary_password_expires_at"].as_str().is_some());
    assert!(!details.to_string().contains(temporary_password));

    // The temporary password authenticates into a restricted session.
    let (status, _headers, login_body) = login(&app, "alice", temporary_password).await;
    assert_eq!(status, StatusCode::OK, "{login_body}");
    assert_eq!(login_body["user"]["must_change_password"], true);

    // A manual password is used verbatim and audited as `manual`.
    let manual = "manual temporary passphrase";
    let (status, _headers, body) = reset(admin_token.clone(), json!({ "password": manual })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["temporary_password"], manual);
    let method: String = sqlx::query_scalar(
        "SELECT details->>'method' FROM audit_logs
         WHERE action = 'user_password_reset' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("manual reset audit");
    assert_eq!(method, "manual");

    // A manual password must satisfy the policy.
    let (status, _headers, problem) =
        reset(admin_token.clone(), json!({ "password": "short" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "password_policy_violation");
    assert_eq!(problem["errors"][0]["pointer"], "/password");
    assert_eq!(problem["errors"][0]["params"]["reason"], "too_short");

    // Resetting a disabled user must not silently re-enable the account.
    sqlx::query("UPDATE users SET status = 'disabled' WHERE id = $1")
        .bind(alice_id)
        .execute(&pool)
        .await
        .expect("disable alice");
    let (status, _headers, body) = reset(admin_token, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["status"], "disabled");
}

#[sqlx::test(migrations = "../../migrations")]
async fn reset_password_requires_step_up_and_authorization(pool: PgPool) {
    let app = app(&pool).await;
    let admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let root_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;

    let admin_token = login_token(&app, "admin1").await;
    let reset = |token: String, target: Uuid| {
        let app = app.clone();
        async move {
            send(
                app,
                authorized_request(
                    "POST",
                    &format!("/api/v1/users/{target}/reset-password"),
                    &token,
                    Some(&json!({})),
                ),
            )
            .await
        }
    };

    let (status, _headers, problem) = reset(admin_token.clone(), admin_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "self_action_forbidden");

    let (status, _headers, problem) = reset(admin_token.clone(), root_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "out_of_scope");

    let plain_token = login_token(&app, "plain").await;
    let (status, _headers, problem) = reset(plain_token, alice_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");

    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at = now() - interval '301 seconds' WHERE user_id = $1",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("age session");
    let (status, _headers, problem) = reset(admin_token.clone(), alice_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "reauth_required");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            &admin_token,
            Some(&json!({ "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fresh_token = body["access_token"].as_str().expect("token").to_owned();
    let (status, _headers, body) = reset(fresh_token, alice_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn unlock_clears_the_account_lock_idempotently_without_step_up(pool: PgPool) {
    let app = app(&pool).await;
    let admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let root_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    sqlx::query(
        "UPDATE users SET failed_login_count = 5,
                          locked_until = now() + interval '10 minutes',
                          last_failed_login_at = now()
         WHERE id = $1",
    )
    .bind(alice_id)
    .execute(&pool)
    .await
    .expect("lock alice");

    let admin_token = login_token(&app, "admin1").await;
    let unlock = |token: String, target: Uuid| {
        let app = app.clone();
        async move {
            send(
                app,
                authorized_request(
                    "POST",
                    &format!("/api/v1/users/{target}/unlock"),
                    &token,
                    None,
                ),
            )
            .await
        }
    };

    let (status, _headers, body) = unlock(admin_token.clone(), alice_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["locked_until"], Value::Null);

    let (failed_count, locked_until): (i32, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT failed_login_count, locked_until FROM users WHERE id = $1")
            .bind(alice_id)
            .fetch_one(&pool)
            .await
            .expect("unlocked row");
    assert_eq!(failed_count, 0);
    assert_eq!(locked_until, None);

    let (severity, details): (String, Value) =
        sqlx::query_as("SELECT severity, details FROM audit_logs WHERE action = 'user_unlocked'")
            .fetch_one(&pool)
            .await
            .expect("unlock audit");
    assert_eq!(severity, "warning");
    assert_eq!(details["cleared_failure_count"], 5);

    // A stale session may still unlock: the action does not require step-up.
    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at = now() - interval '301 seconds' WHERE user_id = $1",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("age session");
    let (status, _headers, body) = unlock(admin_token.clone(), alice_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let latest: i64 = sqlx::query_scalar(
        "SELECT (details->>'cleared_failure_count')::bigint FROM audit_logs
         WHERE action = 'user_unlocked' ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("idempotent unlock audit");
    assert_eq!(latest, 0);

    // Self, out-of-scope and unprivileged callers are refused.
    let (status, _headers, problem) = unlock(admin_token.clone(), admin_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "self_action_forbidden");

    let (status, _headers, problem) = unlock(admin_token.clone(), root_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "out_of_scope");

    let plain_token = login_token(&app, "plain").await;
    let (status, _headers, problem) = unlock(plain_token, alice_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");
}

#[sqlx::test(migrations = "../../migrations")]
async fn revoke_sessions_invalidates_every_session_and_token(pool: PgPool) {
    let app = app(&pool).await;
    let admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let root_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    let admin_token = login_token(&app, "admin1").await;
    let alice_token = login_token(&app, "alice").await;
    let _second = login_token(&app, "alice").await;
    let version_before: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");

    let revoke = |token: String, target: Uuid| {
        let app = app.clone();
        async move {
            send(
                app,
                authorized_request(
                    "DELETE",
                    &format!("/api/v1/users/{target}/sessions"),
                    &token,
                    None,
                ),
            )
            .await
        }
    };

    let (status, _headers, body) = revoke(admin_token.clone(), alice_id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(body, Value::Null);

    let version_after: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(version_after, version_before + 1);
    let unrevoked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(alice_id)
    .fetch_one(&pool)
    .await
    .expect("sessions");
    assert_eq!(unrevoked, 0);
    let unrevoked_tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_refresh_tokens t
         JOIN auth_sessions s ON s.id = t.session_id
         WHERE s.user_id = $1 AND t.revoked_at IS NULL",
    )
    .bind(alice_id)
    .fetch_one(&pool)
    .await
    .expect("refresh tokens");
    assert_eq!(unrevoked_tokens, 0);

    let (status, _headers, problem) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &alice_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{problem}");
    assert_eq!(problem_code(&problem), "session_invalid");

    let (severity, details): (String, Value) = sqlx::query_as(
        "SELECT severity, details FROM audit_logs WHERE action = 'user_sessions_revoked'",
    )
    .fetch_one(&pool)
    .await
    .expect("revoke audit");
    assert_eq!(severity, "warning");
    assert_eq!(details["sessions_revoked_count"], 2);

    // Self, out-of-scope and unprivileged callers are refused.
    let (status, _headers, problem) = revoke(admin_token.clone(), admin_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "self_action_forbidden");

    let (status, _headers, problem) = revoke(admin_token.clone(), root_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "out_of_scope");

    let plain_token = login_token(&app, "plain").await;
    let (status, _headers, problem) = revoke(plain_token, alice_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");

    // Session revocation always requires step-up.
    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at = now() - interval '301 seconds' WHERE user_id = $1",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("age session");
    let (status, _headers, problem) = revoke(admin_token.clone(), alice_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "reauth_required");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            &admin_token,
            Some(&json!({ "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fresh_token = body["access_token"].as_str().expect("token").to_owned();
    let (status, _headers, _body) = revoke(fresh_token, alice_id).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_disables_cannot_remove_the_last_enabled_super_admin(pool: PgPool) {
    let app = app(&pool).await;
    let alice_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let bob_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root2", "root2@example.com")
        },
    )
    .await;
    let alice_token = login_token(&app, "root1").await;
    let bob_token = login_token(&app, "root2").await;

    // Each super administrator tries to disable the other at the same time.
    // The row locks serialize the invariant check: one transition wins and the
    // other observes a single enabled super administrator.
    let disable = |token: String, target: Uuid| {
        let app = app.clone();
        async move {
            send(
                app,
                authorized_request(
                    "PUT",
                    &format!("/api/v1/users/{target}/status"),
                    &token,
                    Some(&json!({ "status": "disabled" })),
                ),
            )
            .await
        }
    };
    let ((first_status, _first_headers, first_body), (second_status, _second_headers, second_body)) =
        tokio::join!(disable(alice_token, bob_id), disable(bob_token, alice_id));

    let statuses = [first_status, second_status];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1,
        "exactly one disable must win: {first_body} / {second_body}"
    );
    let loser = if first_status == StatusCode::OK {
        (&second_status, &second_body)
    } else {
        (&first_status, &first_body)
    };
    assert!(
        matches!(
            *loser.0,
            StatusCode::CONFLICT | StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED
        ),
        "loser must be refused: {} {}",
        loser.0,
        loser.1
    );
    if *loser.0 == StatusCode::CONFLICT {
        assert_eq!(problem_code(loser.1), "last_super_admin");
    }

    let enabled_super_admins: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT u.id)
         FROM users u
         JOIN user_roles ur ON ur.user_id = u.id
         JOIN roles r ON r.id = ur.role_id
         WHERE u.status = 'active' AND r.name = 'super_admin' AND r.is_builtin",
    )
    .fetch_one(&pool)
    .await
    .expect("enabled super admins");
    assert_eq!(enabled_super_admins, 1);
}

/// Count lock requests in this test's database that are still waiting.
async fn waiting_lock_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_locks l
         WHERE NOT granted
           AND (
               l.database = (SELECT oid FROM pg_database WHERE datname = current_database())
               OR (
                   l.locktype = 'transactionid'
                   AND l.pid IN (
                       SELECT pid FROM pg_stat_activity
                       WHERE datname = current_database()
                   )
               )
           )",
    )
    .fetch_one(pool)
    .await
    .expect("lock count")
}

/// Wait until `expected` lock requests are blocked, so the test observes a
/// specific interleaving instead of relying on timing.
async fn wait_for_waiting_locks(pool: &PgPool, expected: i64) {
    for _ in 0..500 {
        if waiting_lock_count(pool).await >= expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {expected} blocked lock requests");
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_demotions_cannot_remove_the_last_enabled_super_admin(pool: PgPool) {
    let app = app(&pool).await;
    let alice_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let bob_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root2", "root2@example.com")
        },
    )
    .await;
    let baseline = builtin_role_id(&pool, "user").await.to_string();
    let alice_token = login_token(&app, "root1").await;
    let bob_token = login_token(&app, "root2").await;

    let demote = |app: Router, token: String, target: Uuid| {
        let baseline = baseline.clone();
        async move {
            send(
                app,
                authorized_request(
                    "PUT",
                    &format!("/api/v1/users/{target}/roles"),
                    &token,
                    Some(&json!({ "role_ids": [baseline] })),
                ),
            )
            .await
        }
    };

    // Holding the baseline role row makes the first request block after it has
    // acquired the enabled-super-admin user row locks. The second request then
    // queues on those locks, which reproduces the stale-snapshot interleaving:
    // a role change only touches `user_roles`, not the locked `users` rows.
    let mut control = pool.begin().await.expect("control transaction");
    sqlx::query("SELECT id FROM roles WHERE name = 'user' FOR UPDATE")
        .fetch_one(&mut *control)
        .await
        .expect("lock baseline role");

    let first = tokio::spawn(demote(app.clone(), alice_token, bob_id));
    for _ in 0..500 {
        if first.is_finished() {
            let result = first.await.expect("first join");
            panic!("first request finished before blocking: {result:?}");
        }
        if waiting_lock_count(&pool).await >= 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let second = tokio::spawn(demote(app.clone(), bob_token, alice_id));
    wait_for_waiting_locks(&pool, 2).await;
    control.commit().await.expect("release baseline role");

    let timeout = std::time::Duration::from_secs(10);
    let (first_status, _first_headers, first_body) = tokio::time::timeout(timeout, first)
        .await
        .expect("first request timed out")
        .expect("first request joined");
    let (second_status, _second_headers, second_body) = tokio::time::timeout(timeout, second)
        .await
        .expect("second request timed out")
        .expect("second request joined");

    let statuses = [first_status, second_status];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1,
        "exactly one demotion must win: {first_body} / {second_body}"
    );
    let loser = if first_status == StatusCode::OK {
        (&second_status, &second_body)
    } else {
        (&first_status, &first_body)
    };
    assert!(
        matches!(*loser.0, StatusCode::CONFLICT | StatusCode::FORBIDDEN),
        "loser must be refused: {} {}",
        loser.0,
        loser.1
    );
    if *loser.0 == StatusCode::CONFLICT {
        assert_eq!(problem_code(loser.1), "last_super_admin");
    }

    let enabled_super_admins: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT u.id)
         FROM users u
         JOIN user_roles ur ON ur.user_id = u.id
         JOIN roles r ON r.id = ur.role_id
         WHERE u.status = 'active' AND r.name = 'super_admin' AND r.is_builtin",
    )
    .fetch_one(&pool)
    .await
    .expect("enabled super admins");
    assert_eq!(enabled_super_admins, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn role_changes_apply_on_the_next_request_without_invalidating_sessions(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let reader_role = insert_custom_role(&pool, "reader", &["users.read"]).await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let baseline = builtin_role_id(&pool, "user").await;

    let alice_token = login_token(&app, "alice").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/users", &alice_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (token_version_before,): (i64,) =
        sqlx::query_as("SELECT token_version FROM users WHERE id = $1")
            .bind(alice_id)
            .fetch_one(&pool)
            .await
            .expect("token version");
    let sessions_before = session_count(&pool, alice_id).await;

    let root_token = login_token(&app, "root1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PUT",
            &format!("/api/v1/users/{alice_id}/roles"),
            &root_token,
            Some(&json!({ "role_ids": [reader_role.to_string()] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The same access token sees the new permission on its very next request.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/users", &alice_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PUT",
            &format!("/api/v1/users/{alice_id}/roles"),
            &root_token,
            Some(&json!({ "role_ids": [baseline.to_string()] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/users", &alice_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // RBAC changes are not session or token events (spec §6.2).
    let (token_version_after,): (i64,) =
        sqlx::query_as("SELECT token_version FROM users WHERE id = $1")
            .bind(alice_id)
            .fetch_one(&pool)
            .await
            .expect("token version");
    assert_eq!(token_version_after, token_version_before);
    assert_eq!(session_count(&pool, alice_id).await, sessions_before);
}
