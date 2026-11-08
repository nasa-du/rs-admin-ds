//! User management API behaviour over HTTP with a real PostgreSQL schema
//! (docs/specs/login-system.md §6.3, §6.6, §7.7).

mod auth_support;

use auth_support::{NewUser, PASSWORD, app, authorized_request, insert_user, login, problem_code};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
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

fn usernames(body: &Value) -> Vec<&str> {
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["username"].as_str().expect("username"))
        .collect()
}

fn item_named<'a>(body: &'a Value, username: &str) -> &'a Value {
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["username"] == username)
        .unwrap_or_else(|| panic!("user {username} missing from {body}"))
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_users_returns_the_management_scope_with_filtered_total(pool: PgPool) {
    let app = app(&pool).await;

    // The operator is a plain administrator.
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    // A custom role within the administrator's own permissions.
    let support_role = insert_custom_role(&pool, "support", &["users.read"]).await;
    let support_id = insert_user(&pool, NewUser::active("support1", "support1@example.com")).await;
    assign_role(&pool, support_id, support_role).await;

    // A custom role beyond the administrator's permissions.
    let maker_role = insert_custom_role(&pool, "role-maker", &["roles.create"]).await;
    let maker_id = insert_user(&pool, NewUser::active("maker", "maker@example.com")).await;
    assign_role(&pool, maker_id, maker_role).await;

    // Another administrator and a super administrator are both out of scope.
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin2", "admin2@example.com")
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

    let token = login_token(&app, "admin1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/users?page_size=200", &token, None),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["page"], 1);
    assert_eq!(body["page_size"], 200);
    // admin1 (self), alice and support1; the broader and privileged users are
    // filtered out, and `total` is the filtered count.
    assert_eq!(body["total"], 3, "{body}");

    let names = usernames(&body);
    assert!(
        names.contains(&"admin1"),
        "self must stay visible: {names:?}"
    );
    assert!(names.contains(&"alice"), "{names:?}");
    assert!(names.contains(&"support1"), "{names:?}");
    assert!(!names.contains(&"maker"), "{names:?}");
    assert!(!names.contains(&"admin2"), "{names:?}");
    assert!(!names.contains(&"root1"), "{names:?}");

    let alice = item_named(&body, "alice");
    assert_eq!(alice["email"], "alice@example.com");
    assert_eq!(alice["display_name"], "alice");
    assert_eq!(alice["phone"], Value::Null);
    assert_eq!(alice["status"], "active");
    assert_eq!(alice["must_change_password"], false);
    assert_eq!(alice["locked_until"], Value::Null);
    assert_eq!(alice["last_login_at"], Value::Null);
    assert!(alice["created_at"].as_str().is_some());
    assert!(alice["updated_at"].as_str().is_some());

    let roles = alice["roles"].as_array().expect("roles");
    assert_eq!(roles.len(), 1);
    assert_eq!(
        roles[0]["id"],
        builtin_role_id(&pool, "user").await.to_string()
    );
    assert_eq!(roles[0]["name"], "user");
    assert_eq!(roles[0]["is_builtin"], true);

    // A super administrator is unrestricted.
    let root_token = login_token(&app, "root1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/users?page_size=200", &root_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 6, "{body}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_users_supports_search_filters_sort_and_pagination(pool: PgPool) {
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
            display_name: Some("Alice Chen"),
            ..NewUser::active("alice", "alice@example.com")
        },
    )
    .await;
    insert_user(
        &pool,
        NewUser {
            status: "disabled",
            ..NewUser::active("bob", "bob@example.com")
        },
    )
    .await;
    let carol_id = insert_user(&pool, NewUser::restricted("carol", "carol@example.com")).await;
    let dave_id = insert_user(&pool, NewUser::active("dave", "dave@example.com")).await;
    let support_role = insert_custom_role(&pool, "support", &["users.read"]).await;
    let erin_id = insert_user(&pool, NewUser::active("erin", "erin@example.com")).await;
    assign_role(&pool, erin_id, support_role).await;

    sqlx::query(
        "UPDATE users SET locked_until = now() + interval '10 minutes',
                          failed_login_count = 5
         WHERE id = $1",
    )
    .bind(dave_id)
    .execute(&pool)
    .await
    .expect("lock dave");
    let now = Utc::now();
    sqlx::query("UPDATE users SET last_login_at = $2 WHERE id = $1")
        .bind(erin_id)
        .bind(now - Duration::days(1))
        .execute(&pool)
        .await
        .expect("erin login");
    sqlx::query("UPDATE users SET last_login_at = $2 WHERE id = $1")
        .bind(carol_id)
        .bind(now)
        .execute(&pool)
        .await
        .expect("carol login");

    let token = login_token(&app, "admin1").await;
    let get = |uri: String| {
        let app = app.clone();
        let token = token.clone();
        async move { send(app, authorized_request("GET", &uri, &token, None)).await }
    };

    // Case-insensitive substring search over username, email and display name.
    for (uri, expected) in [
        ("/api/v1/users?q=ALICE", vec!["alice"]),
        ("/api/v1/users?q=chen", vec!["alice"]),
        ("/api/v1/users?q=BOB%40EXAMPLE", vec!["bob"]),
    ] {
        let (status, _headers, body) = get(uri.to_owned()).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(body["total"], 1, "{uri}: {body}");
        assert_eq!(usernames(&body), expected, "{uri}");
    }

    // Status, role, lock and password-state filters compose with the scope.
    let (status, _headers, body) = get("/api/v1/users?status=disabled".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(usernames(&body), ["bob"]);

    let (status, _headers, body) = get(format!("/api/v1/users?role_id={support_role}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(usernames(&body), ["erin"]);

    let baseline_role = builtin_role_id(&pool, "user").await;
    let (status, _headers, body) = get(format!("/api/v1/users?role_id={baseline_role}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 6, "{body}");

    let (status, _headers, body) = get("/api/v1/users?is_locked=true".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(usernames(&body), ["dave"]);

    let (status, _headers, body) = get("/api/v1/users?is_locked=false".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 5, "{body}");
    assert!(!usernames(&body).contains(&"dave"), "{body}");

    let (status, _headers, body) = get("/api/v1/users?must_change_password=true".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(usernames(&body), ["carol"]);

    let (status, _headers, body) = get("/api/v1/users?must_change_password=false".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 5, "{body}");
    assert!(!usernames(&body).contains(&"carol"), "{body}");

    // Sorting is whitelisted and defaults to newest first.
    let (status, _headers, default_body) = get("/api/v1/users?page_size=200".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{default_body}");
    let default_created: Vec<&str> = default_body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["created_at"].as_str().expect("created_at"))
        .collect();
    let mut descending = default_created.clone();
    descending.sort_unstable_by(|left, right| right.cmp(left));
    assert_eq!(default_created, descending, "default sort is -created_at");

    let (status, _headers, body) = get("/api/v1/users?sort=username".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        usernames(&body),
        ["admin1", "alice", "bob", "carol", "dave", "erin"]
    );

    let (status, _headers, body) = get("/api/v1/users?sort=-username".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        usernames(&body),
        ["erin", "dave", "carol", "bob", "alice", "admin1"]
    );

    let (status, _headers, body) = get("/api/v1/users?sort=-last_login_at".to_owned()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // admin1's login for this test is the newest, then carol, then erin.
    // Never-logged-in users sort after every dated row.
    assert_eq!(
        &usernames(&body)[..3],
        ["admin1", "carol", "erin"],
        "{body}"
    );

    // Offset pagination moves the window; each page is disjoint and ordered.
    for (page, expected) in [
        (1, ["admin1", "alice"]),
        (2, ["bob", "carol"]),
        (3, ["dave", "erin"]),
    ] {
        let (status, _headers, body) = get(format!(
            "/api/v1/users?sort=username&page={page}&page_size=2"
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "page {page}: {body}");
        assert_eq!(body["total"], 6, "page {page}");
        assert_eq!(body["page"], page);
        assert_eq!(body["page_size"], 2);
        assert_eq!(usernames(&body), expected, "page {page}");
    }

    // Invalid queries are 422, never silently clamped.
    for (uri, code, pointer) in [
        ("/api/v1/users?sort=phone", "invalid_sort", None),
        ("/api/v1/users?page_size=201", "page_size_too_large", None),
        ("/api/v1/users?page=0", "validation_failed", Some("/page")),
        (
            "/api/v1/users?status=pending",
            "validation_failed",
            Some("/status"),
        ),
        (
            "/api/v1/users?is_locked=yes",
            "validation_failed",
            Some("/is_locked"),
        ),
        (
            "/api/v1/users?role_id=not-a-uuid",
            "validation_failed",
            Some("/role_id"),
        ),
    ] {
        let (status, _headers, problem) = get(uri.to_owned()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}: {problem}");
        assert_eq!(problem_code(&problem), code, "{uri}");
        if let Some(pointer) = pointer {
            assert_eq!(problem["errors"][0]["pointer"], pointer, "{uri}: {problem}");
        }
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_user_returns_detail_and_enforces_management_scope(pool: PgPool) {
    let app = app(&pool).await;
    let admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin2", "admin2@example.com")
        },
    )
    .await;
    let maker_role = insert_custom_role(&pool, "role-maker", &["roles.create"]).await;
    let maker_id = insert_user(&pool, NewUser::active("maker", "maker@example.com")).await;
    assign_role(&pool, maker_id, maker_role).await;

    let token = login_token(&app, "admin1").await;
    let get = |uri: String| {
        let app = app.clone();
        let token = token.clone();
        async move { send(app, authorized_request("GET", &uri, &token, None)).await }
    };

    let (status, _headers, body) = get(format!("/api/v1/users/{alice_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], alice_id.to_string());
    assert_eq!(body["username"], "alice");
    assert_eq!(body["email"], "alice@example.com");
    assert_eq!(body["display_name"], "alice");
    assert_eq!(body["status"], "active");
    assert_eq!(body["must_change_password"], false);
    assert_eq!(body["temporary_password_expires_at"], Value::Null);
    assert_eq!(body["roles"][0]["name"], "user");

    // Self is always readable, even though a non-super-admin may not manage
    // another holder of the admin role.
    let (status, _headers, body) = get(format!("/api/v1/users/{admin_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["username"], "admin1");

    // Privileged and broader targets are 403 `out_of_scope`, never disguised
    // as 404.
    for target in ["root1", "admin2", "maker"] {
        let id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
            .bind(target)
            .fetch_one(&pool)
            .await
            .expect("target id");
        let (status, _headers, problem) = get(format!("/api/v1/users/{id}")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{target}: {problem}");
        assert_eq!(problem_code(&problem), "out_of_scope", "{target}");
    }

    let denied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied'
           AND details->>'operation' = 'get_user'
           AND details->>'reason_code' = 'out_of_scope'
           AND target_type = 'user'",
    )
    .fetch_one(&pool)
    .await
    .expect("scope denials");
    assert_eq!(denied, 3);

    // Missing and malformed ids are both 404.
    let (status, _headers, problem) = get(format!("/api/v1/users/{}", Uuid::now_v7())).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
    assert_eq!(problem_code(&problem), "not_found");

    let (status, _headers, _problem) = get("/api/v1/users/not-a-uuid".to_owned()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A super administrator can read any target.
    let root_token = login_token(&app, "root1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "GET",
            &format!("/api/v1/users/{maker_id}"),
            &root_token,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["username"], "maker");
}

fn user_names(body: &Value) -> Vec<&str> {
    body["user"]["roles"]
        .as_array()
        .expect("roles")
        .iter()
        .map(|role| role["name"].as_str().expect("role name"))
        .collect()
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_user_issues_a_one_time_temporary_password_and_assigns_roles(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let support_role = insert_custom_role(&pool, "support", &["users.read"]).await;
    let baseline_role = builtin_role_id(&pool, "user").await;
    let token = login_token(&app, "admin1").await;

    let (status, headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/users",
            &token,
            Some(&json!({
                "username": "newbie",
                "email": "newbie@example.com",
                "display_name": "New Bee",
                "phone": "+1 555 0100",
                "role_ids": [baseline_role.to_string(), support_role.to_string()]
            })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    let user_id = body["user"]["id"].as_str().expect("user id");
    let expected_location = format!("/api/v1/users/{user_id}");
    assert_eq!(
        headers.get(header::LOCATION).and_then(|v| v.to_str().ok()),
        Some(expected_location.as_str())
    );

    let temporary_password = body["temporary_password"]
        .as_str()
        .expect("temporary password");
    assert_eq!(temporary_password.len(), 24, "{temporary_password}");
    assert!(
        temporary_password
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
        "{temporary_password}"
    );

    let expires_at = body["user"]["temporary_password_expires_at"]
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
    assert_eq!(body["user"]["must_change_password"], true);
    assert_eq!(body["user"]["status"], "active");
    assert_eq!(body["user"]["username"], "newbie");
    assert_eq!(body["user"]["email"], "newbie@example.com");
    assert_eq!(body["user"]["display_name"], "New Bee");
    assert_eq!(body["user"]["phone"], "+1 555 0100");
    assert_eq!(user_names(&body), ["user", "support"]);

    // The plaintext is persisted only as an Argon2id hash.
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1::uuid")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("stored hash");
    assert!(stored.starts_with("$argon2id$"), "{stored}");
    assert!(!stored.contains(temporary_password));

    // The temporary password authenticates but yields a restricted session.
    let (status, _headers, login_body) = login(&app, "newbie", temporary_password).await;
    assert_eq!(status, StatusCode::OK, "{login_body}");
    assert_eq!(login_body["user"]["must_change_password"], true);

    // Audit carries the required details and never the password.
    let (action, target_id, details): (String, Uuid, Value) = sqlx::query_as(
        "SELECT action, target_id, details FROM audit_logs WHERE action = 'user_created'",
    )
    .fetch_one(&pool)
    .await
    .expect("user_created audit");
    assert_eq!(action, "user_created");
    assert_eq!(target_id, Uuid::parse_str(user_id).expect("uuid"));
    assert_eq!(details["username"], "newbie");
    assert_eq!(details["email"], "newbie@example.com");
    assert_eq!(details["must_change_password"], true);
    assert!(details["temporary_password_expires_at"].as_str().is_some());
    let audited_roles: Vec<&str> = details["role_ids"]
        .as_array()
        .expect("role_ids")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let support_role_id = support_role.to_string();
    assert!(audited_roles.contains(&support_role_id.as_str()));
    assert!(!details.to_string().contains(temporary_password));
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_user_accepts_a_manual_temporary_password_that_passes_policy(pool: PgPool) {
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
    let manual = "manual temporary passphrase";

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/users",
            &token,
            Some(&json!({
                "username": "manual",
                "email": "manual@example.com",
                "password": manual
            })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["temporary_password"], manual);
    assert_eq!(user_names(&body), ["user"]);
    // The display name defaults to the username.
    assert_eq!(body["user"]["display_name"], "manual");

    let (status, _headers, login_body) = login(&app, "manual", manual).await;
    assert_eq!(status, StatusCode::OK, "{login_body}");
    assert_eq!(login_body["user"]["must_change_password"], true);
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_user_validates_fields_identifiers_and_roles(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    insert_user(&pool, NewUser::active("taken", "taken@example.com")).await;
    let token = login_token(&app, "root1").await;

    let post = |payload: Value| {
        let app = app.clone();
        let token = token.clone();
        async move {
            send(
                app,
                authorized_request("POST", "/api/v1/users", &token, Some(&payload)),
            )
            .await
        }
    };

    for (payload, pointer) in [
        (
            json!({"username": "ab", "email": "ok@example.com"}),
            "/username",
        ),
        (
            json!({"username": "root", "email": "ok@example.com"}),
            "/username",
        ),
        (
            json!({"username": "ok-user", "email": "not-an-email"}),
            "/email",
        ),
        (
            json!({"username": "ok-user", "email": "ok@example.com", "display_name": ""}),
            "/display_name",
        ),
        (
            json!({"username": "ok-user", "email": "ok@example.com", "phone": "x".repeat(33)}),
            "/phone",
        ),
    ] {
        let (status, _headers, problem) = post(payload.clone()).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{payload}: {problem}"
        );
        assert_eq!(problem_code(&problem), "validation_failed", "{payload}");
        assert_eq!(problem["errors"][0]["pointer"], pointer, "{payload}");
    }

    // Missing required fields use the shared JSON error mapping.
    let (status, _headers, problem) = post(json!({"email": "ok@example.com"})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "validation_failed");
    assert_eq!(problem["errors"][0]["pointer"], "/username");

    // Unknown body fields are a 400, not a silent ignore.
    let (status, _headers, problem) =
        post(json!({"username": "ok-user", "email": "ok@example.com", "roles": []})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem_code(&problem), "unknown_field");

    // Passwords must satisfy the offline policy.
    let (status, _headers, problem) = post(json!({
        "username": "ok-user",
        "email": "ok@example.com",
        "password": "short"
    }))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "password_policy_violation");
    assert_eq!(problem["errors"][0]["pointer"], "/password");
    assert_eq!(problem["errors"][0]["params"]["reason"], "too_short");

    let (status, _headers, problem) = post(json!({
        "username": "verylongusername",
        "email": "ok@example.com",
        "password": "verylongusername"
    }))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "password_policy_violation");
    assert_eq!(
        problem["errors"][0]["params"]["reason"],
        "equals_identifier"
    );

    // Identifier conflicts are 409 with the colliding field.
    for (payload, pointer) in [
        (
            json!({"username": "taken", "email": "fresh@example.com"}),
            "/username",
        ),
        (
            json!({"username": "fresh", "email": "taken@example.com"}),
            "/email",
        ),
    ] {
        let (status, _headers, problem) = post(payload.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{payload}: {problem}");
        assert_eq!(problem_code(&problem), "identifier_taken", "{payload}");
        assert_eq!(problem["errors"][0]["pointer"], pointer, "{payload}");
    }

    // Unknown, soft-deleted and malformed role ids are rejected.
    let (status, _headers, problem) = post(json!({
        "username": "ok-user",
        "email": "ok@example.com",
        "role_ids": [Uuid::now_v7().to_string()]
    }))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "unknown_role");

    let soft_deleted: Uuid = sqlx::query_scalar(
        "INSERT INTO roles (name, is_builtin, deleted_at) VALUES ('gone', false, now()) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("soft-deleted role");
    let (status, _headers, problem) = post(json!({
        "username": "ok-user",
        "email": "ok@example.com",
        "role_ids": [soft_deleted.to_string()]
    }))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "unknown_role");

    let (status, _headers, problem) = post(json!({
        "username": "ok-user",
        "email": "ok@example.com",
        "role_ids": ["not-a-uuid"]
    }))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "validation_failed");
    assert_eq!(problem["errors"][0]["pointer"], "/role_ids/0");
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_user_enforces_permissions_grant_subset_and_step_up(pool: PgPool) {
    let app = app(&pool).await;

    // A custom role with users.create but not users.assign_role.
    insert_user(&pool, NewUser::active("creator", "creator@example.com")).await;
    let creator_role = insert_custom_role(
        &pool,
        "user-creator",
        &["users.read", "users.create", "users.update"],
    )
    .await;
    let creator_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = 'creator'")
        .fetch_one(&pool)
        .await
        .expect("creator id");
    assign_role(&pool, creator_id, creator_role).await;

    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let maker_role = insert_custom_role(&pool, "role-maker", &["roles.create"]).await;
    let admin_role = builtin_role_id(&pool, "admin").await;

    let creator_token = login_token(&app, "creator").await;
    let admin_token = login_token(&app, "admin1").await;

    // Without users.assign_role, role_ids is refused but a baseline-only
    // creation works.
    let (status, _headers, problem) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/users",
            &creator_token,
            Some(&json!({
                "username": "with-role",
                "email": "with-role@example.com",
                "role_ids": [creator_role.to_string()]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/users",
            &creator_token,
            Some(&json!({
                "username": "without-role",
                "email": "without-role@example.com"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A plain user lacks users.create entirely.
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    let plain_token = login_token(&app, "plain").await;
    let (status, _headers, problem) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/users",
            &plain_token,
            Some(&json!({"username": "nope", "email": "nope@example.com"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");

    // A non-super-admin cannot grant privileged or broader custom roles.
    for role_id in [admin_role, maker_role] {
        let (status, _headers, problem) = send(
            app.clone(),
            authorized_request(
                "POST",
                "/api/v1/users",
                &admin_token,
                Some(&json!({
                    "username": format!("target-{role_id}"),
                    "email": format!("target-{role_id}@example.com"),
                    "role_ids": [role_id.to_string()]
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem_code(&problem), "permission_subset_violation");
    }

    let denied_roles: Vec<Uuid> = sqlx::query_scalar(
        "SELECT target_id FROM audit_logs
         WHERE action = 'authz_denied'
           AND details->>'operation' = 'create_user'
           AND details->>'reason_code' = 'permission_subset_violation'
           AND target_type = 'role'
         ORDER BY target_id",
    )
    .fetch_all(&pool)
    .await
    .expect("subset denials");
    let mut expected_roles = vec![admin_role, maker_role];
    expected_roles.sort();
    assert_eq!(denied_roles, expected_roles);

    // Creation always requires a recent password authentication.
    let admin_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = 'admin1'")
        .fetch_one(&pool)
        .await
        .expect("admin id");
    sqlx::query("UPDATE auth_sessions SET authenticated_at = now() - interval '301 seconds' WHERE user_id = $1")
        .bind(admin_id)
        .execute(&pool)
        .await
        .expect("age session");

    let payload = json!({"username": "stale", "email": "stale@example.com"});
    let (status, _headers, problem) = send(
        app.clone(),
        authorized_request("POST", "/api/v1/users", &admin_token, Some(&payload)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "reauth_required");
    assert_eq!(problem["details"]["max_age"], 300);

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

    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("POST", "/api/v1/users", &fresh_token, Some(&payload)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_user_edits_profile_and_requires_step_up_only_for_identifiers(pool: PgPool) {
    let app = app(&pool).await;
    let admin_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let alice_id = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let alice_token = login_token(&app, "alice").await;
    let token = login_token(&app, "admin1").await;
    let alice_version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");

    let patch = |token: String, payload: Value| {
        let app = app.clone();
        async move {
            send(
                app,
                authorized_request(
                    "PATCH",
                    &format!("/api/v1/users/{alice_id}"),
                    &token,
                    Some(&payload),
                ),
            )
            .await
        }
    };

    // Display name and phone do not need a recent password authentication.
    let (status, _headers, body) = patch(
        token.clone(),
        json!({"display_name": "Alice Chen", "phone": "+1 555 0100"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], "Alice Chen");
    assert_eq!(body["phone"], "+1 555 0100");
    assert_eq!(body["username"], "alice");

    // An explicit null clears the optional phone.
    let (status, _headers, body) = patch(token.clone(), json!({"phone": null})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["phone"], Value::Null);

    // Non-identifier changes record their field names only.
    let first_changed_fields: Value = sqlx::query_scalar(
        "SELECT details->'changed_fields' FROM audit_logs
         WHERE action = 'user_updated'
         ORDER BY chain_seq ASC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("first user_updated audit");
    assert_eq!(
        first_changed_fields,
        json!({ "display_name": true, "phone": true })
    );

    // A stale session can still edit non-identifier profile fields.
    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at = now() - interval '301 seconds' WHERE user_id = $1",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("age session");
    let (status, _headers, body) = patch(token.clone(), json!({"display_name": "Alice C"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["display_name"], "Alice C");

    // Empty updates are rejected before any step-up decision.
    let (status, _headers, problem) = patch(token.clone(), json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem_code(&problem), "empty_update");

    // Username and email changes require step-up.
    for payload in [
        json!({"username": "alice2"}),
        json!({"email": "alice2@example.com"}),
    ] {
        let (status, _headers, problem) = patch(token.clone(), payload).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem_code(&problem), "reauth_required");
        assert_eq!(problem["details"]["max_age"], 300);
    }
    let step_up_denials: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_step_up_denied'
           AND details->>'operation' = 'update_user'",
    )
    .fetch_one(&pool)
    .await
    .expect("step-up denials");
    assert_eq!(step_up_denials, 2);

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

    let (status, _headers, body) = patch(
        fresh_token,
        json!({"username": "alice2", "email": "alice2@example.com"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["username"], "alice2");
    assert_eq!(body["email"], "alice2@example.com");

    // Identifier edits change the login identifiers without revoking sessions
    // or bumping the token version (spec §7.7).
    let still_valid: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = $1")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("token version");
    assert_eq!(still_valid, alice_version);
    let (status, _headers, me) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/auth/me", &alice_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["username"], "alice2");

    // Both login identifiers move to the new values.
    let (status, _headers, body) = login(&app, "alice2", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _headers, problem) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{problem}");
    assert_eq!(problem_code(&problem), "invalid_credentials");

    let changed_fields: Value = sqlx::query_scalar(
        "SELECT details->'changed_fields'
         FROM audit_logs WHERE action = 'user_updated'
         ORDER BY chain_seq DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("user_updated audit");
    assert_eq!(
        changed_fields,
        json!({
            "username": { "old": "alice", "new": "alice2" },
            "email": { "old": "alice@example.com", "new": "alice2@example.com" }
        })
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_user_rejects_conflicts_scope_self_and_invalid_fields(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let admin2_id = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin2", "admin2@example.com")
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
    let bob_id = insert_user(&pool, NewUser::active("bob", "bob@example.com")).await;
    let token = login_token(&app, "admin1").await;

    let patch = |target: Uuid, payload: Value| {
        let app = app.clone();
        let token = token.clone();
        async move {
            send(
                app,
                authorized_request(
                    "PATCH",
                    &format!("/api/v1/users/{target}"),
                    &token,
                    Some(&payload),
                ),
            )
            .await
        }
    };

    // Identifier conflicts answer 409 with the colliding pointer.
    for (payload, pointer) in [
        (json!({"username": "alice"}), "/username"),
        (json!({"email": "alice@example.com"}), "/email"),
    ] {
        let (status, _headers, problem) = patch(bob_id, payload.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{payload}: {problem}");
        assert_eq!(problem_code(&problem), "identifier_taken", "{payload}");
        assert_eq!(problem["errors"][0]["pointer"], pointer, "{payload}");
    }

    // Field validation.
    for (payload, pointer) in [
        (json!({"username": "ab"}), "/username"),
        (json!({"email": "not-an-email"}), "/email"),
        (json!({"display_name": ""}), "/display_name"),
        (json!({"phone": ""}), "/phone"),
    ] {
        let (status, _headers, problem) = patch(bob_id, payload.clone()).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{payload}: {problem}"
        );
        assert_eq!(problem_code(&problem), "validation_failed", "{payload}");
        assert_eq!(problem["errors"][0]["pointer"], pointer, "{payload}");
    }

    // Explicit nulls for non-nullable fields point at the offending field.
    for (payload, pointer) in [
        (json!({"display_name": null}), "/display_name"),
        (json!({"username": null}), "/username"),
        (json!({"email": null}), "/email"),
    ] {
        let (status, _headers, problem) = patch(bob_id, payload.clone()).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{payload}: {problem}"
        );
        assert_eq!(problem_code(&problem), "validation_failed", "{payload}");
        assert_eq!(problem["errors"][0]["pointer"], pointer, "{payload}");
    }

    let (status, _headers, problem) = patch(bob_id, json!({"nickname": "bob"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem_code(&problem), "unknown_field");

    // Self-management is forbidden, even for the display name.
    let (status, _headers, problem) = patch(
        sqlx::query_scalar("SELECT id FROM users WHERE username = 'admin1'")
            .fetch_one(&pool)
            .await
            .expect("admin id"),
        json!({"display_name": "Me"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "self_action_forbidden");

    // Privileged targets are outside a non-super-admin's scope.
    for target in [root_id, admin2_id] {
        let (status, _headers, problem) = patch(target, json!({"display_name": "Nope"})).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem_code(&problem), "out_of_scope");
    }

    let (status, _headers, problem) = patch(Uuid::now_v7(), json!({"display_name": "Ghost"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
    assert_eq!(problem_code(&problem), "not_found");

    // A plain user lacks users.update entirely.
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    let plain_token = login_token(&app, "plain").await;
    let (status, _headers, problem) = send(
        app.clone(),
        authorized_request(
            "PATCH",
            &format!("/api/v1/users/{alice_id}"),
            &plain_token,
            Some(&json!({"display_name": "Nope"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");

    let self_denials: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied' AND details->>'reason_code' = 'self_action_forbidden'",
    )
    .fetch_one(&pool)
    .await
    .expect("self denials");
    assert_eq!(self_denials, 1);
    let scope_denials: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied' AND details->>'reason_code' = 'out_of_scope'",
    )
    .fetch_one(&pool)
    .await
    .expect("scope denials");
    assert_eq!(scope_denials, 2);
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_scope_fails_closed_on_unresolved_roles(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;

    // A custom role holding a permission key that is no longer in the catalog.
    sqlx::query(
        "INSERT INTO permissions (key, group_key, display_name_zh, display_name_en)
         VALUES ('legacy.manage', 'legacy', '旧权限', 'Legacy manage')",
    )
    .execute(&pool)
    .await
    .expect("legacy permission");
    let legacy_role = insert_custom_role(&pool, "legacy", &["legacy.manage"]).await;
    let legacy_user = insert_user(
        &pool,
        NewUser::active("legacy-user", "legacy-user@example.com"),
    )
    .await;
    assign_role(&pool, legacy_user, legacy_role).await;

    // A persisted built-in name the code catalog no longer knows.
    let legacy_builtin: Uuid = sqlx::query_scalar(
        "INSERT INTO roles (name, is_builtin) VALUES ('legacy_admin', true) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy builtin");
    let legacy_builtin_user = insert_user(
        &pool,
        NewUser::active("legacy-builtin", "legacy-builtin@example.com"),
    )
    .await;
    assign_role(&pool, legacy_builtin_user, legacy_builtin).await;

    let token = login_token(&app, "admin1").await;
    let (status, _headers, body) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/users?page_size=200", &token, None),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 2, "{body}");
    assert_eq!(usernames(&body).len(), 2);
    assert!(usernames(&body).contains(&"plain"));
    assert!(!usernames(&body).contains(&"legacy-user"));
    assert!(!usernames(&body).contains(&"legacy-builtin"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_users_requires_users_read(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let token = login_token(&app, "alice").await;

    let (status, _headers, problem) = send(
        app.clone(),
        authorized_request("GET", "/api/v1/users", &token, None),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem_code(&problem), "forbidden");

    let denied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied'
           AND details->>'operation' = 'list_users'
           AND details->>'reason_code' = 'forbidden'",
    )
    .fetch_one(&pool)
    .await
    .expect("denial audit");
    assert_eq!(denied, 1);
}
