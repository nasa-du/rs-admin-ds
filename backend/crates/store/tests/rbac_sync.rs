//! Idempotent startup sync of the permission catalog and built-in roles
//! (docs/specs/login-system.md §4.4 and §6.2).

use rs_admin_domain::permission::PERMISSION_CATALOG;
use rs_admin_domain::rbac::BuiltInRole;
use rs_admin_store::rbac::{CATALOG_SYNC_LOCK_KEY, RbacSyncError, sync_catalog};
use sqlx::PgPool;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use uuid::Uuid;

const ADMIN_EXPECTED: &[&str] = &[
    "users.read",
    "users.create",
    "users.update",
    "users.set_status",
    "users.reset_password",
    "users.unlock",
    "users.assign_role",
    "sessions.revoke",
    "roles.read",
    "audit.read",
];

async fn builtin_roles(pool: &PgPool) -> BTreeMap<String, Uuid> {
    let rows: Vec<(String, Uuid)> =
        sqlx::query_as("SELECT name, id FROM roles WHERE is_builtin ORDER BY name")
            .fetch_all(pool)
            .await
            .expect("built-in roles");
    rows.into_iter().collect()
}

async fn role_permissions(pool: &PgPool, code: &str) -> BTreeSet<String> {
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT rp.permission_key
         FROM role_permissions rp
         JOIN roles r ON r.id = rp.role_id
         WHERE r.name = $1 AND r.is_builtin",
    )
    .bind(code)
    .fetch_all(pool)
    .await
    .expect("role permissions");
    keys.into_iter().collect()
}

