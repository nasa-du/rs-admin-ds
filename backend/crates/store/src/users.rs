//! User reads and writes for the user management API
//! (docs/specs/login-system.md §6.3, §6.6, §7.7).
//!
//! List queries apply the operator's management scope in SQL so pagination and
//! `total` describe the same filtered set. Writes run inside the caller's
//! transaction so the audit event and the business change commit or roll back
//! together.

use crate::identity::IdentifierConflict;
use chrono::{DateTime, Utc};
use rs_admin_domain::lifecycle::UserStatus;
use rs_admin_domain::rbac::{BASELINE_ROLE, BuiltInRole};
use sqlx::{Executor, PgConnection, PgPool, Postgres, QueryBuilder, Transaction};
use std::fmt;
use std::time::Duration;
use uuid::Uuid;

/// A user row as exposed by the management API. The password hash is
/// deliberately absent: it must never reach a response DTO.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub phone: Option<String>,
    pub status: String,
    pub must_change_password: bool,
    pub temporary_password_expires_at: Option<DateTime<Utc>>,
    pub locked_until: Option<DateTime<Utc>>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A role held by a user, as rendered in `UserSummary.roles`.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct UserRoleRef {
    pub user_id: Uuid,
    pub role_id: Uuid,
    pub name: String,
    pub is_builtin: bool,
}

/// Sortable user columns; the API layer validates the wire value against this
/// closed set before building a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserSortField {
    Username,
    Email,
    DisplayName,
    Status,
    CreatedAt,
    LastLoginAt,
    UpdatedAt,
}

impl UserSortField {
    fn sql(self) -> &'static str {
        match self {
            Self::Username => "u.username",
            Self::Email => "u.email",
            Self::DisplayName => "u.display_name",
            Self::Status => "u.status",
            Self::CreatedAt => "u.created_at",
            Self::LastLoginAt => "u.last_login_at",
            Self::UpdatedAt => "u.updated_at",
        }
    }
}

/// A validated user list sort. Defaults to `created_at desc` (spec §7.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserSort {
    pub field: UserSortField,
    pub ascending: bool,
}

impl Default for UserSort {
    fn default() -> Self {
        Self {
            field: UserSortField::CreatedAt,
            ascending: false,
        }
    }
}

/// The operator's management scope (spec §6.3 rule 4).
#[derive(Clone, Copy, Debug)]
pub enum UserScope<'a> {
    /// Super administrators are unrestricted.
    All,
    /// Non-super-admins see themselves plus users whose roles grant no more
    /// than the operator's effective permissions and who hold neither the
    /// built-in `admin` nor `super_admin` role.
    Restricted {
        operator_id: Uuid,
        allowed_permission_keys: &'a [String],
    },
}

/// Validated list filters and offset pagination.
#[derive(Clone, Copy, Debug)]
pub struct UserFilter<'a> {
    pub q: Option<&'a str>,
    pub status: Option<&'a str>,
    pub role_id: Option<Uuid>,
    pub is_locked: Option<bool>,
    pub must_change_password: Option<bool>,
    pub sort: UserSort,
    pub limit: i64,
    pub offset: i64,
    pub scope: UserScope<'a>,
}

/// One page of users plus the filtered total.
#[derive(Clone, Debug)]
pub struct UserPage {
    pub items: Vec<UserRow>,
    pub total: i64,
}

/// Errors from the user store. Display text is stable and never includes SQL
/// or connection details.
#[derive(thiserror::Error)]
pub enum UserStoreError {
    #[error("user store query failed")]
    Database(#[source] sqlx::Error),
    #[error("user identifier is already in use")]
    IdentifierTaken(IdentifierConflict),
}

impl fmt::Debug for UserStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("UserStoreError")
            .field(&self.to_string())
            .finish()
    }
}

fn classify(error: sqlx::Error) -> UserStoreError {
    match IdentifierConflict::from_unique_violation(&error) {
        Some(conflict) => UserStoreError::IdentifierTaken(conflict),
        None => UserStoreError::Database(error),
    }
}

/// One projection for every user read, so the column list cannot drift between
/// list, detail and update responses.
macro_rules! user_select {
    () => {
        "SELECT u.id, u.username, u.email, u.display_name, u.phone, u.status, \
         u.must_change_password, u.temporary_password_expires_at, u.locked_until, \
         u.last_login_at, u.created_at, u.updated_at FROM users u WHERE 1 = 1"
    };
    (by id) => {
        concat!(user_select!(), " AND u.id = $1")
    };
}

