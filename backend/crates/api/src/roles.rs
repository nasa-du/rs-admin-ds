//! Roles and permissions endpoints (docs/specs/login-system.md §6.3–§6.5,
//! §7.8).
//!
//! Reads expose every active role regardless of the operator's management
//! scope and express eligibility through `assignable` / `manageable`. Writes
//! are limited to custom roles, enforce the grant-subset rule, require a recent
//! password authentication (reauthentication) and append one audit event per change in
//! the same transaction as the change itself.

use crate::auth::{AuthServices, Authenticated, SessionPrincipal, write_audit_event};
use crate::authz::{
    Denial, DenialTarget, RequestContext, require_permission, require_recent_auth, write_denial,
};
use crate::client_ip::ClientIp;
use crate::error::{ApiError, FieldError};
use crate::json::ApiJson;
use crate::middleware::RequestId;
use crate::query::{parse_id, parse_pagination, validation_error};
use crate::router::AppState;
use crate::wire::format_timestamp;
use axum::Extension;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::header::LOCATION;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, TargetType};
use rs_admin_domain::ErrorCode;
use rs_admin_domain::permission::{
    PERMISSION_CATALOG, PermissionGroup as CatalogGroup, PermissionKey,
};
use rs_admin_domain::rbac::{BuiltInRole, EffectivePermissions, RoleTarget};
use rs_admin_store::roles::{
    RoleFilter, RoleRow, RoleSort, RoleSortField, count_assignments, find_role, find_role_in,
    insert_role, list_roles as store_list_roles, lock_role, replace_permissions, soft_delete_role,
    update_role,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const MAX_ROLE_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 500;

// ---------------------------------------------------------------------------
// Request and response types
// ---------------------------------------------------------------------------

/// Query parameters for `GET /roles`. Unknown parameters are ignored.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRolesQuery {
    /// Case-insensitive substring search over name and description.
    pub q: Option<String>,
    /// Restrict to built-in (`true`) or custom (`false`) roles.
    #[param(value_type = Option<bool>)]
    pub is_builtin: Option<String>,
    /// One-based page number; defaults to 1.
    #[param(value_type = Option<i64>)]
    pub page: Option<String>,
    /// Page size; defaults to 20 and may not exceed 200.
    #[param(value_type = Option<i64>)]
    pub page_size: Option<String>,
    /// `name`, `-name`, `created_at`, `-created_at`, `updated_at` or
    /// `-updated_at`; defaults to `name`.
    pub sort: Option<String>,
}

/// A role as returned by the list endpoint.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RoleSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_builtin: bool,
    pub permission_keys: Vec<String>,
    pub assignable: bool,
    pub manageable: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// A single role.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RoleDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_builtin: bool,
    pub permission_keys: Vec<String>,
    pub assignable: bool,
    pub manageable: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<RoleSummary> for RoleDetail {
    fn from(summary: RoleSummary) -> Self {
        Self {
            id: summary.id,
            name: summary.name,
            description: summary.description,
            is_builtin: summary.is_builtin,
            permission_keys: summary.permission_keys,
            assignable: summary.assignable,
            manageable: summary.manageable,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

/// Offset-paginated role list envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct RoleListResponse {
    pub items: Vec<RoleSummary>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// One permission in the catalog.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PermissionItem {
    pub key: String,
    pub display_name_zh: String,
    pub display_name_en: String,
    pub grantable: bool,
}

/// A permission group in catalog order.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PermissionGroup {
    pub key: String,
    pub display_name_zh: String,
    pub display_name_en: String,
    pub permissions: Vec<PermissionItem>,
}

/// `GET /permissions` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PermissionListResponse {
    pub groups: Vec<PermissionGroup>,
}

/// `POST /roles` request body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRoleRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub permission_keys: Vec<String>,
}

