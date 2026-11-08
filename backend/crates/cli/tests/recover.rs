//! `recover-super-admin` against a real PostgreSQL schema
//! (docs/specs/login-system.md §4.4, ADR-0003 §6).

mod support;

use chrono::{Duration, Utc};
use rs_admin_cli::{CliError, RecoverInput, RecoverOutcome, recover_super_admin};
use secrecy::SecretString;
use sqlx::PgPool;
use support::{
    NewUser, audit_rows, insert_session, insert_user, password_service, prepare_catalog, services,
    token_hash, user_roles, verify_audit_chain,
};

const EMAIL: &str = "ops.admin@example.com";
const USERNAME: &str = "ops.admin";
const NEW_PASSWORD: &str = "a completely new temporary passphrase";

async fn run_recover(
    pool: &PgPool,
    email: &str,
    password: &str,
) -> Result<RecoverOutcome, CliError> {
    let password = SecretString::from(password.to_owned());
    recover_super_admin(
        pool,
        services(),
        RecoverInput {
            email,
            password: &password,
        },
    )
    .await
}

#[sqlx::test(migrations = "../../migrations")]
async fn recover_reissues_a_temporary_password_and_invalidates_every_session(pool: PgPool) {
    prepare_catalog(&pool).await;
    let user_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active(USERNAME, EMAIL)
        },
    )
    .await;
    let first_session = insert_session(&pool, user_id).await;
    let second_session = insert_session(&pool, user_id).await;
    sqlx::query(
        "UPDATE users
         SET failed_login_count = 4, locked_until = now() + interval '5 minutes',
             last_failed_login_at = now()
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("lock the account");

    let outcome = run_recover(&pool, EMAIL, NEW_PASSWORD)
        .await
        .expect("recover");

    assert_eq!(outcome.user_id, user_id);
    assert_eq!(outcome.username, USERNAME);
    assert_eq!(outcome.email, EMAIL);
    assert_eq!(outcome.sessions_revoked, 2);
    assert_eq!(outcome.token_version, 1, "the token version is incremented");
    assert!(
        outcome.temporary_password_expires_at > Utc::now() + Duration::hours(71),
        "the temporary password expires after the configured 72 hours"
    );

    let (status, must_change_password, failed_login_count, locked_until, token_version, hash): (
        String,
        bool,
        i32,
        Option<chrono::DateTime<Utc>>,
        i64,
        String,
    ) = sqlx::query_as(
        "SELECT status, must_change_password, failed_login_count, locked_until,
                token_version, password_hash
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("user row");
    assert_eq!(status, "active");
    assert!(
        must_change_password,
        "the temporary password forces a change"
    );
    assert_eq!(failed_login_count, 0, "the account lock is cleared");
    assert_eq!(locked_until, None);
    assert_eq!(token_version, 1);

    assert!(
        password_service()
            .verify(NEW_PASSWORD, &hash)
            .await
            .expect("verify")
            .valid,
        "the new temporary password is stored"
    );
    assert!(
        !password_service()
            .verify(support::PASSWORD, &hash)
            .await
            .expect("verify")
            .valid,
        "the old password no longer works"
    );

    for session_id in [first_session, second_session] {
        let (revoked_at, revoke_reason): (Option<chrono::DateTime<Utc>>, Option<String>) =
            sqlx::query_as("SELECT revoked_at, revoke_reason FROM auth_sessions WHERE id = $1")
                .bind(session_id)
                .fetch_one(&pool)
                .await
                .expect("session");
        assert!(revoked_at.is_some(), "every session is revoked");
        assert_eq!(revoke_reason.as_deref(), Some("admin_revoke"));

        let token_revoked: bool = sqlx::query_scalar(
            "SELECT revoked_at IS NOT NULL FROM auth_refresh_tokens WHERE token_hash = $1",
        )
        .bind(token_hash(session_id).as_slice())
        .fetch_one(&pool)
        .await
        .expect("refresh token");
        assert!(token_revoked, "every refresh family is revoked");
    }

    let rows = audit_rows(&pool).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "system_recover_super_admin");
    assert_eq!(row.severity, "critical");
    assert_eq!(row.result, "success");
    assert_eq!(row.actor_label, "cli");
    assert_eq!(row.actor_user_id, None);
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(user_id));
    assert_eq!(row.target_label.as_deref(), Some(USERNAME));
    assert_eq!(row.details["target_user_id"], user_id.to_string());
    assert_eq!(row.details["recovered"], true);
    assert_eq!(row.details["sessions_revoked_count"], 2);
    assert!(
        !row.details.to_string().contains(NEW_PASSWORD),
        "the temporary password never reaches the audit log"
    );

    verify_audit_chain(&pool).await;
}

