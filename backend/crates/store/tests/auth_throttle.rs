//! Login throttle state transitions against a real PostgreSQL schema
//! (docs/specs/login-system.md §8.3).

use rs_admin_store::auth::{
    FailureRecord, ThrottleScope, load_login_throttle_locks, record_known_login_failure,
    record_throttle_failure, record_unknown_login_failure, release_expired_global_throttle,
};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

const DECAY: Duration = Duration::from_secs(24 * 60 * 60);

async fn insert_user(pool: &PgPool, username: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (username, email, display_name, password_hash)
         VALUES ($1, $2, $1, 'unused')
         RETURNING id",
    )
    .bind(username)
    .bind(format!("{username}@example.com"))
    .fetch_one(pool)
    .await
    .expect("insert user")
}

async fn expire_known_lock(pool: &PgPool, user_id: Uuid) {
    sqlx::query("UPDATE users SET locked_until = now() - interval '1 second' WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .expect("expire lock");
}

async fn record_known(pool: &PgPool, user_id: Uuid) -> FailureRecord {
    let mut transaction = pool.begin().await.expect("transaction");
    let record = record_known_login_failure(&mut transaction, user_id, DECAY)
        .await
        .expect("record failure");
    transaction.commit().await.expect("commit");
    record
}

async fn record_unknown(pool: &PgPool, key: &[u8; 32]) -> FailureRecord {
    let mut transaction = pool.begin().await.expect("transaction");
    let record = record_unknown_login_failure(&mut transaction, key, DECAY)
        .await
        .expect("record failure");
    transaction.commit().await.expect("commit");
    record
}

#[sqlx::test(migrations = "../../migrations")]
async fn known_account_locks_at_five_ten_and_fifteen_failures(pool: PgPool) {
    let user_id = insert_user(&pool, "alice").await;

    for expected in 1..5 {
        assert_eq!(
            record_known(&pool, user_id).await,
            FailureRecord::Counted {
                failed_count: expected,
                retry_after_seconds: None
            }
        );
    }
    assert_eq!(
        record_known(&pool, user_id).await,
        FailureRecord::Counted {
            failed_count: 5,
            retry_after_seconds: Some(30)
        }
    );

    for expected in 6..10 {
        expire_known_lock(&pool, user_id).await;
        assert_eq!(
            record_known(&pool, user_id).await,
            FailureRecord::Counted {
                failed_count: expected,
                retry_after_seconds: Some(30)
            }
        );
    }

    expire_known_lock(&pool, user_id).await;
    assert_eq!(
        record_known(&pool, user_id).await,
        FailureRecord::Counted {
            failed_count: 10,
            retry_after_seconds: Some(300)
        }
    );

    for expected in 11..15 {
        expire_known_lock(&pool, user_id).await;
        assert_eq!(
            record_known(&pool, user_id).await,
            FailureRecord::Counted {
                failed_count: expected,
                retry_after_seconds: Some(300)
            }
        );
    }

    expire_known_lock(&pool, user_id).await;
    assert_eq!(
        record_known(&pool, user_id).await,
        FailureRecord::Counted {
            failed_count: 15,
            retry_after_seconds: Some(3_600)
        }
    );

    expire_known_lock(&pool, user_id).await;
    assert_eq!(
        record_known(&pool, user_id).await,
        FailureRecord::Counted {
            failed_count: 16,
            retry_after_seconds: Some(3_600)
        }
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_locked_known_account_does_not_accumulate_failures(pool: PgPool) {
    let user_id = insert_user(&pool, "alice").await;
    for _ in 0..5 {
        record_known(&pool, user_id).await;
    }

    match record_known(&pool, user_id).await {
        FailureRecord::AlreadyLocked {
            retry_after_seconds,
        } => assert!((1..=30).contains(&retry_after_seconds)),
        other => panic!("expected AlreadyLocked, got {other:?}"),
    }

    let count: i32 = sqlx::query_scalar("SELECT failed_login_count FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 5);
}

#[sqlx::test(migrations = "../../migrations")]
async fn account_failures_decay_after_twenty_four_quiet_hours(pool: PgPool) {
    let user_id = insert_user(&pool, "alice").await;
    sqlx::query(
        "UPDATE users
         SET failed_login_count = 12,
             last_failed_login_at = now() - interval '25 hours',
             locked_until = NULL
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("seed stale failures");

    assert_eq!(
        record_known(&pool, user_id).await,
        FailureRecord::Counted {
            failed_count: 1,
            retry_after_seconds: None
        }
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn unknown_identifiers_track_the_same_account_thresholds(pool: PgPool) {
    let key = [3u8; 32];

    for expected in 1..5 {
        assert_eq!(
            record_unknown(&pool, &key).await,
            FailureRecord::Counted {
                failed_count: expected,
                retry_after_seconds: None
            }
        );
    }
    assert_eq!(
        record_unknown(&pool, &key).await,
        FailureRecord::Counted {
            failed_count: 5,
            retry_after_seconds: Some(30)
        }
    );

    sqlx::query(
        "UPDATE auth_login_failures SET locked_until = now() - interval '1 second'
         WHERE key_hash = $1",
    )
    .bind(key.as_slice())
    .execute(&pool)
    .await
    .expect("expire lock");

    assert_eq!(
        record_unknown(&pool, &key).await,
        FailureRecord::Counted {
            failed_count: 6,
            retry_after_seconds: Some(30)
        }
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_locked_unknown_identifier_does_not_accumulate_failures(pool: PgPool) {
    let key = [4u8; 32];
    for _ in 0..5 {
        record_unknown(&pool, &key).await;
    }

    match record_unknown(&pool, &key).await {
        FailureRecord::AlreadyLocked {
            retry_after_seconds,
        } => assert!((1..=30).contains(&retry_after_seconds)),
        other => panic!("expected AlreadyLocked, got {other:?}"),
    }

    let count: i32 =
        sqlx::query_scalar("SELECT failed_count FROM auth_login_failures WHERE key_hash = $1")
            .bind(key.as_slice())
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(count, 5);
}

#[sqlx::test(migrations = "../../migrations")]
async fn ip_throttle_locks_after_twenty_failures_in_the_window(pool: PgPool) {
    let key = [5u8; 32];
    let window = Duration::from_secs(600);
    let lock = Duration::from_secs(900);

    for expected in 1..20 {
        let mut transaction = pool.begin().await.expect("transaction");
        let record =
            record_throttle_failure(&mut transaction, ThrottleScope::Ip, &key, window, lock, 20)
                .await
                .expect("record");
        transaction.commit().await.expect("commit");
        assert_eq!(
            record,
            FailureRecord::Counted {
                failed_count: expected,
                retry_after_seconds: None
            }
        );
    }

    let mut transaction = pool.begin().await.expect("transaction");
    let record =
        record_throttle_failure(&mut transaction, ThrottleScope::Ip, &key, window, lock, 20)
            .await
            .expect("record");
    transaction.commit().await.expect("commit");
    assert_eq!(
        record,
        FailureRecord::Counted {
            failed_count: 20,
            retry_after_seconds: Some(900)
        }
    );

    let locked = {
        let mut transaction = pool.begin().await.expect("transaction");
        let record =
            record_throttle_failure(&mut transaction, ThrottleScope::Ip, &key, window, lock, 20)
                .await
                .expect("record");
        transaction.commit().await.expect("commit");
        record
    };
    match locked {
        FailureRecord::AlreadyLocked {
            retry_after_seconds,
        } => assert!((1..=900).contains(&retry_after_seconds)),
        other => panic!("expected AlreadyLocked, got {other:?}"),
    }

    let failures: i32 = sqlx::query_scalar(
        "SELECT failures FROM auth_login_throttle WHERE scope = 'ip' AND key_hash = $1",
    )
    .bind(key.as_slice())
    .fetch_one(&pool)
    .await
    .expect("failures");
    assert_eq!(failures, 20);
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_expired_ip_window_restarts_the_counter(pool: PgPool) {
    let key = [6u8; 32];
    let window = Duration::from_secs(600);
    let lock = Duration::from_secs(900);
    for _ in 0..3 {
        let mut transaction = pool.begin().await.expect("transaction");
        record_throttle_failure(&mut transaction, ThrottleScope::Ip, &key, window, lock, 20)
            .await
            .expect("record");
        transaction.commit().await.expect("commit");
    }

    sqlx::query(
        "UPDATE auth_login_throttle
         SET window_start = now() - interval '601 seconds',
             last_failure_at = now() - interval '601 seconds'
         WHERE scope = 'ip' AND key_hash = $1",
    )
    .bind(key.as_slice())
    .execute(&pool)
    .await
    .expect("age window");

    let mut transaction = pool.begin().await.expect("transaction");
    let record =
        record_throttle_failure(&mut transaction, ThrottleScope::Ip, &key, window, lock, 20)
            .await
            .expect("record");
    transaction.commit().await.expect("commit");
    assert_eq!(
        record,
        FailureRecord::Counted {
            failed_count: 1,
            retry_after_seconds: None
        }
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_global_circuit_uses_the_same_counter_with_its_own_key(pool: PgPool) {
    let key = [8u8; 32];
    let window = Duration::from_secs(300);
    let lock = Duration::from_secs(60);

    let mut transaction = pool.begin().await.expect("transaction");
    let first = record_throttle_failure(
        &mut transaction,
        ThrottleScope::Global,
        &key,
        window,
        lock,
        2,
    )
    .await
    .expect("record");
    transaction.commit().await.expect("commit");
    assert_eq!(
        first,
        FailureRecord::Counted {
            failed_count: 1,
            retry_after_seconds: None
        }
    );

    let mut transaction = pool.begin().await.expect("transaction");
    let second = record_throttle_failure(
        &mut transaction,
        ThrottleScope::Global,
        &key,
        window,
        lock,
        2,
    )
    .await
    .expect("record");
    transaction.commit().await.expect("commit");
    assert_eq!(
        second,
        FailureRecord::Counted {
            failed_count: 2,
            retry_after_seconds: Some(60)
        }
    );

    let locks = load_login_throttle_locks(&pool, &[9u8; 32], &key)
        .await
        .expect("load locks");
    assert_eq!(locks.ip_retry_after_seconds, None);
    assert!(
        locks
            .global_retry_after_seconds
            .is_some_and(|seconds| (1..=60).contains(&seconds))
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_elapsed_global_circuit_is_released_exactly_once(pool: PgPool) {
    let key = [10u8; 32];
    let window = Duration::from_secs(300);
    let lock = Duration::from_secs(60);

    let mut transaction = pool.begin().await.expect("transaction");
    let record = record_throttle_failure(
        &mut transaction,
        ThrottleScope::Global,
        &key,
        window,
        lock,
        1,
    )
    .await
    .expect("record");
    transaction.commit().await.expect("commit");
    assert_eq!(
        record,
        FailureRecord::Counted {
            failed_count: 1,
            retry_after_seconds: Some(60)
        }
    );

    assert!(
        !release_expired_global_throttle(&pool)
            .await
            .expect("release check"),
        "an active circuit must not be released"
    );

    sqlx::query(
        "UPDATE auth_login_throttle
         SET locked_until = now() - interval '1 second'
         WHERE scope = 'global' AND key_hash = $1",
    )
    .bind(key.as_slice())
    .execute(&pool)
    .await
    .expect("expire circuit");

    assert!(
        release_expired_global_throttle(&pool)
            .await
            .expect("release"),
        "the first caller observes the release"
    );
    assert!(
        !release_expired_global_throttle(&pool)
            .await
            .expect("release check"),
        "the release is announced once"
    );

    let failures: i32 = sqlx::query_scalar(
        "SELECT failures FROM auth_login_throttle WHERE scope = 'global' AND key_hash = $1",
    )
    .bind(key.as_slice())
    .fetch_one(&pool)
    .await
    .expect("failures");
    assert_eq!(failures, 1);
}
