//! Role reads and writes for the roles API (docs/specs/login-system.md §6.3,
//! §6.4, §7.8).
//!
//! List queries always hide soft-deleted roles and never filter by the
//! operator's management scope: the API contract exposes every active role and
//! expresses eligibility through the `assignable` / `manageable` flags.
//! Mutations run inside the caller's transaction so the audit event and the
//! business change commit or roll back together.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use std::fmt;
use uuid::Uuid;

/// Unique index that reserves active role names case-insensitively.
const ROLES_NAME_ACTIVE_UQ: &str = "roles_name_active_uq";
const UNIQUE_VIOLATION: &str = "23505";

/// A role with its permission keys, ordered by key.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct RoleRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub is_builtin: bool,
    pub permission_keys: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Sortable role columns; the API layer validates the wire value against this
/// closed set before building a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleSortField {
    Name,
    CreatedAt,
    UpdatedAt,
}

impl RoleSortField {
    fn sql(self) -> &'static str {
        match self {
            Self::Name => "r.name",
            Self::CreatedAt => "r.created_at",
            Self::UpdatedAt => "r.updated_at",
        }
    }
}

/// A validated role list sort. Defaults to `name asc` (spec §7.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleSort {
    pub field: RoleSortField,
    pub ascending: bool,
}

impl Default for RoleSort {
    fn default() -> Self {
        Self {
            field: RoleSortField::Name,
            ascending: true,
        }
    }
}

/// Validated list filters and offset pagination.
#[derive(Clone, Copy, Debug)]
pub struct RoleFilter<'a> {
    pub q: Option<&'a str>,
    pub is_builtin: Option<bool>,
    pub sort: RoleSort,
    pub limit: i64,
    pub offset: i64,
}

/// One page of roles plus the filtered total.
#[derive(Clone, Debug)]
pub struct RolePage {
    pub items: Vec<RoleRow>,
    pub total: i64,
}

/// Errors from the role store. Display text is stable and never includes SQL
/// or connection details.
#[derive(thiserror::Error)]
pub enum RoleStoreError {
    #[error("role store query failed")]
    Database(#[source] sqlx::Error),
    #[error("role name is already in use")]
    NameTaken,
}

impl fmt::Debug for RoleStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RoleStoreError")
            .field(&self.to_string())
            .finish()
    }
}

fn classify(error: sqlx::Error) -> RoleStoreError {
    let sqlx::Error::Database(database) = &error else {
        return RoleStoreError::Database(error);
    };
    if database.code().as_deref() == Some(UNIQUE_VIOLATION)
        && database.constraint() == Some(ROLES_NAME_ACTIVE_UQ)
    {
        return RoleStoreError::NameTaken;
    }
    RoleStoreError::Database(error)
}

/// One projection for every role read, so the column list and permission
/// aggregation cannot drift between list, detail and lock queries.
macro_rules! role_select {
    () => {
        concat!(
            "SELECT r.id, r.name, r.description, r.is_builtin, r.created_at, r.updated_at, ",
            "ARRAY(SELECT rp.permission_key FROM role_permissions rp ",
            "WHERE rp.role_id = r.id ORDER BY rp.permission_key) AS permission_keys ",
            "FROM roles r WHERE r.deleted_at IS NULL"
        )
    };
    (by id) => {
        concat!(role_select!(), " AND r.id = $1")
    };
    (by id for update) => {
        concat!(role_select!(by id), " FOR UPDATE")
    };
}

const ROLE_SELECT: &str = role_select!();
const ROLE_SELECT_BY_ID: &str = role_select!(by id);
const ROLE_SELECT_BY_ID_FOR_UPDATE: &str = role_select!(by id for update);

/// List active roles matching the filters. `q` searches name and description
/// case-insensitively; wildcard characters in `q` are treated literally.
pub async fn list_roles(pool: &PgPool, filter: RoleFilter<'_>) -> Result<RolePage, RoleStoreError> {
    let mut rows = QueryBuilder::<Postgres>::new(ROLE_SELECT);
    push_filters(&mut rows, &filter);
    push_sort(&mut rows, filter.sort);
    rows.push(" LIMIT ").push_bind(filter.limit);
    rows.push(" OFFSET ").push_bind(filter.offset);

    let items = rows
        .build_query_as::<RoleRow>()
        .fetch_all(pool)
        .await
        .map_err(RoleStoreError::Database)?;

    let mut count =
        QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM roles r WHERE r.deleted_at IS NULL");
    push_filters(&mut count, &filter);
    let total = count
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await
        .map_err(RoleStoreError::Database)?;

    Ok(RolePage { items, total })
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &RoleFilter<'_>) {
    if let Some(term) = filter.q {
        let pattern = like_pattern(term);
        query
            .push(" AND (r.name ILIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR r.description ILIKE ")
            .push_bind(pattern)
            .push(" ESCAPE '\\')");
    }
    if let Some(is_builtin) = filter.is_builtin {
        query.push(" AND r.is_builtin = ").push_bind(is_builtin);
    }
}

