//! Startup synchronization of the code-owned permission catalog and the
//! built-in roles (docs/specs/login-system.md §4.4).
//!
//! The sync runs in one transaction holding an advisory lock, so multiple API
//! instances can start concurrently without duplicates. Sync is additive with
//! respect to catalog rows and roles: v1 never deletes permissions or roles
//! that disappeared from code. Built-in role permission sets, however, are
//! code-defined and converge exactly.

use rs_admin_domain::permission::PERMISSION_CATALOG;
use rs_admin_domain::rbac::BuiltInRole;
use sqlx::{PgPool, Postgres, Transaction};
use std::fmt;
use uuid::Uuid;

/// Fixed advisory-lock key, derived from the ASCII bytes `RSADMIN` plus a
/// format version byte. It is stable across instances and releases.
pub const CATALOG_SYNC_LOCK_KEY: i64 = 0x5253_4144_4D49_4E01;

/// Errors from the startup sync. Display and debug never include SQL or
/// connection details.
#[derive(thiserror::Error)]
pub enum RbacSyncError {
    #[error("rbac catalog sync failed")]
    Database(#[from] sqlx::Error),
    #[error("built-in role name {0} is held by a custom role")]
    BuiltinRoleNameConflict(&'static str),
}

impl fmt::Debug for RbacSyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RbacSyncError")
            .field(&self.to_string())
            .finish()
    }
}

/// Mirror the catalog and built-in roles into PostgreSQL. Idempotent; safe to
/// call on every startup and from every instance.
pub async fn sync_catalog(pool: &PgPool) -> Result<(), RbacSyncError> {
    let mut transaction = pool.begin().await?;

    // Transaction-scoped: released on commit or rollback, including panics.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(CATALOG_SYNC_LOCK_KEY)
        .execute(&mut *transaction)
        .await?;

    sync_permissions(&mut transaction).await?;
    for role in BuiltInRole::ALL {
        sync_builtin_role(&mut transaction, *role).await?;
    }

    transaction.commit().await?;
    Ok(())
}

async fn sync_permissions(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<(), RbacSyncError> {
    for definition in PERMISSION_CATALOG {
        sqlx::query(
            "INSERT INTO permissions (key, group_key, display_name_zh, display_name_en)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (key) DO UPDATE SET
                 group_key = EXCLUDED.group_key,
                 display_name_zh = EXCLUDED.display_name_zh,
                 display_name_en = EXCLUDED.display_name_en",
        )
        .bind(definition.key.as_str())
        .bind(definition.group.key())
        .bind(definition.display_name_zh)
        .bind(definition.display_name_en)
        .execute(&mut **transaction)
        .await?;
    }

    Ok(())
}

async fn sync_builtin_role(
    transaction: &mut Transaction<'_, Postgres>,
    role: BuiltInRole,
) -> Result<(), RbacSyncError> {
    let existing: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT id, is_builtin
         FROM roles
         WHERE lower(name) = lower($1) AND deleted_at IS NULL
         FOR UPDATE",
    )
    .bind(role.code())
    .fetch_optional(&mut **transaction)
    .await?;

    let role_id = match existing {
        Some((id, true)) => id,
        Some((_, false)) => return Err(RbacSyncError::BuiltinRoleNameConflict(role.code())),
        None => {
            sqlx::query_scalar(
                "INSERT INTO roles (name, is_builtin) VALUES ($1, true) RETURNING id",
            )
            .bind(role.code())
            .fetch_one(&mut **transaction)
            .await?
        }
    };

    let permission_keys: Vec<String> = role
        .permission_keys()
        .iter()
        .map(|key| key.as_str().to_owned())
        .collect();

    // Built-in sets are fixed by code: drop grants that are no longer part of
    // the role. Catalog rows themselves stay for audit and history.
    sqlx::query(
        "DELETE FROM role_permissions
         WHERE role_id = $1 AND NOT (permission_key = ANY($2))",
    )
    .bind(role_id)
    .bind(&permission_keys)
    .execute(&mut **transaction)
    .await?;

    sqlx::query(
        "INSERT INTO role_permissions (role_id, permission_key)
         SELECT $1::uuid, unnest($2::text[])
         ON CONFLICT (role_id, permission_key) DO NOTHING",
    )
    .bind(role_id)
    .bind(&permission_keys)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}
