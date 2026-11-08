//! `bootstrap-super-admin` against a real PostgreSQL schema
//! (docs/specs/login-system.md §4.4, ADR-0003 §6).

mod support;

use chrono::{Duration, Utc};
use rs_admin_cli::{BootstrapInput, BootstrapOutcome, CliError, bootstrap_super_admin};
use secrecy::SecretString;
use sqlx::PgPool;
use support::{
    NewUser, PASSWORD, audit_rows, insert_user, services, user_roles, verify_audit_chain,
};

const EMAIL: &str = "ops.admin@example.com";
const USERNAME: &str = "ops.admin";

async fn run_bootstrap(
    pool: &PgPool,
    email: &str,
    username: &str,
    password: &str,
) -> Result<BootstrapOutcome, CliError> {
    let password = SecretString::from(password.to_owned());
    bootstrap_super_admin(
        pool,
        services(),
        BootstrapInput {
            email,
            username,
            password: &password,
        },
    )
    .await
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_creates_the_first_super_admin_on_an_empty_database(pool: PgPool) {
    let outcome = run_bootstrap(&pool, EMAIL, USERNAME, PASSWORD)
        .await
        .expect("bootstrap");

    assert!(outcome.created, "the empty database creates a new user");
    assert_eq!(outcome.username, USERNAME);
    assert_eq!(outcome.email, EMAIL);
    assert_eq!(outcome.sessions_revoked, 0);
    assert!(
        outcome.temporary_password_expires_at > Utc::now() + Duration::hours(71),
        "the temporary password expires after the configured 72 hours"
    );

    let (status, must_change_password, expires_at, password_hash): (
        String,
        bool,
        Option<chrono::DateTime<Utc>>,
        String,
    ) = sqlx::query_as(
        "SELECT status, must_change_password, temporary_password_expires_at, password_hash
         FROM users WHERE id = $1",
    )
    .bind(outcome.user_id)
    .fetch_one(&pool)
    .await
    .expect("user row");
    assert_eq!(status, "active");
    assert!(must_change_password, "first login must be forced to change");
    assert_eq!(expires_at, Some(outcome.temporary_password_expires_at));

    let verified = support::password_service()
        .verify(PASSWORD, &password_hash)
        .await
        .expect("verify password");
    assert!(verified.valid, "the temporary password is stored as a hash");

    assert_eq!(
        user_roles(&pool, outcome.user_id).await,
        ["super_admin", "user"]
    );

    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("user count");
    assert_eq!(user_count, 1);

    let roles: Vec<String> =
        sqlx::query_scalar("SELECT name FROM roles WHERE is_builtin ORDER BY name")
            .fetch_all(&pool)
            .await
            .expect("built-in roles");
    assert_eq!(roles, ["admin", "super_admin", "user"]);

    let rows = audit_rows(&pool).await;
    assert_eq!(rows.len(), 1, "bootstrap writes exactly one audit row");
    let row = &rows[0];
    assert_eq!(row.action, "system_bootstrap_super_admin");
    assert_eq!(row.severity, "critical");
    assert_eq!(row.result, "success");
    assert_eq!(row.actor_label, "cli");
    assert_eq!(row.actor_user_id, None, "CLI events have no actor user");
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(outcome.user_id));
    assert_eq!(row.target_label.as_deref(), Some(USERNAME));
    assert_eq!(row.details["target_user_id"], outcome.user_id.to_string());
    assert_eq!(row.details["created"], true);
    assert_eq!(row.details["sessions_revoked_count"], 0);
    assert!(
        !row.details.to_string().contains(PASSWORD),
        "the temporary password never reaches the audit log"
    );

    verify_audit_chain(&pool).await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn migrations_seed_no_users(pool: PgPool) {
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("user count");
    assert_eq!(user_count, 0, "production migrations must not seed users");
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_refuses_while_an_enabled_super_admin_exists(pool: PgPool) {
    run_bootstrap(&pool, EMAIL, USERNAME, PASSWORD)
        .await
        .expect("first bootstrap");

    let error = run_bootstrap(&pool, "second.admin@example.com", "second.admin", PASSWORD)
        .await
        .expect_err("a second bootstrap must be refused");
    assert_eq!(
        error.to_string(),
        "an enabled super administrator already exists"
    );

    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("user count");
    assert_eq!(user_count, 1, "the refusal creates nothing");
    assert_eq!(
        audit_rows(&pool).await.len(),
        1,
        "the refusal writes no audit"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_ignores_a_disabled_super_admin(pool: PgPool) {
    support::prepare_catalog(&pool).await;
    let _ = insert_user(
        &pool,
        NewUser {
            status: "disabled",
            role: Some("super_admin"),
            ..NewUser::active("old.admin", "old.admin@example.com")
        },
    )
    .await;

    let outcome = run_bootstrap(&pool, EMAIL, USERNAME, PASSWORD)
        .await
        .expect("a disabled super admin does not block bootstrap");
    assert!(outcome.created);

    let super_admins: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT u.id)
         FROM users u
         JOIN user_roles ur ON ur.user_id = u.id
         JOIN roles r ON r.id = ur.role_id
         WHERE r.name = 'super_admin' AND r.is_builtin AND u.status = 'active'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(super_admins, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_promotes_an_existing_user_and_revokes_its_sessions(pool: PgPool) {
    support::prepare_catalog(&pool).await;
    let user_id = insert_user(&pool, NewUser::active(USERNAME, EMAIL)).await;
    let session_id = support::insert_session(&pool, user_id).await;

    let outcome = run_bootstrap(&pool, EMAIL, USERNAME, PASSWORD)
        .await
        .expect("bootstrap promotes");

    assert!(
        !outcome.created,
        "the existing user is promoted, not created"
    );
    assert_eq!(outcome.user_id, user_id);
    assert_eq!(outcome.sessions_revoked, 1);
    assert_eq!(
        user_roles(&pool, user_id).await,
        ["super_admin", "user"],
        "the baseline role is not duplicated"
    );

    let (must_change_password, token_version): (bool, i64) =
        sqlx::query_as("SELECT must_change_password, token_version FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("user state");
    assert!(must_change_password);
    assert_eq!(token_version, 1, "existing access tokens are invalidated");

    let (revoked_at, revoke_reason): (Option<chrono::DateTime<Utc>>, Option<String>) =
        sqlx::query_as("SELECT revoked_at, revoke_reason FROM auth_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("session");
    assert!(revoked_at.is_some(), "the existing session is revoked");
    assert_eq!(revoke_reason.as_deref(), Some("admin_revoke"));

    let token_revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM auth_refresh_tokens WHERE token_hash = $1",
    )
    .bind(support::token_hash(session_id).as_slice())
    .fetch_one(&pool)
    .await
    .expect("refresh token");
    assert!(token_revoked, "the refresh family is revoked");

    let row = &audit_rows(&pool).await[0];
    assert_eq!(row.action, "system_bootstrap_super_admin");
    assert_eq!(row.details["created"], false);
    assert_eq!(row.details["sessions_revoked_count"], 1);
    verify_audit_chain(&pool).await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_keeps_the_stored_username_when_promoting(pool: PgPool) {
    support::prepare_catalog(&pool).await;
    let user_id = insert_user(&pool, NewUser::active(USERNAME, EMAIL)).await;

    // A free but different username is not a conflict: the promoted user
    // keeps the username that owns the email.
    let outcome = run_bootstrap(&pool, EMAIL, "another.available", PASSWORD)
        .await
        .expect("bootstrap promotes");
    assert!(!outcome.created);
    assert_eq!(outcome.username, USERNAME);

    let stored: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("username");
    assert_eq!(stored, USERNAME);
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_rejects_a_taken_username(pool: PgPool) {
    support::prepare_catalog(&pool).await;
    let _ = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let error = run_bootstrap(&pool, "ops.admin@example.com", "alice", PASSWORD)
        .await
        .expect_err("taken username");
    assert_eq!(error.to_string(), "the username is already taken");

    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("user count");
    assert_eq!(user_count, 1);
    assert!(audit_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_rejects_a_username_owned_by_another_user_when_promoting(pool: PgPool) {
    support::prepare_catalog(&pool).await;
    let _ = insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let _ = insert_user(&pool, NewUser::active("bob", "bob@example.com")).await;

    let error = run_bootstrap(&pool, "alice@example.com", "bob", PASSWORD)
        .await
        .expect_err("username conflict while promoting");
    assert_eq!(error.to_string(), "the username is already taken");
    assert!(audit_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_applies_the_password_policy(pool: PgPool) {
    let error = run_bootstrap(&pool, EMAIL, USERNAME, "short")
        .await
        .expect_err("too short");
    assert_eq!(
        error.to_string(),
        "the temporary password does not satisfy the password policy (too_short)"
    );

    let error = run_bootstrap(&pool, EMAIL, USERNAME, EMAIL)
        .await
        .expect_err("equals identifier");
    assert_eq!(
        error.to_string(),
        "the temporary password does not satisfy the password policy (equals_identifier)"
    );

    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("user count");
    assert_eq!(user_count, 0);
    assert!(audit_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_rejects_a_password_equal_to_the_new_username(pool: PgPool) {
    // The username is long enough to pass the length floor, so the equality
    // check is what rejects the password (display name defaults to it).
    let username = "operations.administrator";
    let error = run_bootstrap(&pool, "ops.admin@example.com", username, username)
        .await
        .expect_err("equals username");
    assert_eq!(
        error.to_string(),
        "the temporary password does not satisfy the password policy (equals_identifier)"
    );
    assert!(audit_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_rejects_a_password_equal_to_the_promoted_display_name(pool: PgPool) {
    support::prepare_catalog(&pool).await;
    let user_id = insert_user(&pool, NewUser::active(USERNAME, EMAIL)).await;
    sqlx::query("UPDATE users SET display_name = $2 WHERE id = $1")
        .bind(user_id)
        .bind("Operations Admin Name")
        .execute(&pool)
        .await
        .expect("display name");

    let error = run_bootstrap(&pool, EMAIL, USERNAME, "operations admin name")
        .await
        .expect_err("equals display name");
    assert_eq!(
        error.to_string(),
        "the temporary password does not satisfy the password policy (equals_identifier)"
    );
    assert!(audit_rows(&pool).await.is_empty());
}
