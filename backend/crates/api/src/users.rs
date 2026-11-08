//! User management endpoints: list, detail, create and edit
//! (docs/specs/login-system.md §6.3, §6.6, §7.7).
//!
//! Reads are filtered by the operator's management scope in SQL, with `self`
//! always visible. Writes enforce endpoint permission, target eligibility
//! (management scope and the grant-subset rule), self-action prohibitions and
//! step-up freshness before touching the database. Each successful change and
//! each denial is appended to the audit chain.

use crate::auth::{
    AuthServices, Authenticated, RoleRef, SessionPrincipal, password_unavailable, write_audit_event,
};
use crate::authz::{
    Denial, DenialTarget, RequestContext, require_permission, require_recent_auth, write_denial,
};
use crate::client_ip::ClientIp;
use crate::error::{ApiError, FieldError};
use crate::json::ApiJson;
use crate::middleware::RequestId;
use crate::query::{parse_id, parse_pagination, validation_error};
use crate::roles::role_is_assignable;
use crate::router::AppState;
use axum::Extension;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::http::header::LOCATION;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, TargetType};
use rs_admin_domain::ErrorCode;
use rs_admin_domain::identity::{DisplayName, Email, IdentityError, Phone, Username};
use rs_admin_domain::permission::PermissionKey;
use rs_admin_domain::rbac::{BuiltInRole, EffectivePermissions, RoleTarget};
use rs_admin_security::{PasswordPolicy, PolicyViolation, generate_temporary_password};
use rs_admin_store::auth::{UserRole, load_role_grants, lock_user_row};
use rs_admin_store::roles::{RoleRow, find_roles_by_ids, lock_roles_by_ids};
use rs_admin_store::users::{
    NewUserRow, UserFilter, UserRow, UserScope, UserSort, UserSortField, UserStoreError,
    assign_baseline_role, assign_roles, find_user, find_user_in, insert_user,
    list_users as store_list_users, load_user_role_refs, update_user as store_update_user,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Request and response types
// ---------------------------------------------------------------------------

/// Query parameters for `GET /users`. Unknown parameters are ignored.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListUsersQuery {
    /// Case-insensitive substring search over username, email and display
    /// name.
    pub q: Option<String>,
    /// `active` or `disabled`.
    pub status: Option<String>,
    /// Users holding this role, including the baseline `user` role.
    pub role_id: Option<String>,
    /// `true` when `locked_until > now()`.
    #[param(value_type = Option<bool>)]
    pub is_locked: Option<String>,
    /// Exact match on `must_change_password`.
    #[param(value_type = Option<bool>)]
    pub must_change_password: Option<String>,
    /// One-based page number; defaults to 1.
    #[param(value_type = Option<i64>)]
    pub page: Option<String>,
    /// Page size; defaults to 20 and may not exceed 200.
    #[param(value_type = Option<i64>)]
    pub page_size: Option<String>,
    /// `username`, `email`, `display_name`, `status`, `created_at`,
    /// `last_login_at` or `updated_at`, optionally prefixed with `-`;
    /// defaults to `-created_at`.
    pub sort: Option<String>,
}

/// `POST /users` request body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    /// Omitted means a 24-character temporary password is generated; a
    /// supplied value must satisfy the password policy.
    #[serde(default)]
    pub password: Option<String>,
    /// Extra roles beyond the immovable baseline `user` role. Supplying this
    /// field requires `users.assign_role`.
    #[serde(default)]
    pub role_ids: Option<Vec<String>>,
}

/// `POST /users` success body. The temporary password is shown only in this
/// response and is never persisted in plaintext or written to audit.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserCreatedResponse {
    pub user: UserDetail,
    pub temporary_password: String,
    pub temporary_password_expires_at: String,
}

/// `PATCH /users/{id}` request body. Omitted fields are unchanged; an explicit
/// `null` phone clears it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateUserRequest {
    #[serde(default, deserialize_with = "crate::json::non_null_display_name")]
    #[schema(nullable = false)]
    pub display_name: Option<String>,
    #[serde(default, deserialize_with = "crate::json::non_null_username")]
    #[schema(nullable = false)]
    pub username: Option<String>,
    #[serde(default, deserialize_with = "crate::json::non_null_email")]
    #[schema(nullable = false)]
    pub email: Option<String>,
    #[serde(default, deserialize_with = "crate::json::double_option")]
    pub phone: Option<Option<String>>,
}