/// `PATCH /roles/{id}` request body. Omitted fields are unchanged; an explicit
/// `null` description clears it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateRoleRequest {
    /// Omitted means unchanged; `null` is rejected because names are not
    /// nullable.
    #[serde(default, deserialize_with = "crate::json::non_null_role_name")]
    #[schema(nullable = false)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "crate::json::double_option")]
    pub description: Option<Option<String>>,
}

/// `PUT /roles/{id}/permissions` request body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetRolePermissionsRequest {
    pub permission_keys: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn role_store_unavailable(error: impl std::fmt::Display) -> ApiError {
    tracing::warn!(error = %error, "role store operation failed");
    ApiError::service_unavailable()
}

fn known_permission_keys(row: &RoleRow) -> Vec<PermissionKey> {
    row.permission_keys
        .iter()
        .filter_map(|key| key.parse().ok())
        .collect()
}

/// Whether a role's stored permission set is within the operator's effective
/// permissions. A key removed from a later catalog is persisted for history
/// but grants nothing, so a role holding one is never a subset (spec §4.4).
fn role_within_permissions(row: &RoleRow, effective: &EffectivePermissions) -> bool {
    if effective.is_super_admin() {
        return true;
    }
    let known = known_permission_keys(row);
    row.permission_keys.len() == known.len() && effective.can_grant_keys(known)
}

/// Whether the operator may grant this role to a user (spec §6.4). Built-in
/// names outside the code catalog and custom roles with unresolved permission
/// keys are conservatively ineligible.
pub(crate) fn role_is_assignable(row: &RoleRow, effective: &EffectivePermissions) -> bool {
    if row.is_builtin {
        BuiltInRole::from_code(&row.name)
            .is_some_and(|builtin| effective.can_assign(RoleTarget::BuiltIn(builtin)))
    } else {
        role_within_permissions(row, effective)
    }
}

/// Eligibility of one role for the current operator (spec §6.4). Built-in
/// names outside the code catalog are conservatively ineligible.
fn role_eligibility(row: &RoleRow, effective: &EffectivePermissions) -> (bool, bool) {
    if row.is_builtin {
        let Some(builtin) = BuiltInRole::from_code(&row.name) else {
            return (false, false);
        };
        let target = RoleTarget::BuiltIn(builtin);
        return (effective.can_assign(target), effective.can_manage(target));
    }
    let within = role_within_permissions(row, effective);
    (within, within)
}

fn summarize_role(row: &RoleRow, effective: &EffectivePermissions) -> RoleSummary {
    let (assignable, manageable) = role_eligibility(row, effective);
    RoleSummary {
        id: row.id.to_string(),
        name: row.name.clone(),
        description: row.description.clone(),
        is_builtin: row.is_builtin,
        permission_keys: row.permission_keys.clone(),
        assignable,
        manageable,
        created_at: format_timestamp(row.created_at),
        updated_at: format_timestamp(row.updated_at),
    }
}

fn detail_role(row: &RoleRow, effective: &EffectivePermissions) -> RoleDetail {
    summarize_role(row, effective).into()
}

fn role_target(row: &RoleRow) -> DenialTarget<'_> {
    DenialTarget::role(row.id, Some(row.name.as_str()))
}

/// Audit target for a role addressed by path, even before it is loaded: the
/// id is known, the name snapshot is not.
fn requested_role_target(raw: &str) -> Option<DenialTarget<'static>> {
    Uuid::parse_str(raw)
        .ok()
        .map(|id| DenialTarget::role(id, None))
}

fn normalize_role_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(validation_error("/name", "required"));
    }
    if name.chars().count() > MAX_ROLE_NAME_CHARS {
        return Err(validation_error("/name", "too_long"));
    }
    Ok(name.to_owned())
}

fn validate_description(description: &str) -> Result<(), ApiError> {
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(validation_error("/description", "too_long"));
    }
    Ok(())
}