#[sqlx::test(migrations = "../../migrations")]
async fn recover_refuses_a_target_without_the_super_admin_role(pool: PgPool) {
    prepare_catalog(&pool).await;
    let user_id = insert_user(&pool, NewUser::active(USERNAME, EMAIL)).await;

    let error = run_recover(&pool, EMAIL, NEW_PASSWORD)
        .await
        .expect_err("not a super admin");
    assert_eq!(
        error.to_string(),
        "the target user is not a super administrator"
    );

    let (must_change_password, locked_until): (bool, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT must_change_password, locked_until FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("user row");
    assert!(!must_change_password);
    assert_eq!(locked_until, None);
    assert_eq!(user_roles(&pool, user_id).await, ["user"]);
    assert!(audit_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn recover_refuses_an_unknown_email(pool: PgPool) {
    prepare_catalog(&pool).await;

    let error = run_recover(&pool, EMAIL, NEW_PASSWORD)
        .await
        .expect_err("unknown user");
    assert_eq!(error.to_string(), "no user exists with that email");
    assert!(audit_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn recover_enables_a_disabled_super_admin_and_replaces_an_expired_temporary_password(
    pool: PgPool,
) {
    prepare_catalog(&pool).await;
    let user_id = insert_user(
        &pool,
        NewUser {
            status: "disabled",
            role: Some("super_admin"),
            ..NewUser::active(USERNAME, EMAIL)
        },
    )
    .await;
    sqlx::query(
        "UPDATE users
         SET must_change_password = true,
             temporary_password_expires_at = now() - interval '1 hour'
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("expire the temporary password");

    let outcome = run_recover(&pool, EMAIL, NEW_PASSWORD)
        .await
        .expect("recover");

    assert_eq!(outcome.sessions_revoked, 0);

    let (status, hash): (String, String) =
        sqlx::query_as("SELECT status, password_hash FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("user row");
    assert_eq!(status, "active", "recovery re-enables the account");
    assert!(
        password_service()
            .verify(NEW_PASSWORD, &hash)
            .await
            .expect("verify")
            .valid
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn recover_applies_the_password_policy(pool: PgPool) {
    prepare_catalog(&pool).await;
    let _ = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active(USERNAME, EMAIL)
        },
    )
    .await;

    let error = run_recover(&pool, EMAIL, "short")
        .await
        .expect_err("too short");
    assert_eq!(
        error.to_string(),
        "the temporary password does not satisfy the password policy (too_short)"
    );
    assert!(audit_rows(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn recover_rejects_a_password_equal_to_the_stored_username_or_display_name(pool: PgPool) {
    prepare_catalog(&pool).await;
    let username = "operations.administrator";
    let email = "operations.administrator@example.com";
    let user_id = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active(username, email)
        },
    )
    .await;

    let error = run_recover(&pool, email, username)
        .await
        .expect_err("equals username");
    assert_eq!(
        error.to_string(),
        "the temporary password does not satisfy the password policy (equals_identifier)"
    );

    sqlx::query("UPDATE users SET display_name = $2 WHERE id = $1")
        .bind(user_id)
        .bind("Operations Admin Name")
        .execute(&pool)
        .await
        .expect("display name");
    let error = run_recover(&pool, email, "operations admin name")
        .await
        .expect_err("equals display name");
    assert_eq!(
        error.to_string(),
        "the temporary password does not satisfy the password policy (equals_identifier)"
    );

    assert!(audit_rows(&pool).await.is_empty());
}
