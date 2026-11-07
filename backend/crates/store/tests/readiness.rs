use rs_admin_store::{
    Database, MIGRATOR, PoolSettings, Readiness, UnreadyReason, expected_migration_version,
};
use sqlx::PgPool;
use std::time::Duration;

#[tokio::test]
async fn unreachable_database_is_unready_without_leaking_details() {
    let database = Database::connect_lazy(
        "postgres://rs_admin:supersecret@127.0.0.1:1/rs_admin",
        &PoolSettings::default(),
    )
    .expect("lazy pool");

    let readiness = database.check_ready(Duration::from_millis(300)).await;

    assert_eq!(
        readiness,
        Readiness::Unready(UnreadyReason::DatabaseUnavailable)
    );
    assert!(!format!("{readiness:?}").contains("supersecret"));
}

#[test]
fn applied_version_must_exactly_match_the_embedded_migrator() {
    let expected = 20261107000000;

    assert_eq!(
        Readiness::from_migration_versions(Some(expected), expected),
        Readiness::Ready
    );
    assert!(matches!(
        Readiness::from_migration_versions(Some(expected - 1), expected),
        Readiness::Unready(UnreadyReason::MigrationMismatch { .. })
    ));
    assert!(matches!(
        Readiness::from_migration_versions(None, expected),
        Readiness::Unready(UnreadyReason::MigrationMismatch { .. })
    ));
    assert!(matches!(
        Readiness::from_migration_versions(Some(expected + 1), expected),
        Readiness::Unready(UnreadyReason::MigrationMismatch { .. })
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn ready_after_migrations_are_applied(pool: PgPool) {
    let database = Database::from_pool(pool);

    assert_eq!(
        database.check_ready(Duration::from_secs(2)).await,
        Readiness::Ready
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn missing_latest_migration_version_is_unready(pool: PgPool) {
    sqlx::query(
        "DELETE FROM _sqlx_migrations WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)",
    )
    .execute(&pool)
    .await
    .expect("remove latest migration record");

    let database = Database::from_pool(pool);
    let readiness = database.check_ready(Duration::from_secs(2)).await;

    assert!(matches!(
        readiness,
        Readiness::Unready(UnreadyReason::MigrationMismatch { .. })
    ));
}

#[sqlx::test(migrations = false)]
async fn migrate_records_every_embedded_migration(pool: PgPool) {
    let database = Database::from_pool(pool);
    database.migrate().await.expect("migrations run");

    let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(database.pool())
        .await
        .expect("migration table exists");

    assert_eq!(recorded as usize, MIGRATOR.iter().count());
    assert_eq!(
        database.check_ready(Duration::from_secs(2)).await,
        Readiness::Ready
    );
}

#[test]
fn embedded_migrator_is_not_empty() {
    assert!(expected_migration_version() > 0);
    assert!(MIGRATOR.iter().next().is_some());
}