fn push_sort(query: &mut QueryBuilder<Postgres>, sort: RoleSort) {
    query.push(" ORDER BY ").push(sort.field.sql());
    query.push(if sort.ascending { " ASC" } else { " DESC" });
    // Stable tie-break so pagination cannot repeat or skip a row.
    query.push(", r.id ASC");
}

fn like_pattern(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len() + 2);
    escaped.push('%');
    for character in term.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('%');
    escaped
}

/// Load one active role by id. Soft-deleted roles are indistinguishable from
/// missing ones.
pub async fn find_role(pool: &PgPool, id: Uuid) -> Result<Option<RoleRow>, RoleStoreError> {
    sqlx::query_as::<_, RoleRow>(ROLE_SELECT_BY_ID)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(RoleStoreError::Database)
}

/// Same as [`find_role`], inside a caller's transaction.
pub async fn find_role_in(
    connection: &mut PgConnection,
    id: Uuid,
) -> Result<Option<RoleRow>, RoleStoreError> {
    sqlx::query_as::<_, RoleRow>(ROLE_SELECT_BY_ID)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(RoleStoreError::Database)
}

/// Lock the role row for the rest of the transaction, serializing concurrent
/// metadata, permission and delete changes.
pub async fn lock_role(
    connection: &mut PgConnection,
    id: Uuid,
) -> Result<Option<RoleRow>, RoleStoreError> {
    sqlx::query_as::<_, RoleRow>(ROLE_SELECT_BY_ID_FOR_UPDATE)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(RoleStoreError::Database)
}

/// Insert a custom role and return its generated id. Built-in names are
/// reserved because built-in roles already occupy them in the active-name
/// index.
pub async fn insert_role(
    connection: &mut PgConnection,
    name: &str,
    description: Option<&str>,
) -> Result<Uuid, RoleStoreError> {
    sqlx::query_scalar(
        "INSERT INTO roles (name, description, is_builtin)
         VALUES ($1, $2, false)
         RETURNING id",
    )
    .bind(name)
    .bind(description)
    .fetch_one(&mut *connection)
    .await
    .map_err(classify)
}

/// Update metadata. A `None` name leaves it unchanged; `Some(None)` clears the
/// description, while `None` leaves it unchanged.
pub async fn update_role(
    connection: &mut PgConnection,
    id: Uuid,
    name: Option<&str>,
    description: Option<Option<&str>>,
) -> Result<(), RoleStoreError> {
    sqlx::query(
        "UPDATE roles
         SET name = COALESCE($2, name),
             description = CASE WHEN $3 THEN $4 ELSE description END,
             updated_at = now()
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(name)
    .bind(description.is_some())
    .bind(description.flatten())
    .execute(&mut *connection)
    .await
    .map_err(classify)?;
    Ok(())
}

/// Replace a role's permission set and bump `updated_at`.
pub async fn replace_permissions(
    connection: &mut PgConnection,
    id: Uuid,
    permission_keys: &[String],
) -> Result<(), RoleStoreError> {
    sqlx::query("DELETE FROM role_permissions WHERE role_id = $1")
        .bind(id)
        .execute(&mut *connection)
        .await
        .map_err(RoleStoreError::Database)?;

    sqlx::query(
        "INSERT INTO role_permissions (role_id, permission_key)
         SELECT $1::uuid, unnest($2::text[])",
    )
    .bind(id)
    .bind(permission_keys)
    .execute(&mut *connection)
    .await
    .map_err(RoleStoreError::Database)?;

    sqlx::query("UPDATE roles SET updated_at = now() WHERE id = $1 AND deleted_at IS NULL")
        .bind(id)
        .execute(&mut *connection)
        .await
        .map_err(RoleStoreError::Database)?;

    Ok(())
}

/// Count every assignment to the role, including disabled users (spec §6.3).
pub async fn count_assignments(
    connection: &mut PgConnection,
    id: Uuid,
) -> Result<i64, RoleStoreError> {
    sqlx::query_scalar("SELECT COUNT(*) FROM user_roles WHERE role_id = $1")
        .bind(id)
        .fetch_one(&mut *connection)
        .await
        .map_err(RoleStoreError::Database)
}

/// Soft-delete a custom role. The active-name unique index no longer covers
/// the row, so the name can be reused.
pub async fn soft_delete_role(
    connection: &mut PgConnection,
    id: Uuid,
) -> Result<(), RoleStoreError> {
    sqlx::query(
        "UPDATE roles
         SET deleted_at = now(), updated_at = now()
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *connection)
    .await
    .map_err(RoleStoreError::Database)?;
    Ok(())
}