/// Parse a requested permission set. The catalog is closed, so any unknown key
/// rejects the request (spec §7.8).
fn permission_set(raw: &[String]) -> Result<BTreeSet<PermissionKey>, ApiError> {
    if raw.is_empty() {
        return Err(ApiError::new(ErrorCode::EmptyPermissions));
    }

    let mut keys = BTreeSet::new();
    for key in raw {
        let parsed = key.parse::<PermissionKey>().map_err(|_| {
            ApiError::new(ErrorCode::InvalidPermission).with_errors(vec![FieldError {
                pointer: "/permission_keys".to_owned(),
                code: "invalid_permission".to_owned(),
                params: Some(serde_json::json!({ "key": key })),
            }])
        })?;
        keys.insert(parsed);
    }
    Ok(keys)
}

/// Permission keys in the same lexicographic order the role queries return,
/// so audit snapshots and API responses sort identically.
fn sorted_strings(keys: &BTreeSet<PermissionKey>) -> Vec<String> {
    let mut keys: Vec<String> = keys.iter().map(|key| key.as_str().to_owned()).collect();
    keys.sort();
    keys
}

struct ListParams {
    q: Option<String>,
    is_builtin: Option<bool>,
    sort: RoleSort,
    page: i64,
    page_size: i64,
}

fn parse_list_query(query: ListRolesQuery) -> Result<ListParams, ApiError> {
    let (page, page_size) = parse_pagination(query.page.as_deref(), query.page_size.as_deref())?;

    let is_builtin = match query.is_builtin.as_deref().map(str::trim) {
        None | Some("") => None,
        Some("true") => Some(true),
        Some("false") => Some(false),
        Some(_) => return Err(validation_error("/is_builtin", "invalid")),
    };

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
        is_builtin,
        sort,
        page,
        page_size,
    })
}

fn parse_sort(raw: Option<&str>) -> Result<RoleSort, ApiError> {
    let Some(raw) = raw else {
        return Ok(RoleSort::default());
    };
    let (ascending, field) = match raw.strip_prefix('-') {
        Some(field) => (false, field),
        None => (true, raw),
    };
    let field = match field {
        "name" => RoleSortField::Name,
        "created_at" => RoleSortField::CreatedAt,
        "updated_at" => RoleSortField::UpdatedAt,
        _ => return Err(ApiError::new(ErrorCode::InvalidSort)),
    };
    Ok(RoleSort { field, ascending })
}

/// Target-level denial for built-in roles and grant-subset violations.
async fn deny_role(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    operation: &'static str,
    reason: ErrorCode,
    role: &RoleRow,
) -> ApiError {
    if let Err(error) = write_denial(
        state,
        services,
        principal,
        context,
        Denial::authorization(operation, reason, Some(role_target(role))),
    )
    .await
    {
        return error;
    }
    ApiError::new(reason)
}

/// Load an active role or return 404.
async fn load_role(state: &AppState, id: Uuid) -> Result<RoleRow, ApiError> {
    find_role(state.database.pool(), id)
        .await
        .map_err(role_store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))
}

/// Load a role for a write and refuse built-in targets.
async fn load_custom_role(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    id: Uuid,
    operation: &'static str,
) -> Result<RoleRow, ApiError> {
    let role = load_role(state, id).await?;
    if role.is_builtin {
        return Err(deny_role(
            state,
            services,
            principal,
            context,
            operation,
            ErrorCode::RoleImmutable,
            &role,
        )
        .await);
    }
    Ok(role)
}