const USER_SELECT: &str = user_select!();
const USER_SELECT_BY_ID: &str = user_select!(by id);

/// List users matching the filters and scope. `q` searches username, email and
/// display name case-insensitively; wildcard characters are literal.
pub async fn list_users(pool: &PgPool, filter: UserFilter<'_>) -> Result<UserPage, UserStoreError> {
    let mut rows = QueryBuilder::<Postgres>::new(USER_SELECT);
    push_filters(&mut rows, &filter);
    push_sort(&mut rows, filter.sort);
    rows.push(" LIMIT ").push_bind(filter.limit);
    rows.push(" OFFSET ").push_bind(filter.offset);

    let items = rows
        .build_query_as::<UserRow>()
        .fetch_all(pool)
        .await
        .map_err(UserStoreError::Database)?;

    let mut count = QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM users u WHERE 1 = 1");
    push_filters(&mut count, &filter);
    let total = count
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await
        .map_err(UserStoreError::Database)?;

    Ok(UserPage { items, total })
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &UserFilter<'_>) {
    if let Some(term) = filter.q {
        let pattern = like_pattern(term);
        query
            .push(" AND (u.username ILIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR u.email ILIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR u.display_name ILIKE ")
            .push_bind(pattern)
            .push(" ESCAPE '\\')");
    }
    if let Some(status) = filter.status {
        query.push(" AND u.status = ").push_bind(status.to_owned());
    }
    if let Some(role_id) = filter.role_id {
        query
            .push(" AND EXISTS (SELECT 1 FROM user_roles ur WHERE ur.user_id = u.id AND ur.role_id = ")
            .push_bind(role_id)
            .push(")");
    }
    if let Some(is_locked) = filter.is_locked {
        if is_locked {
            query.push(" AND u.locked_until IS NOT NULL AND u.locked_until > now()");
        } else {
            query.push(" AND (u.locked_until IS NULL OR u.locked_until <= now())");
        }
    }
    if let Some(must_change_password) = filter.must_change_password {
        query
            .push(" AND u.must_change_password = ")
            .push_bind(must_change_password);
    }
    if let UserScope::Restricted {
        operator_id,
        allowed_permission_keys,
    } = filter.scope
    {
        // Privileged and catalog-known built-in names come from the domain
        // catalog, so a future built-in role cannot silently fall out of the
        // scope rules.
        let privileged_codes: Vec<String> = [BuiltInRole::Admin, BuiltInRole::SuperAdmin]
            .iter()
            .map(|role| role.code().to_owned())
            .collect();
        let all_codes: Vec<String> = BuiltInRole::ALL
            .iter()
            .map(|role| role.code().to_owned())
            .collect();

        query
            .push(" AND (u.id = ")
            .push_bind(operator_id)
            .push(
                " OR (\
                 NOT EXISTS (SELECT 1 FROM user_roles ur \
                     JOIN roles r ON r.id = ur.role_id AND r.deleted_at IS NULL \
                     WHERE ur.user_id = u.id AND r.is_builtin \
                       AND r.name = ANY(",
            )
            .push_bind(privileged_codes)
            .push(
                ")) AND NOT EXISTS (SELECT 1 FROM user_roles ur \
                     JOIN roles r ON r.id = ur.role_id AND r.deleted_at IS NULL \
                     WHERE ur.user_id = u.id AND r.is_builtin \
                       AND NOT (r.name = ANY(",
            )
            .push_bind(all_codes)
            .push(
                "))) AND NOT EXISTS (SELECT 1 FROM user_roles ur \
                     JOIN roles r ON r.id = ur.role_id AND r.deleted_at IS NULL \
                     JOIN role_permissions rp ON rp.role_id = r.id \
                     WHERE ur.user_id = u.id AND NOT r.is_builtin \
                       AND NOT (rp.permission_key = ANY(",
            )
            .push_bind(allowed_permission_keys.to_vec())
            .push(")))))");
    }
}

