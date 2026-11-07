//! Identity/RBAC schema behavior (docs/specs/login-system.md §4.2–§4.3).
//!
//! These tests exercise the migration as deployed: constraints, unique
//! indexes, `RESTRICT` foreign keys and the role soft-delete rule.

use rs_admin_store::identity::IdentifierConflict;
use sqlx::PgPool;
use uuid::Uuid;

const PASSWORD_HASH: &str = "$argon2id$v=19$m=65536,t=3,p=4$placeholder";

async fn insert_user(pool: &PgPool, username: &str, email: &str) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO users (username, email, display_name, password_hash)
         VALUES ($1, $2, $1, $3)
         RETURNING id",
    )
    .bind(username)
    .bind(email)
    .bind(PASSWORD_HASH)
    .fetch_one(pool)
    .await
}

fn constraint(error: &sqlx::Error) -> Option<&str> {
    match error {
        sqlx::Error::Database(error) => error.constraint(),
        _ => None,
    }
}

fn sql_state(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(error) => error.code().map(|code| code.into_owned()),
        _ => None,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn identity_and_rbac_tables_exist(pool: PgPool) {
    for table in [
        "users",
        "roles",
        "permissions",
        "role_permissions",
        "user_roles",
    ] {
        let found: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(format!("public.{table}"))
            .fetch_one(&pool)
            .await
            .expect("catalog query");

        assert_eq!(found.as_deref(), Some(table), "{table} is missing");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn username_storage_is_lowercase_and_format_checked(pool: PgPool) {
    insert_user(&pool, "alice", "alice@example.com")
        .await
        .expect("valid user");

    let error = insert_user(&pool, "Alice", "alice2@example.com")
        .await
        .expect_err("uppercase username must be rejected");
    assert_eq!(sql_state(&error).as_deref(), Some("23514"));
    // The format check also requires lowercase ASCII, so Postgres may report
    // either constraint first.
    assert!(
        matches!(
            constraint(&error),
            Some("users_username_format" | "users_username_lower")
        ),
        "unexpected constraint: {:?}",
        constraint(&error)
    );

    let error = insert_user(&pool, "-bad", "bad@example.com")
        .await
        .expect_err("invalid username must be rejected");
    assert_eq!(constraint(&error), Some("users_username_format"));

    let error = insert_user(&pool, "ab", "short@example.com")
        .await
        .expect_err("too-short username must be rejected");
    assert_eq!(constraint(&error), Some("users_username_format"));

    let error = insert_user(&pool, "alice", "ALICE@Example.com")
        .await
        .expect_err("uppercase email must be rejected");
    assert_eq!(constraint(&error), Some("users_email_lower"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn display_name_and_phone_lengths_are_enforced(pool: PgPool) {
    let error = insert_user_with_name(&pool, "alice", "alice@example.com", "")
        .await
        .expect_err("empty display name must be rejected");
    assert_eq!(constraint(&error), Some("users_display_name_len"));

    let error = insert_user_with_name(&pool, "bob", "bob@example.com", &"n".repeat(65))
        .await
        .expect_err("long display name must be rejected");
    assert_eq!(constraint(&error), Some("users_display_name_len"));

    let error = sqlx::query(
        "INSERT INTO users (username, email, display_name, password_hash, phone)
         VALUES ('carol', 'carol@example.com', 'Carol', $1, $2)",
    )
    .bind(PASSWORD_HASH)
    .bind("1".repeat(33))
    .execute(&pool)
    .await
    .expect_err("long phone must be rejected");
    assert_eq!(constraint(&error), Some("users_phone_len"));
}

async fn insert_user_with_name(
    pool: &PgPool,
    username: &str,
    email: &str,
    display_name: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO users (username, email, display_name, password_hash)
         VALUES ($1, $2, $3, $4)
         RETURNING id",
    )
    .bind(username)
    .bind(email)
    .bind(display_name)
    .bind(PASSWORD_HASH)
    .fetch_one(pool)
    .await
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_identifiers_are_rejected_and_classified(pool: PgPool) {
    insert_user(&pool, "alice", "alice@example.com")
        .await
        .expect("first user");

    let error = insert_user(&pool, "alice", "other@example.com")
        .await
        .expect_err("duplicate username");
    assert_eq!(sql_state(&error).as_deref(), Some("23505"));
    assert_eq!(constraint(&error), Some("users_username_uq"));
    assert_eq!(
        IdentifierConflict::from_unique_violation(&error),
        Some(IdentifierConflict::Username)
    );

    let error = insert_user(&pool, "other", "alice@example.com")
        .await
        .expect_err("duplicate email");
    assert_eq!(sql_state(&error).as_deref(), Some("23505"));
    assert_eq!(constraint(&error), Some("users_email_uq"));
    assert_eq!(
        IdentifierConflict::from_unique_violation(&error),
        Some(IdentifierConflict::Email)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn role_names_are_case_insensitively_unique_while_active(pool: PgPool) {
    let role_id: Uuid = sqlx::query_scalar(
        "INSERT INTO roles (name, description) VALUES ('Editor', 'Edits content') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("custom role");

    let error = sqlx::query("INSERT INTO roles (name) VALUES ('editor')")
        .execute(&pool)
        .await
        .expect_err("duplicate active name");
    assert_eq!(sql_state(&error).as_deref(), Some("23505"));
    assert_eq!(constraint(&error), Some("roles_name_active_uq"));

    sqlx::query("UPDATE roles SET deleted_at = now() WHERE id = $1")
        .bind(role_id)
        .execute(&pool)
        .await
        .expect("soft delete");

    sqlx::query("INSERT INTO roles (name) VALUES ('editor')")
        .execute(&pool)
        .await
        .expect("a soft-deleted name can be reused");

    let error = sqlx::query(
        "INSERT INTO roles (name, is_builtin, deleted_at) VALUES ('builtin', true, now())",
    )
    .execute(&pool)
    .await
    .expect_err("built-in roles cannot be soft-deleted");
    assert_eq!(constraint(&error), Some("roles_builtin_not_deleted"));

    let error = sqlx::query("INSERT INTO roles (name) VALUES ($1)")
        .bind("r".repeat(65))
        .execute(&pool)
        .await
        .expect_err("role name too long");
    assert_eq!(constraint(&error), Some("roles_name_len"));

    let error = sqlx::query("INSERT INTO roles (name, description) VALUES ('long', $1)")
        .bind("d".repeat(501))
        .execute(&pool)
        .await
        .expect_err("description too long");
    assert_eq!(constraint(&error), Some("roles_description_len"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn foreign_keys_restrict_deletes(pool: PgPool) {
    let user_id = insert_user(&pool, "alice", "alice@example.com")
        .await
        .expect("user");
    let role_id: Uuid =
        sqlx::query_scalar("INSERT INTO roles (name) VALUES ('Editor') RETURNING id")
            .fetch_one(&pool)
            .await
            .expect("role");
    sqlx::query("INSERT INTO permissions (key, group_key, display_name_zh, display_name_en) VALUES ('users.read', 'users', '查看用户', 'View users')")
        .execute(&pool)
        .await
        .expect("permission");
    sqlx::query("INSERT INTO role_permissions (role_id, permission_key) VALUES ($1, 'users.read')")
        .bind(role_id)
        .execute(&pool)
        .await
        .expect("role permission");

    let error = sqlx::query("DELETE FROM permissions WHERE key = 'users.read'")
        .execute(&pool)
        .await
        .expect_err("referenced permission cannot be deleted");
    assert_eq!(sql_state(&error).as_deref(), Some("23001"));

    let error = sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(role_id)
        .execute(&pool)
        .await
        .expect_err("referenced role cannot be deleted");
    assert_eq!(sql_state(&error).as_deref(), Some("23001"));

    sqlx::query("DELETE FROM role_permissions WHERE role_id = $1")
        .bind(role_id)
        .execute(&pool)
        .await
        .expect("detach permission");
    sqlx::query("INSERT INTO user_roles (user_id, role_id, assigned_by) VALUES ($1, $2, $1)")
        .bind(user_id)
        .bind(role_id)
        .execute(&pool)
        .await
        .expect("role assignment");

    let error = sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(role_id)
        .execute(&pool)
        .await
        .expect_err("assigned role cannot be deleted");
    assert_eq!(sql_state(&error).as_deref(), Some("23001"));

    let error = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect_err("referenced user cannot be deleted");
    assert_eq!(sql_state(&error).as_deref(), Some("23001"));

    let error = sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(&pool)
        .await
        .expect_err("duplicate assignment");
    assert_eq!(sql_state(&error).as_deref(), Some("23505"));
    assert_eq!(constraint(&error), Some("user_roles_pkey"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn spec_indexes_exist(pool: PgPool) {
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexname FROM pg_indexes
         WHERE schemaname = 'public'
           AND tablename IN ('users', 'roles', 'permissions', 'role_permissions', 'user_roles')",
    )
    .fetch_all(&pool)
    .await
    .expect("index catalog");

    for expected in [
        "users_username_uq",
        "users_email_uq",
        "users_username_trgm",
        "users_email_trgm",
        "users_display_name_trgm",
        "roles_name_active_uq",
        "user_roles_role_idx",
    ] {
        assert!(
            indexes.iter().any(|index| index == expected),
            "missing index {expected}; found {indexes:?}"
        );
    }
}

#[test]
fn unique_violations_on_other_errors_are_not_identifier_conflicts() {
    assert_eq!(
        IdentifierConflict::from_unique_violation(&sqlx::Error::RowNotFound),
        None
    );
}