/// A user as returned by the list endpoint.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UserSummary {
    pub id: String,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub phone: Option<String>,
    pub status: String,
    pub roles: Vec<RoleRef>,
    pub must_change_password: bool,
    pub locked_until: Option<String>,
    pub last_login_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A single user. `UserDetail` is `UserSummary` plus the temporary-password
/// expiry; it never exposes password material or failure counters.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UserDetail {
    pub id: String,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub phone: Option<String>,
    pub status: String,
    pub roles: Vec<RoleRef>,
    pub must_change_password: bool,
    pub locked_until: Option<String>,
    pub last_login_at: Option<String>,
    pub temporary_password_expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<UserSummary> for UserDetail {
    fn from(summary: UserSummary) -> Self {
        Self {
            id: summary.id,
            username: summary.username,
            email: summary.email,
            display_name: summary.display_name,
            phone: summary.phone,
            status: summary.status,
            roles: summary.roles,
            must_change_password: summary.must_change_password,
            locked_until: summary.locked_until,
            last_login_at: summary.last_login_at,
            temporary_password_expires_at: None,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

/// Offset-paginated user list envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserListResponse {
    pub items: Vec<UserSummary>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn store_unavailable(error: impl std::fmt::Display) -> ApiError {
    tracing::warn!(error = %error, "user store operation failed");
    ApiError::service_unavailable()
}

/// The management scope for the current operator (spec §6.3 rule 4).
fn user_scope<'a>(principal: &Authenticated, allowed_keys: &'a [String]) -> UserScope<'a> {
    if principal.0.effective.is_super_admin() {
        UserScope::All
    } else {
        UserScope::Restricted {
            operator_id: principal.0.user.id,
            allowed_permission_keys: allowed_keys,
        }
    }
}

/// Effective permission keys in the wire form the scope SQL compares against.
fn allowed_permission_keys(principal: &Authenticated) -> Vec<String> {
    principal
        .0
        .effective
        .permission_keys()
        .iter()
        .map(|key| key.as_str().to_owned())
        .collect()
}

fn role_refs_by_user(
    roles: Vec<rs_admin_store::users::UserRoleRef>,
) -> HashMap<Uuid, Vec<RoleRef>> {
    let mut by_user: HashMap<Uuid, Vec<RoleRef>> = HashMap::new();
    for role in roles {
        by_user.entry(role.user_id).or_default().push(RoleRef {
            id: role.role_id.to_string(),
            name: role.name,
            is_builtin: role.is_builtin,
        });
    }
    by_user
}

fn summarize_user(row: &UserRow, roles: &[RoleRef]) -> UserSummary {
    UserSummary {
        id: row.id.to_string(),
        username: row.username.clone(),
        email: row.email.clone(),
        display_name: row.display_name.clone(),
        phone: row.phone.clone(),
        status: row.status.clone(),
        roles: roles.to_vec(),
        must_change_password: row.must_change_password,
        locked_until: row.locked_until.map(crate::wire::format_timestamp),
        last_login_at: row.last_login_at.map(crate::wire::format_timestamp),
        created_at: crate::wire::format_timestamp(row.created_at),
        updated_at: crate::wire::format_timestamp(row.updated_at),
    }
}

fn detail_user(row: &UserRow, roles: &[RoleRef]) -> UserDetail {
    let summary = summarize_user(row, roles);
    UserDetail {
        temporary_password_expires_at: row
            .temporary_password_expires_at
            .map(crate::wire::format_timestamp),
        ..summary.into()
    }
}

fn user_target(row: &UserRow) -> DenialTarget<'_> {
    DenialTarget::user(row.id, Some(row.username.as_str()))
}

/// Audit target for a user addressed by path, even before it is loaded: the id
/// is known, the username snapshot is not.
fn requested_user_target(raw: &str) -> Option<DenialTarget<'static>> {
    Uuid::parse_str(raw)
        .ok()
        .map(|id| DenialTarget::user(id, None))
}

fn role_ref(role: &UserRole) -> RoleRef {
    RoleRef {
        id: role.id.to_string(),
        name: role.name.clone(),
        is_builtin: role.is_builtin,
    }
}