fn push_sort(query: &mut QueryBuilder<Postgres>, sort: UserSort) {
    query.push(" ORDER BY ").push(sort.field.sql());
    query.push(if sort.ascending { " ASC" } else { " DESC" });
    // Never-logged-in users sort after every dated row in both directions.
    if sort.field == UserSortField::LastLoginAt {
        query.push(" NULLS LAST");
    }
    // Stable tie-break so pagination cannot repeat or skip a row.
    query.push(", u.id ASC");
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

/// Load one user by id, independent of management scope.
pub async fn find_user(pool: &PgPool, id: Uuid) -> Result<Option<UserRow>, UserStoreError> {
    sqlx::query_as(USER_SELECT_BY_ID)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(UserStoreError::Database)
}

/// Same as [`find_user`] inside a caller's transaction.
pub async fn find_user_in(
    connection: &mut PgConnection,
    id: Uuid,
) -> Result<Option<UserRow>, UserStoreError> {
    sqlx::query_as(USER_SELECT_BY_ID)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(UserStoreError::Database)
}

/// Role references for the given users, grouped by user id in caller order
/// within each user: built-in roles first, then by name.
pub async fn load_user_role_refs<'e, E>(
    executor: E,
    user_ids: &[Uuid],
) -> Result<Vec<UserRoleRef>, UserStoreError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        "SELECT ur.user_id, r.id AS role_id, r.name, r.is_builtin
         FROM user_roles ur
         JOIN roles r ON r.id = ur.role_id AND r.deleted_at IS NULL
         WHERE ur.user_id = ANY($1)
         ORDER BY ur.user_id, r.is_builtin DESC, r.name",
    )
    .bind(user_ids)
    .fetch_all(executor)
    .await
    .map_err(UserStoreError::Database)
}

/// Parameters for an administrator-created user. The caller has already
/// normalized and validated every value and hashed the temporary password.
pub struct NewUserRow<'a> {
    pub username: &'a str,
    pub email: &'a str,
    pub display_name: &'a str,
    pub phone: Option<&'a str>,
    pub password_hash: &'a str,
    pub temporary_password_ttl: Duration,
}

/// Insert an enabled user holding a temporary password. The database clock
/// defines the expiry. Returns the generated id and expiry timestamp.
pub async fn insert_user(
    transaction: &mut Transaction<'_, Postgres>,
    user: NewUserRow<'_>,
) -> Result<(Uuid, DateTime<Utc>), UserStoreError> {
    sqlx::query_as(
        "INSERT INTO users (
            username, email, display_name, phone, password_hash, status,
            must_change_password, temporary_password_expires_at
         ) VALUES (
            $1, $2, $3, $4, $5, 'active',
            true, now() + make_interval(secs => $6)
         )
         RETURNING id, temporary_password_expires_at",
    )
    .bind(user.username)
    .bind(user.email)
    .bind(user.display_name)
    .bind(user.phone)
    .bind(user.password_hash)
    .bind(user.temporary_password_ttl.as_secs_f64())
    .fetch_one(&mut **transaction)
    .await
    .map_err(classify)
}

/// Grant the immovable baseline `user` role. Idempotent.
pub async fn assign_baseline_role(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    assigned_by: Option<Uuid>,
) -> Result<(), UserStoreError> {
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id, assigned_by)
         SELECT $1, id, $2
         FROM roles
         WHERE name = 'user' AND is_builtin AND deleted_at IS NULL
         ON CONFLICT (user_id, role_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(assigned_by)
    .execute(&mut **transaction)
    .await
    .map_err(UserStoreError::Database)?;
    Ok(())
}

/// Grant the given roles to a user. Idempotent for duplicate assignments.
pub async fn assign_roles(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    role_ids: &[Uuid],
    assigned_by: Option<Uuid>,
) -> Result<(), UserStoreError> {
    if role_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id, assigned_by)
         SELECT $1, unnest($2::uuid[]), $3
         ON CONFLICT (user_id, role_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(role_ids)
    .bind(assigned_by)
    .execute(&mut **transaction)
    .await
    .map_err(UserStoreError::Database)?;
    Ok(())
}

/// The id of the immovable baseline `user` role.
pub async fn baseline_role_id<'e, E>(executor: E) -> Result<Option<Uuid>, UserStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar("SELECT id FROM roles WHERE name = $1 AND is_builtin AND deleted_at IS NULL")
        .bind(BASELINE_ROLE.code())
        .fetch_optional(executor)
        .await
        .map_err(UserStoreError::Database)
}

/// Replace a user's role set. The baseline `user` role is preserved even when
/// the caller's list omits it, so an administrator can never strip it. Grants
/// not in `role_ids` are removed; new grants are attributed to `assigned_by`.
pub async fn replace_user_roles(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    role_ids: &[Uuid],
    assigned_by: Option<Uuid>,
) -> Result<(), UserStoreError> {
    sqlx::query(
        "DELETE FROM user_roles ur
         USING roles r
         WHERE ur.user_id = $1
           AND ur.role_id = r.id
           AND NOT (ur.role_id = ANY($2::uuid[]))
           AND NOT (r.is_builtin AND r.name = $3)",
    )
    .bind(user_id)
    .bind(role_ids)
    .bind(BASELINE_ROLE.code())
    .execute(&mut **transaction)
    .await
    .map_err(UserStoreError::Database)?;

    if role_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id, assigned_by)
         SELECT $1, unnest($2::uuid[]), $3
         ON CONFLICT (user_id, role_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(role_ids)
    .bind(assigned_by)
    .execute(&mut **transaction)
    .await
    .map_err(UserStoreError::Database)?;
    Ok(())
}