/// Grant-subset rule for a role's existing permission set.
async fn ensure_role_within_permissions(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    operation: &'static str,
    role: &RoleRow,
) -> Result<(), ApiError> {
    if role_within_permissions(role, &principal.effective) {
        return Ok(());
    }
    Err(deny_role(
        state,
        services,
        principal,
        context,
        operation,
        ErrorCode::PermissionSubsetViolation,
        role,
    )
    .await)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/roles",
    tag = "Roles",
    operation_id = "listRoles",
    security(("bearerAuth" = [])),
    params(ListRolesQuery),
    responses(
        (
            status = 200,
            description = "Roles page with target eligibility flags",
            body = RoleListResponse,
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
            description = "Missing roles.read",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Invalid pagination or sort",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Role store unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn list_roles(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Query(query): Query<ListRolesQuery>,
) -> Result<Json<RoleListResponse>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::RolesRead,
        "list_roles",
        None,
    )
    .await?;

    let params = parse_list_query(query)?;
    let filter = RoleFilter {
        q: params.q.as_deref(),
        is_builtin: params.is_builtin,
        sort: params.sort,
        limit: params.page_size,
        offset: (params.page - 1) * params.page_size,
    };
    let page = store_list_roles(state.database.pool(), filter)
        .await
        .map_err(role_store_unavailable)?;

    Ok(Json(RoleListResponse {
        items: page
            .items
            .iter()
            .map(|row| summarize_role(row, &principal.effective))
            .collect(),
        total: page.total,
        page: params.page,
        page_size: params.page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/roles/{id}",
    tag = "Roles",
    operation_id = "getRole",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "Role id (UUID)")),
    responses(
        (
            status = 200,
            description = "Role detail",
            body = RoleDetail,
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
            description = "Missing roles.read",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 404,
            description = "Role does not exist or was soft-deleted",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Role store unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn get_role(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
) -> Result<Json<RoleDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::RolesRead,
        "get_role",
        requested_role_target(&id),
    )
    .await?;

    let id = parse_id(&id)?;
    let role = load_role(&state, id).await?;
    Ok(Json(detail_role(&role, &principal.effective)))
}

#[utoipa::path(
    get,
    path = "/api/v1/permissions",
    tag = "Permissions",
    operation_id = "listPermissions",
    security(("bearerAuth" = [])),
    responses(
        (
            status = 200,
            description = "Full permission catalog grouped in code order",
            body = PermissionListResponse,
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
            description = "Holds none of roles.read, roles.create, roles.update",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn list_permissions(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
) -> Result<Json<PermissionListResponse>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    let allowed = [
        PermissionKey::RolesRead,
        PermissionKey::RolesCreate,
        PermissionKey::RolesUpdate,
    ]
    .into_iter()
    .any(|permission| principal.effective.contains(permission));
    if !allowed {
        write_denial(
            &state,
            services,
            &principal,
            &context,
            Denial::authorization("list_permissions", ErrorCode::Forbidden, None),
        )
        .await?;
        return Err(ApiError::new(ErrorCode::Forbidden));
    }

    let groups = CatalogGroup::ALL
        .iter()
        .map(|group| PermissionGroup {
            key: group.key().to_owned(),
            display_name_zh: group.display_name_zh().to_owned(),
            display_name_en: group.display_name_en().to_owned(),
            permissions: PERMISSION_CATALOG
                .iter()
                .filter(|definition| definition.group == *group)
                .map(|definition| PermissionItem {
                    key: definition.key.as_str().to_owned(),
                    display_name_zh: definition.display_name_zh.to_owned(),
                    display_name_en: definition.display_name_en.to_owned(),
                    grantable: principal.effective.contains(definition.key),
                })
                .collect(),
        })
        .collect();

    Ok(Json(PermissionListResponse { groups }))
}

#[utoipa::path(
    post,
    path = "/api/v1/roles",
    tag = "Roles",
    operation_id = "createRole",
    security(("bearerAuth" = [])),
    request_body = CreateRoleRequest,
    responses(
        (
            status = 201,
            description = "Custom role created",
            body = RoleDetail,
            headers(
                ("Location" = String, description = "URL of the created role"),
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
            description = "Missing roles.create, grant-subset violation or stale authentication",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 409,
            description = "Name already in use",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Invalid name, empty permissions or unknown permission",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Role store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn create_role(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    ApiJson(payload): ApiJson<CreateRoleRequest>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::RolesCreate,
        "create_role",
        None,
    )
    .await?;

    // Grant-subset rule first (target eligibility), then reauthentication, then payload
    // validation, matching the per-request pipeline order (spec §5.4).
    let requested = permission_set(&payload.permission_keys);
    if let Ok(keys) = &requested
        && !principal.effective.can_grant(keys)
    {
        write_denial(
            &state,
            services,
            &principal,
            &context,
            Denial::authorization("create_role", ErrorCode::PermissionSubsetViolation, None),
        )
        .await?;
        return Err(ApiError::new(ErrorCode::PermissionSubsetViolation));
    }

    require_recent_auth(&state, services, &principal, &context, "create_role", None).await?;

    let name = normalize_role_name(&payload.name)?;
    if let Some(description) = payload.description.as_deref() {
        validate_description(description)?;
    }
    let keys = requested?;
    let permission_keys = sorted_strings(&keys);

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(role_store_unavailable)?;

    let role_id = insert_role(&mut transaction, &name, payload.description.as_deref())
        .await
        .map_err(|error| match error {
            rs_admin_store::roles::RoleStoreError::NameTaken => {
                ApiError::new(ErrorCode::RoleNameTaken)
            }
            other => role_store_unavailable(other),
        })?;

    replace_permissions(&mut transaction, role_id, &permission_keys)
        .await
        .map_err(role_store_unavailable)?;

    let role = find_role_in(&mut transaction, role_id)
        .await
        .map_err(role_store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;

    let event = AuditEvent::builder(
        AuditAction::RoleCreated,
        AuditResult::Success,
        AuditActor::user(principal.user.id, principal.user.username.clone()),
        context.request_id,
    )
    .target(TargetType::Role, Some(role.id), Some(role.name.clone()))
    .detail("role_name", role.name.clone())
    .detail("permission_keys", permission_keys)
    .ip(context.client_ip);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(role_store_unavailable)?;

    let detail = detail_role(&role, &principal.effective);
    let location = HeaderValue::from_str(&format!("/api/v1/roles/{}", role.id))
        .map_err(|_| ApiError::new(ErrorCode::InternalError))?;
    Ok((StatusCode::CREATED, [(LOCATION, location)], Json(detail)).into_response())
}

#[utoipa::path(
    patch,
    path = "/api/v1/roles/{id}",
    tag = "Roles",
    operation_id = "updateRole",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "Role id (UUID)")),
    request_body = UpdateRoleRequest,
    responses(
        (
            status = 200,
            description = "Role metadata updated",
            body = RoleDetail,
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
            description = "Missing roles.update, built-in role, grant-subset violation or stale authentication",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 404,
            description = "Role does not exist or was soft-deleted",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 409,
            description = "Name already in use",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Empty update or invalid fields",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Role store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn update_role_metadata(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<UpdateRoleRequest>,
) -> Result<Json<RoleDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::RolesUpdate,
        "update_role",
        requested_role_target(&id),
    )
    .await?;

    let id = parse_id(&id)?;
    let role = load_custom_role(&state, services, &principal, &context, id, "update_role").await?;
    ensure_role_within_permissions(&state, services, &principal, &context, "update_role", &role)
        .await?;

    let target = role_target(&role);
    require_recent_auth(
        &state,
        services,
        &principal,
        &context,
        "update_role",
        Some(target),
    )
    .await?;

    if payload.name.is_none() && payload.description.is_none() {
        return Err(ApiError::new(ErrorCode::EmptyUpdate));
    }
    let name = match payload.name.as_deref() {
        Some(raw) => Some(normalize_role_name(raw)?),
        None => None,
    };
    if let Some(description) = payload.description.as_ref().and_then(Option::as_deref) {
        validate_description(description)?;
    }

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(role_store_unavailable)?;

    let locked = lock_role(&mut transaction, id)
        .await
        .map_err(role_store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
    if locked.is_builtin {
        transaction
            .rollback()
            .await
            .map_err(role_store_unavailable)?;
        return Err(deny_role(
            &state,
            services,
            &principal,
            &context,
            "update_role",
            ErrorCode::RoleImmutable,
            &locked,
        )
        .await);
    }
    // Re-check under the lock: a concurrent permission change may have made
    // this role exceed the operator's effective permissions.
    if !role_within_permissions(&locked, &principal.effective) {
        transaction
            .rollback()
            .await
            .map_err(role_store_unavailable)?;
        return Err(deny_role(
            &state,
            services,
            &principal,
            &context,
            "update_role",
            ErrorCode::PermissionSubsetViolation,
            &locked,
        )
        .await);
    }

    let mut changed_fields = Vec::new();
    if let Some(new_name) = &name
        && *new_name != locked.name
    {
        changed_fields.push("name");
    }
    let description_changed = match &payload.description {
        Some(new_description) => new_description.as_deref() != locked.description.as_deref(),
        None => false,
    };
    if description_changed {
        changed_fields.push("description");
    }

    if !changed_fields.is_empty() {
        update_role(
            &mut transaction,
            id,
            name.as_deref(),
            payload.description.as_ref().map(|value| value.as_deref()),
        )
        .await
        .map_err(|error| match error {
            rs_admin_store::roles::RoleStoreError::NameTaken => {
                ApiError::new(ErrorCode::RoleNameTaken)
            }
            other => role_store_unavailable(other),
        })?;

        let updated = find_role_in(&mut transaction, id)
            .await
            .map_err(role_store_unavailable)?
            .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;

        let event = AuditEvent::builder(
            AuditAction::RoleUpdated,
            AuditResult::Success,
            AuditActor::user(principal.user.id, principal.user.username.clone()),
            context.request_id,
        )
        .target(
            TargetType::Role,
            Some(updated.id),
            Some(updated.name.clone()),
        )
        .detail("changed_fields", changed_fields)
        .ip(context.client_ip);
        write_audit_event(
            &mut transaction,
            services,
            event,
            context.user_agent.clone(),
        )
        .await?;

        transaction.commit().await.map_err(role_store_unavailable)?;
        return Ok(Json(detail_role(&updated, &principal.effective)));
    }

    transaction.commit().await.map_err(role_store_unavailable)?;
    Ok(Json(detail_role(&locked, &principal.effective)))
}

#[utoipa::path(
    put,
    path = "/api/v1/roles/{id}/permissions",
    tag = "Roles",
    operation_id = "setRolePermissions",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "Role id (UUID)")),
    request_body = SetRolePermissionsRequest,
    responses(
        (
            status = 200,
            description = "Permission set replaced",
            body = RoleDetail,
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
            description = "Missing roles.update, built-in role, grant-subset violation or stale authentication",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 404,
            description = "Role does not exist or was soft-deleted",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Empty permissions or unknown permission",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Role store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn set_role_permissions(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<SetRolePermissionsRequest>,
) -> Result<Json<RoleDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::RolesUpdate,
        "set_role_permissions",
        requested_role_target(&id),
    )
    .await?;

    let id = parse_id(&id)?;
    let role = load_custom_role(
        &state,
        services,
        &principal,
        &context,
        id,
        "set_role_permissions",
    )
    .await?;

    let requested = permission_set(&payload.permission_keys);
    if let Ok(keys) = &requested
        && !principal.effective.can_grant(keys)
    {
        return Err(deny_role(
            &state,
            services,
            &principal,
            &context,
            "set_role_permissions",
            ErrorCode::PermissionSubsetViolation,
            &role,
        )
        .await);
    }

    let target = role_target(&role);
    require_recent_auth(
        &state,
        services,
        &principal,
        &context,
        "set_role_permissions",
        Some(target),
    )
    .await?;

    let keys = requested?;
    let permission_keys = sorted_strings(&keys);

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(role_store_unavailable)?;

    let locked = lock_role(&mut transaction, id)
        .await
        .map_err(role_store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
    if locked.is_builtin {
        transaction
            .rollback()
            .await
            .map_err(role_store_unavailable)?;
        return Err(deny_role(
            &state,
            services,
            &principal,
            &context,
            "set_role_permissions",
            ErrorCode::RoleImmutable,
            &locked,
        )
        .await);
    }

    let previous: BTreeSet<String> = locked.permission_keys.iter().cloned().collect();
    let next: BTreeSet<String> = permission_keys.iter().cloned().collect();
    let added: Vec<String> = next.difference(&previous).cloned().collect();
    let removed: Vec<String> = previous.difference(&next).cloned().collect();

    if added.is_empty() && removed.is_empty() {
        transaction.commit().await.map_err(role_store_unavailable)?;
        return Ok(Json(detail_role(&locked, &principal.effective)));
    }

    replace_permissions(&mut transaction, id, &permission_keys)
        .await
        .map_err(role_store_unavailable)?;

    let updated = find_role_in(&mut transaction, id)
        .await
        .map_err(role_store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;

    let event = AuditEvent::builder(
        AuditAction::RolePermissionsChanged,
        AuditResult::Success,
        AuditActor::user(principal.user.id, principal.user.username.clone()),
        context.request_id,
    )
    .target(
        TargetType::Role,
        Some(updated.id),
        Some(updated.name.clone()),
    )
    .detail("added_permission_keys", added)
    .detail("removed_permission_keys", removed)
    .ip(context.client_ip);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(role_store_unavailable)?;
    Ok(Json(detail_role(&updated, &principal.effective)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/roles/{id}",
    tag = "Roles",
    operation_id = "deleteRole",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "Role id (UUID)")),
    responses(
        (
            status = 204,
            description = "Role soft-deleted",
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
            description = "Missing roles.delete, built-in role, grant-subset violation or stale authentication",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 404,
            description = "Role does not exist or was soft-deleted",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 409,
            description = "Role is still assigned to users",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Role store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn delete_role(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::RolesDelete,
        "delete_role",
        requested_role_target(&id),
    )
    .await?;

    let id = parse_id(&id)?;
    let role = load_custom_role(&state, services, &principal, &context, id, "delete_role").await?;
    ensure_role_within_permissions(&state, services, &principal, &context, "delete_role", &role)
        .await?;

    let target = role_target(&role);
    require_recent_auth(
        &state,
        services,
        &principal,
        &context,
        "delete_role",
        Some(target),
    )
    .await?;

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(role_store_unavailable)?;

    let locked = lock_role(&mut transaction, id)
        .await
        .map_err(role_store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
    if locked.is_builtin {
        transaction
            .rollback()
            .await
            .map_err(role_store_unavailable)?;
        return Err(deny_role(
            &state,
            services,
            &principal,
            &context,
            "delete_role",
            ErrorCode::RoleImmutable,
            &locked,
        )
        .await);
    }
    if !role_within_permissions(&locked, &principal.effective) {
        transaction
            .rollback()
            .await
            .map_err(role_store_unavailable)?;
        return Err(deny_role(
            &state,
            services,
            &principal,
            &context,
            "delete_role",
            ErrorCode::PermissionSubsetViolation,
            &locked,
        )
        .await);
    }

    let assignments = count_assignments(&mut transaction, id)
        .await
        .map_err(role_store_unavailable)?;
    if assignments > 0 {
        transaction
            .rollback()
            .await
            .map_err(role_store_unavailable)?;
        return Err(ApiError::new(ErrorCode::RoleInUse));
    }

    soft_delete_role(&mut transaction, id)
        .await
        .map_err(role_store_unavailable)?;

    let event = AuditEvent::builder(
        AuditAction::RoleDeleted,
        AuditResult::Success,
        AuditActor::user(principal.user.id, principal.user.username.clone()),
        context.request_id,
    )
    .target(TargetType::Role, Some(locked.id), Some(locked.name.clone()))
    .detail("role_name", locked.name.clone())
    .detail("permission_keys", locked.permission_keys.clone())
    .ip(context.client_ip);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(role_store_unavailable)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