/// Role references in the same order everywhere: built-in roles first, then
/// by name.
fn role_refs(roles: &[UserRole]) -> Vec<RoleRef> {
    let mut refs: Vec<RoleRef> = roles.iter().map(role_ref).collect();
    refs.sort_by(|left, right| {
        right
            .is_builtin
            .cmp(&left.is_builtin)
            .then_with(|| left.name.cmp(&right.name))
    });
    refs
}

/// A role held by a target user, resolved against the code catalog.
enum ScopeRole {
    BuiltIn(Option<BuiltInRole>),
    Custom(Vec<PermissionKey>),
    Unresolved,
}

/// Refuse a role assignment that exceeds the operator's grant capabilities,
/// recording the first offending role in the audit chain (spec §6.3 rule 3).
async fn ensure_roles_assignable(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    roles: &[RoleRow],
) -> Result<(), ApiError> {
    let Some(role) = roles
        .iter()
        .find(|role| !role_is_assignable(role, &principal.effective))
    else {
        return Ok(());
    };
    write_denial(
        state,
        services,
        principal,
        context,
        Denial::authorization(
            "create_user",
            ErrorCode::PermissionSubsetViolation,
            Some(DenialTarget::role(role.id, Some(role.name.as_str()))),
        ),
    )
    .await?;
    Err(ApiError::new(ErrorCode::PermissionSubsetViolation))
}

/// Whether a non-super-admin may read and manage the target (spec §6.3 rule
/// 4). Callers apply the `self` exception first.
fn user_within_scope(effective: &EffectivePermissions, roles: &[UserRole]) -> bool {
    let parsed: Vec<ScopeRole> = roles
        .iter()
        .map(|role| {
            if role.is_builtin {
                return ScopeRole::BuiltIn(BuiltInRole::from_code(&role.name));
            }
            let mut keys = Vec::with_capacity(role.permission_keys.len());
            for key in &role.permission_keys {
                match key.parse::<PermissionKey>() {
                    Ok(key) => keys.push(key),
                    Err(_) => return ScopeRole::Unresolved,
                }
            }
            ScopeRole::Custom(keys)
        })
        .collect();

    effective.can_manage_user(parsed.iter().map(|role| match role {
        ScopeRole::BuiltIn(Some(builtin)) => RoleTarget::BuiltIn(*builtin),
        ScopeRole::BuiltIn(None) | ScopeRole::Unresolved => RoleTarget::Unresolved,
        ScopeRole::Custom(keys) => RoleTarget::Custom(keys.as_slice()),
    }))
}

/// Map an identity value-object error to a field-level validation failure.
fn identity_error(pointer: &str, error: IdentityError) -> ApiError {
    let code = match error {
        IdentityError::UsernameLength | IdentityError::DisplayNameLength => "invalid_length",
        IdentityError::UsernameFormat
        | IdentityError::UsernameReserved
        | IdentityError::EmailTooLong
        | IdentityError::EmailFormat
        | IdentityError::PhoneLength => "invalid_format",
    };
    validation_error(pointer, code)
}

fn policy_error(violation: PolicyViolation) -> ApiError {
    ApiError::new(ErrorCode::PasswordPolicyViolation).with_errors(vec![FieldError {
        pointer: "/password".to_owned(),
        code: ErrorCode::PasswordPolicyViolation.as_str().to_owned(),
        params: Some(serde_json::json!({ "reason": violation.as_reason() })),
    }])
}

fn unknown_role_error() -> ApiError {
    ApiError::new(ErrorCode::UnknownRole).with_errors(vec![FieldError {
        pointer: "/role_ids".to_owned(),
        code: ErrorCode::UnknownRole.as_str().to_owned(),
        params: None,
    }])
}