/// Ids of enabled holders of the built-in `super_admin` role, in a stable
/// order.
macro_rules! enabled_super_admin_select {
    () => {
        "SELECT u.id
         FROM users u
         WHERE u.status = 'active'
           AND EXISTS (
               SELECT 1
               FROM user_roles ur
               JOIN roles r ON r.id = ur.role_id
               WHERE ur.user_id = u.id
                 AND r.is_builtin
                 AND r.name = $1
                 AND r.deleted_at IS NULL
           )
         ORDER BY u.id"
    };
    (for update) => {
        concat!(enabled_super_admin_select!(), " FOR UPDATE")
    };
}

const ENABLED_SUPER_ADMIN_SELECT: &str = enabled_super_admin_select!();

/// Lock every enabled holder of the built-in `super_admin` role and return
/// their ids in a stable order. Serializing the last-super-admin invariant on
/// these rows makes concurrent disables unable to reach zero.
///
/// Role changes do not update the `users` row, so the qualification of a
/// blocked `FOR UPDATE` scan may still reflect the pre-wait snapshot. Callers
/// that make the invariant decision must re-read with
/// [`find_enabled_super_admin_ids`] after acquiring every lock.
pub async fn lock_enabled_super_admin_ids(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<Vec<Uuid>, UserStoreError> {
    sqlx::query_scalar(enabled_super_admin_select!(for update))
        .bind(BuiltInRole::SuperAdmin.code())
        .fetch_all(&mut **transaction)
        .await
        .map_err(UserStoreError::Database)
}

/// Re-read the enabled super administrators with a fresh snapshot. Callers
/// hold the user row locks acquired by [`lock_enabled_super_admin_ids`], so no
/// concurrent demotion or disable can change this set before they commit.
pub async fn find_enabled_super_admin_ids<'e, E>(executor: E) -> Result<Vec<Uuid>, UserStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar(ENABLED_SUPER_ADMIN_SELECT)
        .bind(BuiltInRole::SuperAdmin.code())
        .fetch_all(executor)
        .await
        .map_err(UserStoreError::Database)
}

/// Set the account status.
pub async fn update_user_status(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    status: UserStatus,
) -> Result<(), UserStoreError> {
    sqlx::query("UPDATE users SET status = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(status.as_str())
        .execute(&mut **transaction)
        .await
        .map_err(UserStoreError::Database)?;
    Ok(())
}

/// Install a new temporary password, clear the account-level login lock and
/// return the database-defined expiry. The account status is left unchanged:
/// a reset for a disabled user must not silently enable it.
pub async fn reset_temporary_password(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    password_hash: &str,
    temporary_password_ttl: Duration,
) -> Result<DateTime<Utc>, UserStoreError> {
    sqlx::query_scalar(
        "UPDATE users
         SET password_hash = $2,
             must_change_password = true,
             temporary_password_expires_at = now() + make_interval(secs => $3),
             failed_login_count = 0,
             locked_until = NULL,
             last_failed_login_at = NULL,
             updated_at = now()
         WHERE id = $1
         RETURNING temporary_password_expires_at",
    )
    .bind(user_id)
    .bind(password_hash)
    .bind(temporary_password_ttl.as_secs_f64())
    .fetch_one(&mut **transaction)
    .await
    .map_err(UserStoreError::Database)
}

/// Update the administrator-managed profile fields. `None` leaves a field
/// unchanged; `Some(None)` clears the nullable phone. Callers validate and
/// normalize every value first.
pub async fn update_user(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    username: Option<&str>,
    email: Option<&str>,
    display_name: Option<&str>,
    phone: Option<Option<&str>>,
) -> Result<(), UserStoreError> {
    sqlx::query(
        "UPDATE users
         SET username = COALESCE($2, username),
             email = COALESCE($3, email),
             display_name = COALESCE($4, display_name),
             phone = CASE WHEN $5 THEN $6 ELSE phone END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(username)
    .bind(email)
    .bind(display_name)
    .bind(phone.is_some())
    .bind(phone.flatten())
    .execute(&mut **transaction)
    .await
    .map_err(classify)?;
    Ok(())
}
