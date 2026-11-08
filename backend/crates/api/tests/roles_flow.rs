//! Roles and permissions API behaviour over HTTP with a real PostgreSQL schema
//! (docs/specs/login-system.md §6.3–§6.5, §7.8).

mod auth_support;

use auth_support::{NewUser, PASSWORD, app, authorized_request, insert_user, login, problem_code};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
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

/// Insert a custom role and its permission rows directly; role writes are
/// exercised through HTTP in the tests that care about them.
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

/// The user with a custom management role: roles.read/create/update/delete,
/// but no audit.read and no user permissions.
async fn management_user(pool: &PgPool) -> Uuid {
    let user_id = insert_user(pool, NewUser::active("manager", "manager@example.com")).await;
    let role_id = insert_custom_role(
        pool,
        "role_manager",
        &["roles.read", "roles.create", "roles.update", "roles.delete"],
    )
    .await;
    assign_role(pool, user_id, role_id).await;
    user_id
}

fn item_named<'a>(body: &'a Value, name: &str) -> &'a Value {
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("role {name} missing from {body}"))
}

async fn audit_count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("audit count")
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_roles_returns_target_eligibility_without_scope_filtering(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    insert_custom_role(&pool, "auditors", &["audit.read"]).await;
    insert_custom_role(&pool, "role-makers", &["roles.create"]).await;

    // A plain administrator sees every active role, including ones it cannot
    // assign or manage.
    let token = login_token(&app, "admin1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?page_size=200", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 5);
    assert_eq!(body["page"], 1);
    assert_eq!(body["page_size"], 200);

    let user = item_named(&body, "user");
    assert_eq!(user["is_builtin"], true);
    assert_eq!(user["assignable"], true);
    assert_eq!(user["manageable"], false);

    for builtin in ["admin", "super_admin"] {
        let role = item_named(&body, builtin);
        assert_eq!(role["is_builtin"], true);
        assert_eq!(role["assignable"], false, "{builtin}");
        assert_eq!(role["manageable"], false, "{builtin}");
    }

    let within = item_named(&body, "auditors");
    assert_eq!(within["is_builtin"], false);
    assert_eq!(within["assignable"], true);
    assert_eq!(within["manageable"], true);
    assert_eq!(within["permission_keys"], json!(["audit.read"]));

    let beyond = item_named(&body, "role-makers");
    assert_eq!(beyond["assignable"], false);
    assert_eq!(beyond["manageable"], false);

    // The super administrator can assign every role, but built-ins stay
    // unmanageable.
    let root_token = login_token(&app, "root1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?page_size=200", &root_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(item_named(&body, "admin")["assignable"], true);
    assert_eq!(item_named(&body, "super_admin")["assignable"], true);
    assert_eq!(item_named(&body, "role-makers")["assignable"], true);
    assert_eq!(item_named(&body, "role-makers")["manageable"], true);
    assert_eq!(item_named(&body, "admin")["manageable"], false);
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_roles_supports_search_filters_sort_and_pagination(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    insert_custom_role(&pool, "alpha", &["users.read"]).await;
    insert_custom_role(&pool, "beta", &["users.read"]).await;
    insert_custom_role(&pool, "gamma", &["users.read"]).await;
    sqlx::query("UPDATE roles SET description = 'Handles 100% of audits' WHERE name = 'gamma'")
        .execute(&pool)
        .await
        .expect("description");
    let token = login_token(&app, "admin1").await;

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?q=ALP", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["name"], "alpha");

    // `%` in the search term is literal, not a wildcard.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?q=100%25", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["name"], "gamma");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?is_builtin=true", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 3);
    assert!(
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["is_builtin"] == true)
    );

    // Default order is name ascending; page 2 of 2 shows the second pair.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?page=2&page_size=2", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 6);
    let names: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["beta", "gamma"]);

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?sort=-name&page_size=1", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"][0]["name"], "user");

    for (query, code) in [
        ("sort=bogus", "invalid_sort"),
        ("page_size=201", "page_size_too_large"),
        ("page=0", "validation_failed"),
        ("page_size=0", "validation_failed"),
        ("is_builtin=maybe", "validation_failed"),
    ] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request("GET", &format!("/api/v1/roles?{query}"), &token, None),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {body}");
        assert_eq!(problem_code(&body), code, "{query}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn role_detail_is_returned_or_404(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let role_id = insert_custom_role(&pool, "auditors", &["audit.read"]).await;
    let token = login_token(&app, "admin1").await;

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", &format!("/api/v1/roles/{role_id}"), &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], role_id.to_string());
    assert_eq!(body["name"], "auditors");
    assert!(body["description"].is_null());
    assert_eq!(body["permission_keys"], json!(["audit.read"]));
    assert_eq!(body["assignable"], true);
    assert_eq!(body["manageable"], true);
    assert!(body["created_at"].as_str().unwrap().ends_with('Z'));

    for id in [Uuid::now_v7().to_string(), "not-a-uuid".to_owned()] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request("GET", &format!("/api/v1/roles/{id}"), &token, None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(problem_code(&body), "not_found");
    }

    sqlx::query("UPDATE roles SET deleted_at = now() WHERE id = $1")
        .bind(role_id)
        .execute(&pool)
        .await
        .expect("soft delete");
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", &format!("/api/v1/roles/{role_id}"), &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn creating_a_custom_role_validates_returns_location_and_writes_audit(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "root1").await;

    let (status, headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/roles",
            &token,
            Some(&json!({
                "name": "  Auditors  ",
                "description": "Reads the audit trail",
                "permission_keys": ["users.read", "audit.read", "audit.read"]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let role_id = body["id"].as_str().expect("role id").to_owned();
    assert_eq!(
        headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("/api/v1/roles/{role_id}").as_str())
    );
    assert_eq!(body["name"], "Auditors");
    assert_eq!(body["description"], "Reads the audit trail");
    assert_eq!(body["is_builtin"], false);
    assert_eq!(body["permission_keys"], json!(["audit.read", "users.read"]));
    assert_eq!(body["assignable"], true);
    assert_eq!(body["manageable"], true);

    let (stored_name, is_builtin): (String, bool) =
        sqlx::query_as("SELECT name, is_builtin FROM roles WHERE id = $1")
            .bind(Uuid::parse_str(&role_id).unwrap())
            .fetch_one(&pool)
            .await
            .expect("stored role");
    assert_eq!(stored_name, "Auditors");
    assert!(!is_builtin);

    let (action, severity, result, target_type, target_id, details): (
        String,
        String,
        String,
        String,
        Option<Uuid>,
        Value,
    ) = sqlx::query_as(
        "SELECT action, severity, result, target_type, target_id, details
         FROM audit_logs WHERE action = 'role_created'",
    )
    .fetch_one(&pool)
    .await
    .expect("role_created audit");
    assert_eq!(action, "role_created");
    assert_eq!(severity, "warning");
    assert_eq!(result, "success");
    assert_eq!(target_type, "role");
    assert_eq!(target_id, Some(Uuid::parse_str(&role_id).unwrap()));
    assert_eq!(details["role_name"], "Auditors");
    assert_eq!(
        details["permission_keys"],
        json!(["audit.read", "users.read"])
    );

    // Active names are unique case-insensitively, and built-in names are
    // reserved because the built-in rows occupy them.
    for name in ["auditors", "ADMIN", "super_admin"] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request(
                "POST",
                "/api/v1/roles",
                &token,
                Some(&json!({
                    "name": name,
                    "permission_keys": ["users.read"]
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{name}: {body}");
        assert_eq!(problem_code(&body), "role_name_taken");
    }

    for (payload, code) in [
        (
            json!({"name": "empty", "permission_keys": []}),
            "empty_permissions",
        ),
        (
            json!({"name": "bad", "permission_keys": ["not.a.permission"]}),
            "invalid_permission",
        ),
        (
            json!({"name": "", "permission_keys": ["users.read"]}),
            "validation_failed",
        ),
        (
            json!({"name": "x".repeat(65), "permission_keys": ["users.read"]}),
            "validation_failed",
        ),
        (
            json!({"name": "long", "description": "x".repeat(501), "permission_keys": ["users.read"]}),
            "validation_failed",
        ),
        (json!({"name": "missing"}), "validation_failed"),
        (
            json!({"name": "unknown", "permission_keys": ["users.read"], "is_builtin": false}),
            "unknown_field",
        ),
    ] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request("POST", "/api/v1/roles", &token, Some(&payload)),
        )
        .await;
        assert_eq!(status.as_u16(), body_status(code), "{body}");
        assert_eq!(problem_code(&body), code);
    }
}

fn body_status(code: &str) -> u16 {
    match code {
        "unknown_field" => 400,
        "validation_failed" | "empty_permissions" | "invalid_permission" => 422,
        _ => unreachable!("unexpected code {code}"),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn builtin_roles_reject_every_write_with_role_immutable(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "root1").await;
    let admin_id: Uuid = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'admin'")
        .fetch_one(&pool)
        .await
        .expect("admin role");

    let requests = [
        (
            "PATCH",
            format!("/api/v1/roles/{admin_id}"),
            Some(json!({ "name": "renamed" })),
        ),
        (
            "PUT",
            format!("/api/v1/roles/{admin_id}/permissions"),
            Some(json!({ "permission_keys": ["users.read"] })),
        ),
        ("DELETE", format!("/api/v1/roles/{admin_id}"), None),
    ];
    for (method, uri, body) in requests {
        let (status, _headers, problem) = send(
            app.clone(),
            authorized_request(method, &uri, &token, body.as_ref()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {problem}");
        assert_eq!(problem_code(&problem), "role_immutable");
    }

    let denied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied'
           AND details->>'reason_code' = 'role_immutable'
           AND target_type = 'role' AND target_id = $1",
    )
    .bind(admin_id)
    .fetch_one(&pool)
    .await
    .expect("denied audit");
    assert_eq!(denied, 3);
}

#[sqlx::test(migrations = "../../migrations")]
async fn non_super_admins_cannot_grant_beyond_their_own_permissions(pool: PgPool) {
    let app = app(&pool).await;
    management_user(&pool).await;
    let beyond = insert_custom_role(&pool, "auditors", &["audit.read"]).await;
    let token = login_token(&app, "manager").await;

    // Create: requested set beyond the operator.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/roles",
            &token,
            Some(&json!({
                "name": "new-role",
                "permission_keys": ["roles.read", "audit.read"]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(problem_code(&body), "permission_subset_violation");

    // A subset of the operator's own permissions is allowed.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/roles",
            &token,
            Some(&json!({
                "name": "new-role",
                "permission_keys": ["roles.read"]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let created_id = body["id"].as_str().unwrap().to_owned();

    // Metadata edit and delete of a role holding permissions the operator
    // lacks.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{beyond}"),
            &token,
            Some(&json!({ "name": "auditors-renamed" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(problem_code(&body), "permission_subset_violation");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("DELETE", &format!("/api/v1/roles/{beyond}"), &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(problem_code(&body), "permission_subset_violation");

    // Permission replacement may not add a permission the operator lacks.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PUT",
            &format!("/api/v1/roles/{created_id}/permissions"),
            &token,
            Some(&json!({ "permission_keys": ["roles.read", "audit.read"] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(problem_code(&body), "permission_subset_violation");

    // The operator can edit a role inside its own permission set.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PUT",
            &format!("/api/v1/roles/{created_id}/permissions"),
            &token,
            Some(&json!({ "permission_keys": ["roles.update", "roles.delete"] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["permission_keys"],
        json!(["roles.delete", "roles.update"])
    );

    let denied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied'
           AND details->>'reason_code' = 'permission_subset_violation'",
    )
    .fetch_one(&pool)
    .await
    .expect("subset audit");
    assert_eq!(denied, 4);
}

#[sqlx::test(migrations = "../../migrations")]
async fn every_write_requires_recent_authentication(pool: PgPool) {
    let app = app(&pool).await;
    let root_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let role_id = insert_custom_role(&pool, "auditors", &["audit.read"]).await;
    let token = login_token(&app, "root1").await;

    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at = now() - interval '301 seconds'
         WHERE user_id = $1",
    )
    .bind(root_id)
    .execute(&pool)
    .await
    .expect("age the session");

    for (method, uri, body) in [
        (
            "POST",
            "/api/v1/roles".to_owned(),
            Some(json!({"name": "fresh", "permission_keys": ["audit.read"]})),
        ),
        (
            "PATCH",
            format!("/api/v1/roles/{role_id}"),
            Some(json!({ "name": "auditors-2" })),
        ),
        (
            "PUT",
            format!("/api/v1/roles/{role_id}/permissions"),
            Some(json!({ "permission_keys": ["users.read"] })),
        ),
        ("DELETE", format!("/api/v1/roles/{role_id}"), None),
    ] {
        let (status, _headers, problem) = send(
            app.clone(),
            authorized_request(method, &uri, &token, body.as_ref()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {problem}");
        assert_eq!(problem_code(&problem), "reauth_required");
        assert_eq!(problem["details"]["max_age"], 300);
    }

    let reauth_denials: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_step_up_denied'
           AND details->>'reason_code' = 'reauth_required'
           AND details->>'max_age_seconds' = '300'",
    )
    .fetch_one(&pool)
    .await
    .expect("reauthentication audit");
    assert_eq!(reauth_denials, 4);

    // Reauthenticate and retry the same edit with the reissued token.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/reauthenticate",
            &token,
            Some(&json!({ "password": PASSWORD })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fresh_token = body["access_token"].as_str().expect("token").to_owned();

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{role_id}"),
            &fresh_token,
            Some(&json!({ "name": "auditors-2" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "auditors-2");
}

#[sqlx::test(migrations = "../../migrations")]
async fn role_metadata_and_permissions_update_with_audit(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let role_id = insert_custom_role(&pool, "auditors", &["users.read"]).await;
    insert_custom_role(&pool, "operators", &["users.read"]).await;
    let token = login_token(&app, "root1").await;

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{role_id}"),
            &token,
            Some(&json!({ "name": "audit-team", "description": "Reviews events" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "audit-team");
    assert_eq!(body["description"], "Reviews events");
    assert_eq!(body["permission_keys"], json!(["users.read"]));

    let (changed, target_label): (Value, Option<String>) = sqlx::query_as(
        "SELECT details->'changed_fields', target_label
         FROM audit_logs WHERE action = 'role_updated'",
    )
    .fetch_one(&pool)
    .await
    .expect("role_updated audit");
    assert_eq!(changed, json!(["name", "description"]));
    assert_eq!(target_label.as_deref(), Some("audit-team"));

    // Explicit null clears the description.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{role_id}"),
            &token,
            Some(&json!({ "description": null })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["description"].is_null());

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{role_id}"),
            &token,
            Some(&json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(problem_code(&body), "empty_update");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{role_id}"),
            &token,
            Some(&json!({ "name": null })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(problem_code(&body), "validation_failed");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{role_id}"),
            &token,
            Some(&json!({ "name": "operators" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(problem_code(&body), "role_name_taken");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PUT",
            &format!("/api/v1/roles/{role_id}/permissions"),
            &token,
            Some(&json!({ "permission_keys": ["roles.read", "audit.read"] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["permission_keys"], json!(["audit.read", "roles.read"]));

    let (added, removed): (Value, Value) = sqlx::query_as(
        "SELECT details->'added_permission_keys', details->'removed_permission_keys'
         FROM audit_logs WHERE action = 'role_permissions_changed'",
    )
    .fetch_one(&pool)
    .await
    .expect("permissions audit");
    assert_eq!(added, json!(["audit.read", "roles.read"]));
    assert_eq!(removed, json!(["users.read"]));

    // Saving the same set is a no-op and writes no second audit event.
    let before = audit_count(&pool, "role_permissions_changed").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PUT",
            &format!("/api/v1/roles/{role_id}/permissions"),
            &token,
            Some(&json!({ "permission_keys": ["audit.read", "roles.read"] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(audit_count(&pool, "role_permissions_changed").await, before);

    for (payload, code) in [
        (json!({ "permission_keys": [] }), "empty_permissions"),
        (
            json!({ "permission_keys": ["no.such.permission"] }),
            "invalid_permission",
        ),
    ] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request(
                "PUT",
                &format!("/api/v1/roles/{role_id}/permissions"),
                &token,
                Some(&payload),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(problem_code(&body), code);
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_role_requires_no_assignments_and_is_soft(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let holder_id = insert_user(&pool, NewUser::active("holder", "holder@example.com")).await;
    let role_id = insert_custom_role(&pool, "auditors", &["audit.read"]).await;
    assign_role(&pool, holder_id, role_id).await;
    let token = login_token(&app, "root1").await;

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("DELETE", &format!("/api/v1/roles/{role_id}"), &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(problem_code(&body), "role_in_use");

    sqlx::query("DELETE FROM user_roles WHERE role_id = $1")
        .bind(role_id)
        .execute(&pool)
        .await
        .expect("unassign");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("DELETE", &format!("/api/v1/roles/{role_id}"), &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let deleted_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM roles WHERE id = $1")
            .bind(role_id)
            .fetch_one(&pool)
            .await
            .expect("deleted_at");
    assert!(deleted_at.is_some(), "delete must be soft");

    let (status, _headers, _body) = send(
        app.clone(),
        authorized_request("GET", &format!("/api/v1/roles/{role_id}"), &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A soft-deleted role is not a write target either.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{role_id}"),
            &token,
            Some(&json!({ "name": "ghost" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(problem_code(&body), "not_found");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?q=auditors", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 0);

    let (role_name, permission_keys): (String, Value) = sqlx::query_as(
        "SELECT details->>'role_name', details->'permission_keys'
         FROM audit_logs WHERE action = 'role_deleted'",
    )
    .fetch_one(&pool)
    .await
    .expect("role_deleted audit");
    assert_eq!(role_name, "auditors");
    assert_eq!(permission_keys, json!(["audit.read"]));

    // The soft-deleted name is reusable.
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/roles",
            &token,
            Some(&json!({
                "name": "AUDITORS",
                "permission_keys": ["audit.read"]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_ne!(body["id"], role_id.to_string());
}

#[sqlx::test(migrations = "../../migrations")]
async fn permission_catalog_requires_any_role_permission_and_computes_grantable(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let creator_id = insert_user(&pool, NewUser::active("creator", "creator@example.com")).await;
    let creator_role = insert_custom_role(&pool, "creator_role", &["roles.create"]).await;
    assign_role(&pool, creator_id, creator_role).await;

    // No roles.read/create/update: refused.
    let plain_token = login_token(&app, "plain").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/permissions", &plain_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(problem_code(&body), "forbidden");
    assert_eq!(audit_count(&pool, "authz_denied").await, 1);

    // Any one of the three permissions is enough.
    let creator_token = login_token(&app, "creator").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/permissions", &creator_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Administrator: full catalog, grantable reflects own permissions.
    let admin_token = login_token(&app, "admin1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/permissions", &admin_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let groups = body["groups"].as_array().expect("groups");
    let group_keys: Vec<&str> = groups
        .iter()
        .map(|group| group["key"].as_str().unwrap())
        .collect();
    assert_eq!(group_keys, ["users", "sessions", "roles", "audit"]);

    let items: Vec<&Value> = groups
        .iter()
        .flat_map(|group| group["permissions"].as_array().unwrap())
        .collect();
    assert_eq!(items.len(), 13);
    let role_item = |key: &str| {
        items
            .iter()
            .find(|item| item["key"] == key)
            .unwrap_or_else(|| panic!("missing permission {key}"))
    };
    assert_eq!(role_item("users.read")["grantable"], true);
    assert_eq!(role_item("roles.read")["grantable"], true);
    assert_eq!(role_item("audit.read")["grantable"], true);
    assert_eq!(role_item("roles.create")["grantable"], false);
    assert_eq!(role_item("roles.delete")["grantable"], false);
    assert_eq!(role_item("roles.read")["display_name_zh"], "查看角色");
    assert_eq!(role_item("roles.read")["display_name_en"], "View roles");

    // Super administrator: every permission is grantable.
    let root_token = login_token(&app, "root1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/permissions", &root_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["permissions"].as_array().unwrap())
            .all(|item| item["grantable"] == true)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn each_endpoint_requires_its_action_permission(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    let reader_id = insert_user(&pool, NewUser::active("reader", "reader@example.com")).await;
    let reader_role = insert_custom_role(&pool, "role_reader", &["roles.read"]).await;
    assign_role(&pool, reader_id, reader_role).await;
    let target = insert_custom_role(&pool, "target", &["users.read"]).await;

    let plain_token = login_token(&app, "plain").await;
    for (method, uri, body) in [
        ("GET", "/api/v1/roles".to_owned(), None),
        ("GET", format!("/api/v1/roles/{target}"), None),
        (
            "POST",
            "/api/v1/roles".to_owned(),
            Some(json!({ "name": "blocked", "permission_keys": ["users.read"] })),
        ),
        (
            "PATCH",
            format!("/api/v1/roles/{target}"),
            Some(json!({ "name": "blocked" })),
        ),
        (
            "PUT",
            format!("/api/v1/roles/{target}/permissions"),
            Some(json!({ "permission_keys": ["users.read"] })),
        ),
        ("DELETE", format!("/api/v1/roles/{target}"), None),
        ("GET", "/api/v1/permissions".to_owned(), None),
    ] {
        let (status, _headers, problem) = send(
            app.clone(),
            authorized_request(method, &uri, &plain_token, body.as_ref()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {problem}");
        assert_eq!(problem_code(&problem), "forbidden");
    }

    // Roles.read is enough for the reads but not for any write.
    let reader_token = login_token(&app, "reader").await;
    for uri in [
        "/api/v1/roles".to_owned(),
        format!("/api/v1/roles/{target}"),
    ] {
        let (status, _headers, body) = send(
            app.clone(),
            authorized_request("GET", &uri, &reader_token, None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }
    for (method, uri, body) in [
        (
            "POST",
            "/api/v1/roles".to_owned(),
            Some(json!({ "name": "blocked", "permission_keys": ["users.read"] })),
        ),
        (
            "PATCH",
            format!("/api/v1/roles/{target}"),
            Some(json!({ "name": "blocked" })),
        ),
        (
            "PUT",
            format!("/api/v1/roles/{target}/permissions"),
            Some(json!({ "permission_keys": ["users.read"] })),
        ),
        ("DELETE", format!("/api/v1/roles/{target}"), None),
    ] {
        let (status, _headers, problem) = send(
            app.clone(),
            authorized_request(method, &uri, &reader_token, body.as_ref()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {problem}");
        assert_eq!(problem_code(&problem), "forbidden");
    }

    // A missing-permission denial on a role path names that role as the audit
    // target, since the path id determines it (spec §8.5.2).
    let targeted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied'
           AND details->>'reason_code' = 'forbidden'
           AND details->>'operation' IN ('update_role', 'set_role_permissions', 'delete_role')
           AND target_type = 'role' AND target_id = $1",
    )
    .bind(target)
    .fetch_one(&pool)
    .await
    .expect("targeted denial audits");
    assert_eq!(
        targeted, 6,
        "each write denial on both principals names the role"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn legacy_permission_keys_make_a_role_ineligible_for_non_super_admins(pool: PgPool) {
    let app = app(&pool).await;
    management_user(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;

    // Simulate a permission removed from a later catalog: the row and grants
    // persist (spec §4.4), but nobody holds the key.
    sqlx::query(
        "INSERT INTO permissions (key, group_key, display_name_zh, display_name_en)
         VALUES ('legacy.read', 'legacy', '旧权限', 'Legacy permission')",
    )
    .execute(&pool)
    .await
    .expect("legacy permission");
    let legacy_role = insert_custom_role(&pool, "legacy-role", &["legacy.read"]).await;

    let manager_token = login_token(&app, "manager").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles?page_size=200", &manager_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let legacy = item_named(&body, "legacy-role");
    assert_eq!(legacy["permission_keys"], json!(["legacy.read"]));
    assert_eq!(legacy["assignable"], false);
    assert_eq!(legacy["manageable"], false);

    // The returned permission set is not a subset of the operator's, so the
    // subset rule refuses edits and deletes rather than trusting the dropped
    // key.
    for (method, uri, body) in [
        (
            "PATCH",
            format!("/api/v1/roles/{legacy_role}"),
            Some(json!({ "name": "legacy-renamed" })),
        ),
        ("DELETE", format!("/api/v1/roles/{legacy_role}"), None),
    ] {
        let (status, _headers, problem) = send(
            app.clone(),
            authorized_request(method, &uri, &manager_token, body.as_ref()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}: {problem}");
        assert_eq!(problem_code(&problem), "permission_subset_violation");
    }

    // A super administrator is exempt from the subset rule.
    let root_token = login_token(&app, "root1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/roles/{legacy_role}"),
            &root_token,
            Some(&json!({ "name": "legacy-renamed" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "legacy-renamed");
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_permission_writes_serialize_on_the_role_row(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let role_id = insert_custom_role(&pool, "auditors", &["users.read"]).await;
    let token = login_token(&app, "root1").await;

    let first = authorized_request(
        "PUT",
        &format!("/api/v1/roles/{role_id}/permissions"),
        &token,
        Some(&json!({ "permission_keys": ["users.read", "audit.read"] })),
    );
    let second = authorized_request(
        "PUT",
        &format!("/api/v1/roles/{role_id}/permissions"),
        &token,
        Some(&json!({ "permission_keys": ["roles.read", "roles.delete"] })),
    );
    let (first, second) = tokio::join!(send(app.clone(), first), send(app.clone(), second));
    assert_eq!(first.0, StatusCode::OK, "{}", first.2);
    assert_eq!(second.0, StatusCode::OK, "{}", second.2);

    let final_keys: Vec<String> = sqlx::query_scalar(
        "SELECT permission_key FROM role_permissions WHERE role_id = $1 ORDER BY permission_key",
    )
    .bind(role_id)
    .fetch_all(&pool)
    .await
    .expect("final permissions");
    assert!(
        final_keys == ["audit.read", "users.read"] || final_keys == ["roles.delete", "roles.read"],
        "final set must be one complete write, found {final_keys:?}"
    );

    // Both changes were applied in turn, so both were audited.
    assert_eq!(audit_count(&pool, "role_permissions_changed").await, 2);

    let verification =
        rs_admin_audit::verify_chain(&pool, &auth_support::security_config().audit_chain.keys)
            .await
            .expect("audit chain");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .expect("audit count");
    assert_eq!(verification.verified_rows, stored as u64);
}

#[sqlx::test(migrations = "../../migrations")]
async fn restricted_sessions_are_refused_before_reaching_role_handlers(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::restricted("temp", "temp@example.com")
        },
    )
    .await;
    let token = login_token(&app, "temp").await;

    for (method, uri, body) in [
        ("GET", "/api/v1/roles".to_owned(), None),
        (
            "POST",
            "/api/v1/roles".to_owned(),
            Some(json!({ "name": "blocked", "permission_keys": ["roles.read"] })),
        ),
        ("GET", "/api/v1/permissions".to_owned(), None),
    ] {
        let (status, _headers, problem) = send(
            app.clone(),
            authorized_request(method, &uri, &token, body.as_ref()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {problem}");
        assert_eq!(problem_code(&problem), "password_change_required");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn roles_api_requires_authentication_and_audit_chain_verifies(pool: PgPool) {
    let app = app(&pool).await;

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/roles", "not-a-token", None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(problem_code(&body), "access_token_invalid");

    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "root1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/roles",
            &token,
            Some(&json!({
                "name": "auditors",
                "permission_keys": ["audit.read"]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let verification =
        rs_admin_audit::verify_chain(&pool, &auth_support::security_config().audit_chain.keys)
            .await
            .expect("audit chain");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .expect("audit count");
    assert_eq!(verification.verified_rows, stored as u64);
    assert_eq!(verification.head_seq, stored);
}
