//! Authentication persistence behaviour (docs/specs/login-system.md §4.2–§5.3).
//!
//! The tests exercise the migration and the public store API together: users
//! are found by their normalized login identifier, sessions are only visible
//! while unrevoked and unexpired, and refresh tokens are stored by hash.

use chrono::{Duration, Utc};
use rs_admin_store::auth::{
    AuthStoreError, NewRefreshToken, NewSession, create_session, find_user_by_email,
    find_user_by_username, insert_refresh_token, load_active_session, load_role_grants,
    lock_refresh_context, lock_user_row, mark_refresh_token_rotated, record_login_failure,
    reset_login_failures, update_successful_login,
};
use sqlx::PgPool;
use uuid::Uuid;

const PASSWORD_HASH: &str = "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ$c29tZWhhc2g";

async fn insert_user(pool: &PgPool, username: &str, email: &str) -> Uuid {
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
    .expect("insert user")
}

#[sqlx::test(migrations = "../../migrations")]
async fn auth_tables_and_constraints_exist(pool: PgPool) {
    for table in ["auth_sessions", "auth_refresh_tokens"] {
        let found: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(format!("public.{table}"))
            .fetch_one(&pool)
            .await
            .expect("catalog query");
        assert_eq!(found.as_deref(), Some(table), "{table} is missing");
    }

    let user_id = insert_user(&pool, "alice", "alice@example.com").await;
    let mut transaction = pool.begin().await.expect("begin");
    let session_id = create_session(
        &mut transaction,
        NewSession {
            user_id,
            idle_expires_at: Utc::now() + Duration::minutes(30),
            absolute_expires_at: Utc::now() + Duration::hours(12),
            authenticated_at: Utc::now(),
            ip: None,
            user_agent: None,
        },
    )
    .await
    .expect("create session");
    transaction.commit().await.expect("commit");

    let error = sqlx::query(
        "INSERT INTO auth_refresh_tokens (token_hash, session_id, rotation_counter, expires_at)
         VALUES ($1, $2, -1, now())",
    )
    .bind(vec![1u8; 32])
    .bind(session_id)
    .execute(&pool)
    .await
    .expect_err("negative rotation counter must be rejected");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("auth_refresh_tokens_counter_nonneg")
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn users_are_found_by_each_login_identifier(pool: PgPool) {
    let id = insert_user(&pool, "alice", "alice@example.com").await;

    assert_eq!(
        find_user_by_username(&pool, "alice")
            .await
            .expect("query")
            .expect("username")
            .id,
        id
    );
    assert_eq!(
        find_user_by_email(&pool, "alice@example.com")
            .await
            .expect("query")
            .expect("email")
            .id,
        id
    );
    assert!(
        find_user_by_username(&pool, "alice@example.com")
            .await
            .expect("query")
            .is_none()
    );
    assert!(
        find_user_by_email(&pool, "alice")
            .await
            .expect("query")
            .is_none()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn sessions_are_active_only_until_revoked_or_expired(pool: PgPool) {
    let user_id = insert_user(&pool, "alice", "alice@example.com").await;
    let now = Utc::now();

    let mut transaction = pool.begin().await.expect("begin");
    let active_id = create_session(
        &mut transaction,
        NewSession {
            user_id,
            idle_expires_at: now + Duration::minutes(30),
            absolute_expires_at: now + Duration::hours(12),
            authenticated_at: now,
            ip: None,
            user_agent: None,
        },
    )
    .await
    .expect("active session");
    transaction.commit().await.expect("commit");
    assert!(
        load_active_session(&pool, active_id, user_id)
            .await
            .expect("query")
            .is_some()
    );

    let mut transaction = pool.begin().await.expect("begin");
    let expired_id = create_session(
        &mut transaction,
        NewSession {
            user_id,
            idle_expires_at: now - Duration::seconds(1),
            absolute_expires_at: now + Duration::hours(12),
            authenticated_at: now,
            ip: None,
            user_agent: None,
        },
    )
    .await
    .expect("expired session");
    transaction.commit().await.expect("commit");
    assert!(
        load_active_session(&pool, expired_id, user_id)
            .await
            .expect("query")
            .is_none()
    );

    sqlx::query(
        "UPDATE auth_sessions SET revoked_at = now(), revoke_reason = 'logout' WHERE id = $1",
    )
    .bind(active_id)
    .execute(&pool)
    .await
    .expect("revoke");
    assert!(
        load_active_session(&pool, active_id, user_id)
            .await
            .expect("query")
            .is_none()
    );

    assert!(
        load_active_session(&pool, active_id, Uuid::now_v7())
            .await
            .expect("query")
            .is_none(),
        "a session must not be returned for another user"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn refresh_tokens_are_stored_by_hash_only(pool: PgPool) {
    let user_id = insert_user(&pool, "alice", "alice@example.com").await;
    let mut transaction = pool.begin().await.expect("begin");
    let session_id = create_session(
        &mut transaction,
        NewSession {
            user_id,
            idle_expires_at: Utc::now() + Duration::minutes(30),
            absolute_expires_at: Utc::now() + Duration::hours(12),
            authenticated_at: Utc::now(),
            ip: None,
            user_agent: None,
        },
    )
    .await
    .expect("session");
    insert_refresh_token(
        &mut transaction,
        NewRefreshToken {
            token_hash: &[7u8; 32],
            session_id,
            rotation_counter: 0,
            expires_at: Utc::now() + Duration::hours(12),
        },
    )
    .await
    .expect("refresh token");
    transaction.commit().await.expect("commit");

    let (stored, counter): (Vec<u8>, i64) = sqlx::query_as(
        "SELECT token_hash, rotation_counter FROM auth_refresh_tokens WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("stored token");

    assert_eq!(stored, vec![7u8; 32]);
    assert_eq!(counter, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_counters_reset_on_success_and_increment_on_failure(pool: PgPool) {
    let user_id = insert_user(&pool, "alice", "alice@example.com").await;

    let mut transaction = pool.begin().await.expect("begin");
    record_login_failure(&mut transaction, user_id)
        .await
        .expect("first failure");
    record_login_failure(&mut transaction, user_id)
        .await
        .expect("second failure");
    transaction.commit().await.expect("commit");

    let (count, last_failed): (i32, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT failed_login_count, last_failed_login_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("counters");
    assert_eq!(count, 2);
    assert!(last_failed.is_some());

    let mut transaction = pool.begin().await.expect("begin");
    update_successful_login(&mut transaction, user_id, None)
        .await
        .expect("success");
    transaction.commit().await.expect("commit");

    let (count, last_login): (i32, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("SELECT failed_login_count, last_login_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("counters");
    assert_eq!(count, 0);
    assert!(last_login.is_some());

    sqlx::query("UPDATE users SET failed_login_count = 3 WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("seed counter");
    let mut transaction = pool.begin().await.expect("begin");
    reset_login_failures(&mut transaction, user_id)
        .await
        .expect("reset");
    transaction.commit().await.expect("commit");
    let count: i32 = sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_locks_the_user_row_for_the_transaction(pool: PgPool) {
    let user_id = insert_user(&pool, "alice", "alice@example.com").await;

    let mut transaction = pool.begin().await.expect("begin");
    let user = lock_user_row(&mut transaction, user_id)
        .await
        .expect("lock")
        .expect("user");
    assert_eq!(user.username, "alice");
    transaction.rollback().await.expect("rollback");

    assert!(
        matches!(
            AuthStoreError::from(sqlx::Error::RowNotFound),
            AuthStoreError::Database(_)
        ),
        "store errors keep a single redacted database variant"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn role_grants_group_permissions_per_role(pool: PgPool) {
    rs_admin_store::rbac::sync_catalog(&pool)
        .await
        .expect("sync");
    let user_id = insert_user(&pool, "alice", "alice@example.com").await;
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id)
         SELECT $1, id FROM roles WHERE name = 'user'",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("baseline role");

    let grants = load_role_grants(&pool, user_id).await.expect("grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].name, "user");
    assert!(grants[0].is_builtin);
    assert!(grants[0].permission_keys.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_waiting_refresh_observes_a_rotation_committed_while_it_waited(pool: PgPool) {
    let user_id = insert_user(&pool, "alice", "alice@example.com").await;
    let now = Utc::now();

    let mut transaction = pool.begin().await.expect("begin");
    let session_id = create_session(
        &mut transaction,
        NewSession {
            user_id,
            idle_expires_at: now + Duration::minutes(30),
            absolute_expires_at: now + Duration::hours(12),
            authenticated_at: now,
            ip: None,
            user_agent: None,
        },
    )
    .await
    .expect("session");
    let original_hash = [7u8; 32];
    insert_refresh_token(
        &mut transaction,
        NewRefreshToken {
            token_hash: &original_hash,
            session_id,
            rotation_counter: 0,
            expires_at: now + Duration::hours(12),
        },
    )
    .await
    .expect("token");
    transaction.commit().await.expect("commit");

    // The first refresh holds the session lock while it rotates.
    let mut first = pool.begin().await.expect("first transaction");
    let context = lock_refresh_context(&mut first, &original_hash)
        .await
        .expect("lock")
        .expect("context");
    assert!(context.token.used_at.is_none());

    // A second refresh starts and blocks on the same session lock.
    let waiting_pool = pool.clone();
    let waiter = tokio::spawn(async move {
        let mut second = waiting_pool.begin().await.expect("second transaction");
        let context = lock_refresh_context(&mut second, &original_hash)
            .await
            .expect("waiting lock")
            .expect("context");
        second.rollback().await.expect("rollback");
        context
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let successor_hash = [8u8; 32];
    mark_refresh_token_rotated(&mut first, &original_hash, &successor_hash, now)
        .await
        .expect("rotate");
    insert_refresh_token(
        &mut first,
        NewRefreshToken {
            token_hash: &successor_hash,
            session_id,
            rotation_counter: 1,
            expires_at: now + Duration::hours(12),
        },
    )
    .await
    .expect("successor");
    first.commit().await.expect("commit rotation");

    let waiting = waiter.await.expect("waiter task");
    assert!(
        waiting.token.used_at.is_some(),
        "the waiter must observe the committed rotation instead of a stale snapshot"
    );
    assert_eq!(
        waiting.token.replaced_by_hash.as_deref(),
        Some(successor_hash.as_slice())
    );
}