/// Parse `role_ids` in request order, rejecting duplicates and malformed ids.
fn parse_role_ids(raw: &[String]) -> Result<Vec<Uuid>, ApiError> {
    let mut ids = Vec::with_capacity(raw.len());
    for (index, value) in raw.iter().enumerate() {
        let id = Uuid::parse_str(value.trim())
            .map_err(|_| validation_error(&format!("/role_ids/{index}"), "invalid"))?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn map_user_store_error(error: UserStoreError) -> ApiError {
    match error {
        UserStoreError::IdentifierTaken(conflict) => {
            let pointer = match conflict {
                rs_admin_store::identity::IdentifierConflict::Username => "/username",
                rs_admin_store::identity::IdentifierConflict::Email => "/email",
            };
            ApiError::new(ErrorCode::IdentifierTaken).with_errors(vec![FieldError {
                pointer: pointer.to_owned(),
                code: ErrorCode::IdentifierTaken.as_str().to_owned(),
                params: None,
            }])
        }
        error @ UserStoreError::Database(_) => store_unavailable(error),
    }
}

fn temporary_password_generation_failed(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "temporary password generation failed");
    ApiError::new(ErrorCode::InternalError)
}

struct ListParams {
    q: Option<String>,
    status: Option<String>,
    role_id: Option<Uuid>,
    is_locked: Option<bool>,
    must_change_password: Option<bool>,
    sort: UserSort,
    page: i64,
    page_size: i64,
}

fn parse_list_query(query: ListUsersQuery) -> Result<ListParams, ApiError> {
    let (page, page_size) = parse_pagination(query.page.as_deref(), query.page_size.as_deref())?;

    let status = match query.status.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(status @ ("active" | "disabled")) => Some(status.to_owned()),
        Some(_) => return Err(validation_error("/status", "invalid")),
    };
    let role_id = match query.role_id.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => {
            Some(Uuid::parse_str(raw).map_err(|_| validation_error("/role_id", "invalid"))?)
        }
    };
    let is_locked = parse_boolean(query.is_locked.as_deref(), "/is_locked")?;
    let must_change_password = parse_boolean(
        query.must_change_password.as_deref(),
        "/must_change_password",
    )?;
    let sort = parse_sort(
        query
            .sort
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    )?;

    Ok(ListParams {
        q: query
            .q
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty()),
        status,
        role_id,
        is_locked,
        must_change_password,
        sort,
        page,
        page_size,
    })
}

fn parse_boolean(raw: Option<&str>, pointer: &'static str) -> Result<Option<bool>, ApiError> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(_) => Err(validation_error(pointer, "invalid")),
    }
}

