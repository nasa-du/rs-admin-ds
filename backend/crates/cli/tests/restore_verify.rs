//! Restore-drill verification against a real PostgreSQL schema (AC-OPS-05).

mod support;

use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, AuditWriter};
use rs_admin_cli::{RestoreVerifyError, verify_restored_database};
use rs_admin_security::AuditKeyRing;
use rs_admin_store::expected_migration_version;
use sqlx::PgPool;
use support::{NewUser, insert_user, prepare_catalog, services};
use uuid::Uuid;

fn keyring() -> AuditKeyRing {
    AuditKeyRing::from_json(
        &format!(
            "{{\"{}\":\"{}\"}}",
            support::AUDIT_KEY_ID,
            support::AUDIT_KEY_B64
        ),
        support::AUDIT_KEY_ID,
    )
    .expect("keyring")
}

fn audit_event() -> AuditEvent {
    AuditEvent::builder(
        AuditAction::CryptoKeyRotated,
        AuditResult::Success,
        AuditActor::system(),
        Uuid::now_v7(),
    )
    .detail("key_type", "jwt")
    .build()
    .expect("event")
}

#[sqlx::test(migrations = "../../migrations")]
async fn verifies_the_schema_counts_and_audit_chain(pool: PgPool) {
    prepare_catalog(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: None,
            ..NewUser::active("restore.user", "restore.user@example.com")
        },
    )
    .await;

    let writer = AuditWriter::new(keyring());
    writer.write(&pool, &audit_event()).await.expect("audit");
    writer.write(&pool, &audit_event()).await.expect("audit");

    let verification =
        verify_restored_database(&pool, &keyring(), Some(expected_migration_version()))
            .await
            .expect("restored database verifies");

    assert_eq!(verification.migration_version, expected_migration_version());
    assert!(verification.migrations_ok);
    assert_eq!(verification.row_counts.users, 1);
    assert_eq!(verification.row_counts.roles, 3);
    assert_eq!(verification.row_counts.sessions, 0);
    assert_eq!(verification.row_counts.audit_logs, 2);
    assert_eq!(verification.audit_chain.verified_rows, 2);
    assert_eq!(verification.audit_chain.head_seq, 2);
}

#[sqlx::test(migrations = "../../migrations")]
async fn rejects_a_database_without_key_rows(pool: PgPool) {
    let error = verify_restored_database(&pool, &keyring(), None)
        .await
        .expect_err("a database without users or built-in roles is not usable");
    assert!(matches!(
        error,
        RestoreVerifyError::MissingKeyRows { users: 0, roles: 0 }
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn rejects_an_unexpected_migration_version(pool: PgPool) {
    let expected = expected_migration_version() + 1;
    let error = verify_restored_database(&pool, &keyring(), Some(expected))
        .await
        .expect_err("the version differs");

    assert!(matches!(
        error,
        RestoreVerifyError::UnexpectedMigrationVersion { found, expected: want }
            if found == expected_migration_version() && want == expected
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn rejects_a_failed_migration(pool: PgPool) {
    sqlx::query("UPDATE _sqlx_migrations SET success = false")
        .execute(&pool)
        .await
        .expect("mark failed");

    let error = verify_restored_database(&pool, &keyring(), None)
        .await
        .expect_err("a failed migration is invalid");
    assert!(matches!(error, RestoreVerifyError::FailedMigration));
}

#[sqlx::test(migrations = "../../migrations")]
async fn rejects_missing_migration_history(pool: PgPool) {
    sqlx::query("DELETE FROM _sqlx_migrations")
        .execute(&pool)
        .await
        .expect("clear history");

    let error = verify_restored_database(&pool, &keyring(), None)
        .await
        .expect_err("the migration table is empty");
    assert!(matches!(error, RestoreVerifyError::NoMigrations));
}

#[sqlx::test(migrations = "../../migrations")]
async fn rejects_a_broken_audit_chain(pool: PgPool) {
    prepare_catalog(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: None,
            ..NewUser::active("broken.chain", "broken.chain@example.com")
        },
    )
    .await;

    let writer = services().audit();
    writer.write(&pool, &audit_event()).await.expect("audit");
    writer.write(&pool, &audit_event()).await.expect("audit");

    sqlx::query("UPDATE audit_logs SET details = '{\"tampered\":true}'::jsonb WHERE chain_seq = 1")
        .execute(&pool)
        .await
        .expect("tamper");

    let error = verify_restored_database(&pool, &keyring(), None)
        .await
        .expect_err("a tampered chain is invalid");
    assert!(matches!(error, RestoreVerifyError::Audit(_)));
}