fn expected(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

#[sqlx::test(migrations = "../../migrations")]
async fn sync_creates_the_catalog_and_three_builtin_roles(pool: PgPool) {
    sync_catalog(&pool).await.expect("sync");

    let permission_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM permissions")
        .fetch_one(&pool)
        .await
        .expect("permission count");
    assert_eq!(permission_count as usize, PERMISSION_CATALOG.len());

    for definition in PERMISSION_CATALOG {
        let row: (String, String, String) = sqlx::query_as(
            "SELECT group_key, display_name_zh, display_name_en
             FROM permissions WHERE key = $1",
        )
        .bind(definition.key.as_str())
        .fetch_one(&pool)
        .await
        .expect("catalog row");

        assert_eq!(row.0, definition.group.key());
        assert_eq!(row.1, definition.display_name_zh);
        assert_eq!(row.2, definition.display_name_en);
    }

    let roles = builtin_roles(&pool).await;
    assert_eq!(
        roles.keys().cloned().collect::<Vec<_>>(),
        ["admin", "super_admin", "user"]
    );

    assert!(role_permissions(&pool, "user").await.is_empty());
    assert_eq!(
        role_permissions(&pool, "admin").await,
        expected(ADMIN_EXPECTED)
    );
    assert_eq!(role_permissions(&pool, "super_admin").await, all_keys());
}

fn all_keys() -> BTreeSet<String> {
    PERMISSION_CATALOG
        .iter()
        .map(|definition| definition.key.as_str().to_owned())
        .collect()
}

#[sqlx::test(migrations = "../../migrations")]
async fn sync_is_idempotent_across_restarts(pool: PgPool) {
    sync_catalog(&pool).await.expect("first sync");
    let first_roles = builtin_roles(&pool).await;

    sync_catalog(&pool).await.expect("second sync");
    sync_catalog(&pool).await.expect("third sync");
    let later_roles = builtin_roles(&pool).await;

    assert_eq!(first_roles, later_roles, "built-in role ids must be stable");

    let permission_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM permissions")
        .fetch_one(&pool)
        .await
        .expect("permission count");
    assert_eq!(permission_count as usize, PERMISSION_CATALOG.len());
    assert_eq!(
        role_permissions(&pool, "admin").await,
        expected(ADMIN_EXPECTED)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn sync_updates_catalog_metadata(pool: PgPool) {
    sqlx::query("INSERT INTO permissions (key, group_key, display_name_zh, display_name_en) VALUES ('users.read', 'wrong', '旧名称', 'Old name')")
        .execute(&pool)
        .await
        .expect("stale permission");

    sync_catalog(&pool).await.expect("sync");

    let row: (String, String, String) = sqlx::query_as(
        "SELECT group_key, display_name_zh, display_name_en FROM permissions WHERE key = 'users.read'",
    )
    .fetch_one(&pool)
    .await
    .expect("updated permission");

    assert_eq!(row.0, "users");
    assert_eq!(row.1, "查看用户");
    assert_eq!(row.2, "View users");
}

#[sqlx::test(migrations = "../../migrations")]
async fn sync_converges_builtin_permission_sets_without_touching_other_items(pool: PgPool) {
    sync_catalog(&pool).await.expect("initial sync");
    let admin_id = builtin_roles(&pool).await["admin"];

    sqlx::query(
        "INSERT INTO role_permissions (role_id, permission_key) VALUES ($1, 'roles.create')",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("extra admin permission");
    sqlx::query(
        "DELETE FROM role_permissions WHERE role_id = $1 AND permission_key = 'users.read'",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("missing admin permission");

    sqlx::query("INSERT INTO permissions (key, group_key, display_name_zh, display_name_en) VALUES ('legacy.read', 'legacy', '旧权限', 'Legacy view')")
        .execute(&pool)
        .await
        .expect("legacy permission");
    sqlx::query(
        "INSERT INTO role_permissions (role_id, permission_key) VALUES ($1, 'legacy.read')",
    )
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("legacy grant");
    sqlx::query("INSERT INTO roles (name, description) VALUES ('Legacy Role', 'kept')")
        .execute(&pool)
        .await
        .expect("legacy role");

    sync_catalog(&pool).await.expect("second sync");

    assert_eq!(
        role_permissions(&pool, "admin").await,
        expected(ADMIN_EXPECTED),
        "built-in permission sets are code-defined"
    );

    let legacy_permission: Option<String> =
        sqlx::query_scalar("SELECT key FROM permissions WHERE key = 'legacy.read'")
            .fetch_optional(&pool)
            .await
            .expect("legacy permission query");
    assert_eq!(
        legacy_permission.as_deref(),
        Some("legacy.read"),
        "v1 never deletes catalog rows"
    );

    let legacy_role: Option<String> =
        sqlx::query_scalar("SELECT name FROM roles WHERE name = 'Legacy Role'")
            .fetch_optional(&pool)
            .await
            .expect("legacy role query");
    assert_eq!(legacy_role.as_deref(), Some("Legacy Role"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_custom_role_holding_a_builtin_name_fails_the_sync_atomically(pool: PgPool) {
    sqlx::query("INSERT INTO roles (name) VALUES ('admin')")
        .execute(&pool)
        .await
        .expect("custom role with a reserved name");

    let error = sync_catalog(&pool)
        .await
        .expect_err("sync must fail closed");

    assert!(matches!(
        error,
        RbacSyncError::BuiltinRoleNameConflict("admin")
    ));

    let permission_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM permissions")
        .fetch_one(&pool)
        .await
        .expect("permission count");
    assert_eq!(permission_count, 0, "the failed sync must roll back");
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_syncs_serialize_on_the_advisory_lock(pool: PgPool) {
    let (first, second) = tokio::join!(sync_catalog(&pool), sync_catalog(&pool));

    first.expect("first concurrent sync");
    second.expect("second concurrent sync");

    assert_eq!(builtin_roles(&pool).await.len(), 3);
    assert_eq!(
        role_permissions(&pool, "admin").await,
        expected(ADMIN_EXPECTED)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn sync_waits_for_the_catalog_advisory_lock(pool: PgPool) {
    let mut blocker = pool.begin().await.expect("blocker transaction");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(CATALOG_SYNC_LOCK_KEY)
        .execute(&mut *blocker)
        .await
        .expect("hold catalog lock");

    let mut sync = Box::pin(sync_catalog(&pool));
    let blocked = tokio::time::timeout(Duration::from_millis(200), &mut sync).await;
    assert!(
        blocked.is_err(),
        "sync must wait while another session holds the lock"
    );

    blocker.commit().await.expect("release catalog lock");
    tokio::time::timeout(Duration::from_secs(5), sync)
        .await
        .expect("sync finishes after the lock is released")
        .expect("sync succeeds");

    assert_eq!(builtin_roles(&pool).await.len(), 3);
}

#[test]
fn builtin_role_codes_cover_the_three_roles() {
    let codes: Vec<_> = BuiltInRole::ALL.iter().map(|role| role.code()).collect();
    assert_eq!(codes, ["user", "admin", "super_admin"]);
}