fn parse_sort(raw: Option<&str>) -> Result<UserSort, ApiError> {
    let Some(raw) = raw else {
        return Ok(UserSort::default());
    };
    let (ascending, field) = match raw.strip_prefix('-') {
        Some(field) => (false, field),
        None => (true, raw),
    };
    let field = match field {
        "username" => UserSortField::Username,
        "email" => UserSortField::Email,
        "display_name" => UserSortField::DisplayName,
        "status" => UserSortField::Status,
        "created_at" => UserSortField::CreatedAt,
        "last_login_at" => UserSortField::LastLoginAt,
        "updated_at" => UserSortField::UpdatedAt,
        _ => return Err(ApiError::new(ErrorCode::InvalidSort)),
    };
    Ok(UserSort { field, ascending })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "Users",
    operation_id = "listUsers",
    security(("bearerAuth" = [])),
    params(ListUsersQuery),
    responses(
        (
            status = 200,
            description = "Users page filtered by the operator's management scope",
            body = UserListResponse,
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Missing users.read",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Invalid filter, pagination or sort",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "User store unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn list_users(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<UserListResponse>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal.0,
        &context,
        PermissionKey::UsersRead,
        "list_users",
        None,
    )
    .await?;

    let params = parse_list_query(query)?;
    let allowed_keys = allowed_permission_keys(&principal);
    let filter = UserFilter {
        q: params.q.as_deref(),
        status: params.status.as_deref(),
        role_id: params.role_id,
        is_locked: params.is_locked,
        must_change_password: params.must_change_password,
        sort: params.sort,
        limit: params.page_size,
        offset: (params.page - 1) * params.page_size,
        scope: user_scope(&principal, &allowed_keys),
    };
    let page = store_list_users(state.database.pool(), filter)
        .await
        .map_err(store_unavailable)?;

    let ids: Vec<Uuid> = page.items.iter().map(|row| row.id).collect();
    let roles = load_user_role_refs(state.database.pool(), &ids)
        .await
        .map_err(store_unavailable)?;
    let mut roles_by_user = role_refs_by_user(roles);

    Ok(Json(UserListResponse {
        items: page
            .items
            .iter()
            .map(|row| {
                let roles = roles_by_user.remove(&row.id).unwrap_or_default();
                summarize_user(row, &roles)
            })
            .collect(),
        total: page.total,
        page: params.page,
        page_size: params.page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/users/{id}",
    tag = "Users",
    operation_id = "getUser",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "User id (UUID)")),
    responses(
        (
            status = 200,
            description = "User detail",
            body = UserDetail,
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Missing users.read or the target is out of scope",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 404,
            description = "User does not exist",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "User store unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn get_user(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
) -> Result<Json<UserDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal.0,
        &context,
        PermissionKey::UsersRead,
        "get_user",
        requested_user_target(&id),
    )
    .await?;

    let id = parse_id(&id)?;
    let row = find_user(state.database.pool(), id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;

    let roles = load_role_grants(state.database.pool(), row.id)
        .await
        .map_err(store_unavailable)?;
    // `self` is always readable; every other target must be inside the
    // operator's management scope (spec §6.3 rule 4, §6.6).
    if row.id != principal.0.user.id && !user_within_scope(&principal.0.effective, &roles) {
        write_denial(
            &state,
            services,
            &principal.0,
            &context,
            Denial::authorization("get_user", ErrorCode::OutOfScope, Some(user_target(&row))),
        )
        .await?;
        return Err(ApiError::new(ErrorCode::OutOfScope));
    }

    Ok(Json(detail_user(&row, &role_refs(&roles))))
}

#[utoipa::path(
    post,
    path = "/api/v1/users",
    tag = "Users",
    operation_id = "createUser",
    security(("bearerAuth" = [])),
    request_body = CreateUserRequest,
    responses(
        (
            status = 201,
            description = "User created; the temporary password is returned exactly once",
            body = UserCreatedResponse,
            headers(
                ("Location" = String, description = "URL of the created user"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Missing users.create/users.assign_role, grant-subset violation or stale authentication",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 409,
            description = "Username or email already in use",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Invalid field, password policy violation or unknown role",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "User store or password hashing unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    ApiJson(payload): ApiJson<CreateUserRequest>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::UsersCreate,
        "create_user",
        None,
    )
    .await?;

    let requested_role_ids = match payload.role_ids.as_deref() {
        Some(raw) => {
            require_permission(
                &state,
                services,
                &principal,
                &context,
                PermissionKey::UsersAssignRole,
                "create_user",
                None,
            )
            .await?;
            parse_role_ids(raw)?
        }
        None => Vec::new(),
    };

    // Grant-subset and role existence are target eligibility rules, checked
    // before step-up so a stale session never sees a reauthentication prompt
    // for a request it could not perform anyway (spec §5.4).
    if !requested_role_ids.is_empty() {
        let roles = find_roles_by_ids(state.database.pool(), &requested_role_ids)
            .await
            .map_err(store_unavailable)?;
        if roles.len() != requested_role_ids.len() {
            return Err(unknown_role_error());
        }
        ensure_roles_assignable(&state, services, &principal, &context, &roles).await?;
    }

    require_recent_auth(&state, services, &principal, &context, "create_user", None).await?;

    let username =
        Username::parse(&payload.username).map_err(|error| identity_error("/username", error))?;
    let email = Email::parse(&payload.email).map_err(|error| identity_error("/email", error))?;
    let display_name = match payload.display_name.as_deref() {
        Some(raw) => {
            DisplayName::parse(raw).map_err(|error| identity_error("/display_name", error))?
        }
        None => DisplayName::from_username(&username),
    };
    let phone = payload
        .phone
        .as_deref()
        .map(Phone::parse)
        .transpose()
        .map_err(|error| identity_error("/phone", error))?;

    let temporary_password = match payload.password.as_deref() {
        Some(manual) => {
            PasswordPolicy::new()
                .validate(
                    manual,
                    &[username.as_str(), email.as_str(), display_name.as_str()],
                )
                .map_err(policy_error)?;
            manual.to_owned()
        }
        None => generate_temporary_password().map_err(temporary_password_generation_failed)?,
    };
    let password_hash = services
        .passwords()
        .hash(&temporary_password)
        .await
        .map_err(password_unavailable)?;

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(store_unavailable)?;

    // Re-check under a share lock: a concurrent soft delete must not leave the
    // new user holding a role that is no longer assignable.
    if !requested_role_ids.is_empty() {
        let roles = lock_roles_by_ids(&mut transaction, &requested_role_ids)
            .await
            .map_err(store_unavailable)?;
        if roles.len() != requested_role_ids.len() {
            transaction.rollback().await.ok();
            return Err(unknown_role_error());
        }
        if roles
            .iter()
            .any(|role| !role_is_assignable(role, &principal.effective))
        {
            transaction.rollback().await.ok();
            ensure_roles_assignable(&state, services, &principal, &context, &roles).await?;
            return Err(ApiError::new(ErrorCode::PermissionSubsetViolation));
        }
    }

    let (user_id, expires_at) = insert_user(
        &mut transaction,
        NewUserRow {
            username: username.as_str(),
            email: email.as_str(),
            display_name: display_name.as_str(),
            phone: phone.as_ref().map(Phone::as_str),
            password_hash: &password_hash,
            temporary_password_ttl: services.temporary_password_ttl(),
        },
    )
    .await
    .map_err(map_user_store_error)?;

    assign_baseline_role(&mut transaction, user_id, Some(principal.user.id))
        .await
        .map_err(store_unavailable)?;
    assign_roles(
        &mut transaction,
        user_id,
        &requested_role_ids,
        Some(principal.user.id),
    )
    .await
    .map_err(store_unavailable)?;

    let roles = load_role_grants(&mut *transaction, user_id)
        .await
        .map_err(store_unavailable)?;
    let refs = role_refs(&roles);
    let audited_role_ids: Vec<String> = roles.iter().map(|role| role.id.to_string()).collect();
    let row = find_user_in(&mut transaction, user_id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;

    let expires_at_wire = crate::wire::format_timestamp(expires_at);
    let event = AuditEvent::builder(
        AuditAction::UserCreated,
        AuditResult::Success,
        AuditActor::user(principal.user.id, principal.user.username.clone()),
        context.request_id,
    )
    .target(TargetType::User, Some(user_id), Some(username.to_string()))
    .detail("username", username.as_str())
    .detail("email", email.as_str())
    .detail("role_ids", audited_role_ids)
    .detail("must_change_password", true)
    .detail("temporary_password_expires_at", expires_at_wire.clone())
    .ip(context.client_ip);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(store_unavailable)?;

    let location = HeaderValue::from_str(&format!("/api/v1/users/{user_id}"))
        .map_err(|_| ApiError::new(ErrorCode::InternalError))?;
    Ok((
        StatusCode::CREATED,
        [(LOCATION, location)],
        Json(UserCreatedResponse {
            user: detail_user(&row, &refs),
            temporary_password,
            temporary_password_expires_at: expires_at_wire,
        }),
    )
        .into_response())
}

#[utoipa::path(
    patch,
    path = "/api/v1/users/{id}",
    tag = "Users",
    operation_id = "updateUser",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "User id (UUID)")),
    request_body = UpdateUserRequest,
    responses(
        (
            status = 200,
            description = "User profile updated",
            body = UserDetail,
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Missing users.update, out of scope, self action, or stale authentication for an identifier change",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 404,
            description = "User does not exist",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 409,
            description = "Username or email already in use",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Empty update or invalid field",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "User store unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn update_user(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<UpdateUserRequest>,
) -> Result<Json<UserDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::UsersUpdate,
        "update_user",
        requested_user_target(&id),
    )
    .await?;

    let id = parse_id(&id)?;
    let existing = find_user(state.database.pool(), id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;

    // Management endpoints never act on the operator's own account; profile
    // self-service goes through `/auth/me` (spec §6.3 rule 5).
    if existing.id == principal.user.id {
        write_denial(
            &state,
            services,
            &principal,
            &context,
            Denial::authorization(
                "update_user",
                ErrorCode::SelfActionForbidden,
                Some(user_target(&existing)),
            ),
        )
        .await?;
        return Err(ApiError::new(ErrorCode::SelfActionForbidden));
    }

    let roles = load_role_grants(state.database.pool(), id)
        .await
        .map_err(store_unavailable)?;
    if !user_within_scope(&principal.effective, &roles) {
        write_denial(
            &state,
            services,
            &principal,
            &context,
            Denial::authorization(
                "update_user",
                ErrorCode::OutOfScope,
                Some(user_target(&existing)),
            ),
        )
        .await?;
        return Err(ApiError::new(ErrorCode::OutOfScope));
    }

    if payload.display_name.is_none()
        && payload.username.is_none()
        && payload.email.is_none()
        && payload.phone.is_none()
    {
        return Err(ApiError::new(ErrorCode::EmptyUpdate));
    }

    // Only identifier changes are sensitive enough to require a fresh
    // password authentication (spec §5.8).
    if payload.username.is_some() || payload.email.is_some() {
        require_recent_auth(
            &state,
            services,
            &principal,
            &context,
            "update_user",
            Some(user_target(&existing)),
        )
        .await?;
    }

    let username = payload
        .username
        .as_deref()
        .map(Username::parse)
        .transpose()
        .map_err(|error| identity_error("/username", error))?;
    let email = payload
        .email
        .as_deref()
        .map(Email::parse)
        .transpose()
        .map_err(|error| identity_error("/email", error))?;
    let display_name = payload
        .display_name
        .as_deref()
        .map(DisplayName::parse)
        .transpose()
        .map_err(|error| identity_error("/display_name", error))?;
    let phone = match payload.phone.as_ref() {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(Some(
            Phone::parse(raw).map_err(|error| identity_error("/phone", error))?,
        )),
    };

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(store_unavailable)?;

    let locked = lock_user_row(&mut transaction, id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;

    // A concurrent role change may have moved the target out of scope.
    let locked_roles = load_role_grants(&mut *transaction, id)
        .await
        .map_err(store_unavailable)?;
    if !user_within_scope(&principal.effective, &locked_roles) {
        transaction.rollback().await.ok();
        write_denial(
            &state,
            services,
            &principal,
            &context,
            Denial::authorization(
                "update_user",
                ErrorCode::OutOfScope,
                Some(DenialTarget::user(
                    locked.id,
                    Some(locked.username.as_str()),
                )),
            ),
        )
        .await?;
        return Err(ApiError::new(ErrorCode::OutOfScope));
    }

    // `changed_fields` records old/new for the login identifiers and just the
    // field name (as `true`) for the other profile fields (spec §8.5.2).
    let mut changed_fields = serde_json::Map::new();
    if let Some(new_username) = username.as_ref()
        && new_username.as_str() != locked.username
    {
        changed_fields.insert(
            "username".to_owned(),
            serde_json::json!({ "old": locked.username, "new": new_username.as_str() }),
        );
    }
    if let Some(new_email) = email.as_ref()
        && new_email.as_str() != locked.email
    {
        changed_fields.insert(
            "email".to_owned(),
            serde_json::json!({ "old": locked.email, "new": new_email.as_str() }),
        );
    }
    if let Some(new_display_name) = display_name.as_ref()
        && new_display_name.as_str() != locked.display_name
    {
        changed_fields.insert("display_name".to_owned(), serde_json::Value::Bool(true));
    }
    let phone_changed = match &phone {
        None => false,
        Some(None) => locked.phone.is_some(),
        Some(Some(new_phone)) => locked.phone.as_deref() != Some(new_phone.as_str()),
    };
    if phone_changed {
        changed_fields.insert("phone".to_owned(), serde_json::Value::Bool(true));
    }

    if changed_fields.is_empty() {
        transaction.rollback().await.ok();
        return Ok(Json(detail_user(&existing, &role_refs(&locked_roles))));
    }

    store_update_user(
        &mut transaction,
        id,
        username.as_ref().map(Username::as_str),
        email.as_ref().map(Email::as_str),
        display_name.as_ref().map(DisplayName::as_str),
        phone
            .as_ref()
            .map(|value| value.as_ref().map(Phone::as_str)),
    )
    .await
    .map_err(map_user_store_error)?;

    let row = find_user_in(&mut transaction, id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;
    let roles = load_role_grants(&mut *transaction, id)
        .await
        .map_err(store_unavailable)?;

    let target_label = username
        .as_ref()
        .map(Username::to_string)
        .unwrap_or_else(|| locked.username.clone());
    let builder = AuditEvent::builder(
        AuditAction::UserUpdated,
        AuditResult::Success,
        AuditActor::user(principal.user.id, principal.user.username.clone()),
        context.request_id,
    )
    .target(TargetType::User, Some(id), Some(target_label))
    .detail("changed_fields", serde_json::Value::Object(changed_fields))
    .ip(context.client_ip);
    write_audit_event(
        &mut transaction,
        services,
        builder,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(store_unavailable)?;
    Ok(Json(detail_user(&row, &role_refs(&roles))))
}
