//! User role assignment and lifecycle actions: role replacement, status,
//! credential reset, unlock and session revocation
//! (docs/specs/login-system.md §5.7–§5.9, §6.3, §7.7).
//!
//! Every handler enforces the endpoint permission, management scope, the
//! self-action prohibition and the applicable last-super-admin invariant
//! before touching the database. The check is repeated under a row lock inside
//! the transaction so concurrent writers cannot bypass it. Each successful
//! change and each refusal appends to the audit chain in the same transaction
//! as the change itself.

use crate::auth::{
    AuthServices, Authenticated, SessionPrincipal, password_unavailable, write_audit_event,
};
use crate::authz::{
    Denial, DenialTarget, RequestContext, require_permission, require_recent_auth, write_denial,
};
use crate::client_ip::ClientIp;
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::middleware::RequestId;
use crate::query::{parse_id, validation_error};
use crate::roles::role_is_assignable;
use crate::router::AppState;
use crate::users::{
    UserCreatedResponse, UserDetail, detail_user, ensure_roles_assignable, parse_role_ids,
    policy_error, requested_user_target, role_refs, store_unavailable,
    temporary_password_generation_failed, unknown_role_error, user_target, user_within_scope,
};
use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, Severity, TargetType};
use rs_admin_domain::ErrorCode;
use rs_admin_domain::lifecycle::{UserStatus, demotes_last_super_admin, disables_last_super_admin};
use rs_admin_domain::permission::PermissionKey;
use rs_admin_domain::rbac::BuiltInRole;
use rs_admin_security::{PasswordPolicy, generate_temporary_password};
use rs_admin_store::auth::{
    REVOKE_REASON_ACCOUNT_DISABLED, REVOKE_REASON_ADMIN_REVOKE, UserRole, bump_token_version,
    load_role_grants, lock_user_row, reset_login_failures, revoke_user_refresh_tokens,
    revoke_user_sessions as revoke_all_sessions,
};
use rs_admin_store::roles::{RoleRow, find_roles_by_ids, lock_roles_by_ids};
use rs_admin_store::users::{
    UserRow, baseline_role_id, find_enabled_super_admin_ids, find_user, find_user_in,
    lock_enabled_super_admin_ids, replace_user_roles, reset_temporary_password, update_user_status,
};
use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeSet;
use utoipa::ToSchema;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// `PUT /users/{id}/roles` request body. The array is the complete desired
/// role set; the baseline `user` role is retained even when omitted.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetUserRolesRequest {
    pub role_ids: Vec<String>,
}

/// `PUT /users/{id}/status` request body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetUserStatusRequest {
    /// `active` or `disabled`.
    pub status: String,
}

/// `POST /users/{id}/reset-password` request body. An omitted password is
/// generated; a supplied one must satisfy the password policy.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResetUserPasswordRequest {
    #[serde(default)]
    pub password: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load the target after the endpoint gate and enforce the self-action
/// prohibition and management scope (spec §6.3 rules 4–5).
async fn authorize_target(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    raw_id: &str,
    permission: PermissionKey,
    operation: &'static str,
) -> Result<UserRow, ApiError> {
    require_permission(
        state,
        services,
        principal,
        context,
        permission,
        operation,
        requested_user_target(raw_id),
    )
    .await?;

    let id = parse_id(raw_id)?;
    let row = find_user(state.database.pool(), id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;

    if row.id == principal.user.id {
        write_denial(
            state,
            services,
            principal,
            context,
            Denial::authorization(
                operation,
                ErrorCode::SelfActionForbidden,
                Some(user_target(&row)),
            ),
        )
        .await?;
        return Err(ApiError::new(ErrorCode::SelfActionForbidden));
    }

    let roles = load_role_grants(state.database.pool(), row.id)
        .await
        .map_err(store_unavailable)?;
    if !user_within_scope(&principal.effective, &roles) {
        write_denial(
            state,
            services,
            principal,
            context,
            Denial::authorization(operation, ErrorCode::OutOfScope, Some(user_target(&row))),
        )
        .await?;
        return Err(ApiError::new(ErrorCode::OutOfScope));
    }

    Ok(row)
}

/// Lock the target user row and load its active role grants.
async fn lock_target(
    transaction: &mut Transaction<'_, Postgres>,
    target_id: Uuid,
) -> Result<(rs_admin_store::auth::AuthUser, Vec<UserRole>), ApiError> {
    let locked = lock_user_row(transaction, target_id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
    let roles = load_role_grants(&mut **transaction, target_id)
        .await
        .map_err(store_unavailable)?;
    Ok((locked, roles))
}

/// The target row as read under its transaction lock.
struct LockedTarget<'a> {
    id: Uuid,
    label: &'a str,
    roles: &'a [UserRole],
}

/// Re-check the management scope after the target row lock, because a
/// concurrent role change may have moved the target out of scope. On a
/// refusal the transaction is rolled back (releasing its connection and row
/// locks) before the denial is appended, so the audit write cannot exhaust a
/// small pool.
async fn recheck_scope_under_lock<'t>(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    operation: &'static str,
    transaction: Transaction<'t, Postgres>,
    target: LockedTarget<'_>,
) -> Result<Transaction<'t, Postgres>, ApiError> {
    if user_within_scope(&principal.effective, target.roles) {
        return Ok(transaction);
    }
    transaction.rollback().await.ok();
    write_denial(
        state,
        services,
        principal,
        context,
        Denial::authorization(
            operation,
            ErrorCode::OutOfScope,
            Some(DenialTarget::user(target.id, Some(target.label))),
        ),
    )
    .await?;
    Err(ApiError::new(ErrorCode::OutOfScope))
}

fn audit_event(
    action: AuditAction,
    principal: &SessionPrincipal,
    context: &RequestContext,
    target_id: Uuid,
    target_label: &str,
) -> rs_admin_audit::AuditEventBuilder {
    AuditEvent::builder(
        action,
        AuditResult::Success,
        AuditActor::user(principal.user.id, principal.user.username.clone()),
        context.request_id,
    )
    .target(
        TargetType::User,
        Some(target_id),
        Some(target_label.to_owned()),
    )
    .ip(context.client_ip)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    put,
    path = "/api/v1/users/{id}/roles",
    tag = "Users",
    operation_id = "setUserRoles",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "User id (UUID)")),
    request_body = SetUserRolesRequest,
    responses(
        (
            status = 200,
            description = "Role set replaced; the baseline `user` role is always retained",
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
            description = "Missing users.assign_role, out of scope, self action, grant-subset violation or stale authentication",
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
            description = "Demoting the last enabled super administrator",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Malformed role id or unknown/soft-deleted role",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "User store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn set_user_roles(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<SetUserRolesRequest>,
) -> Result<Json<UserDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    let existing = authorize_target(
        &state,
        services,
        &principal,
        &context,
        &id,
        PermissionKey::UsersAssignRole,
        "set_user_roles",
    )
    .await?;
    let target_id = existing.id;

    let requested = parse_role_ids(&payload.role_ids)?;
    let baseline = baseline_role_id(state.database.pool())
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;
    let mut desired = requested;
    if !desired.contains(&baseline) {
        desired.push(baseline);
    }

    // Target eligibility (role existence, privileged roles and the
    // grant-subset rule) is checked before step-up so a stale session never
    // sees a reauthentication prompt for a request it could not perform
    // anyway (spec §5.4).
    let roles = find_roles_by_ids(state.database.pool(), &desired)
        .await
        .map_err(store_unavailable)?;
    if roles.len() != desired.len() {
        return Err(unknown_role_error());
    }
    ensure_roles_assignable(
        &state,
        services,
        &principal,
        &context,
        "set_user_roles",
        &roles,
    )
    .await?;

    require_recent_auth(
        &state,
        services,
        &principal,
        &context,
        "set_user_roles",
        Some(user_target(&existing)),
    )
    .await?;

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(store_unavailable)?;

    // Acquire the enabled super-administrator row locks before the target, so
    // every lifecycle writer takes the same locks in the same order.
    lock_enabled_super_admin_ids(&mut transaction)
        .await
        .map_err(store_unavailable)?;
    let (locked, locked_roles) = lock_target(&mut transaction, target_id).await?;
    let mut transaction = recheck_scope_under_lock(
        &state,
        services,
        &principal,
        &context,
        "set_user_roles",
        transaction,
        LockedTarget {
            id: locked.id,
            label: &locked.username,
            roles: &locked_roles,
        },
    )
    .await?;
    // A role change only touches `user_roles`, so the locking scan above may
    // have evaluated its qualification against a pre-wait snapshot. Re-read
    // the set now that every lock is held.
    let enabled_super_admins = find_enabled_super_admin_ids(&mut *transaction)
        .await
        .map_err(store_unavailable)?;

    // Re-validate under share locks: a concurrent soft delete must not leave
    // the target holding a role that is no longer assignable.
    let locked_desired = lock_roles_by_ids(&mut transaction, &desired)
        .await
        .map_err(store_unavailable)?;
    if locked_desired.len() != desired.len() {
        transaction.rollback().await.ok();
        return Err(unknown_role_error());
    }
    if locked_desired
        .iter()
        .any(|role| !role_is_assignable(role, &principal.effective))
    {
        transaction.rollback().await.ok();
        ensure_roles_assignable(
            &state,
            services,
            &principal,
            &context,
            "set_user_roles",
            &locked_desired,
        )
        .await?;
        return Err(ApiError::new(ErrorCode::PermissionSubsetViolation));
    }

    let had_super_admin = locked_roles.iter().any(is_super_admin_role);
    let keeps_super_admin = locked_desired.iter().any(is_super_admin_role);
    let target_is_enabled_super_admin = UserStatus::parse(&locked.status)
        == Some(UserStatus::Active)
        && had_super_admin
        && enabled_super_admins.contains(&locked.id);
    if demotes_last_super_admin(
        target_is_enabled_super_admin,
        keeps_super_admin,
        enabled_super_admins.len(),
    ) {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::LastSuperAdmin));
    }

    let current_ids: BTreeSet<Uuid> = locked_roles.iter().map(|role| role.id).collect();
    let desired_ids: BTreeSet<Uuid> = desired.iter().copied().collect();
    let added: Vec<Uuid> = desired_ids.difference(&current_ids).copied().collect();
    let removed: Vec<Uuid> = current_ids.difference(&desired_ids).copied().collect();

    if added.is_empty() && removed.is_empty() {
        let unchanged = find_user_in(&mut transaction, target_id)
            .await
            .map_err(store_unavailable)?
            .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;
        transaction.commit().await.map_err(store_unavailable)?;
        return Ok(Json(detail_user(&unchanged, &role_refs(&locked_roles))));
    }

    // A change that adds or removes `admin`/`super_admin` is critical
    // (spec §8.5.1).
    let privileged_ids: BTreeSet<Uuid> = locked_roles
        .iter()
        .filter(|role| is_privileged_role(*role))
        .map(|role| role.id)
        .chain(
            locked_desired
                .iter()
                .filter(|role| is_privileged_role(*role))
                .map(|role| role.id),
        )
        .collect();
    let privileged_changed = added
        .iter()
        .chain(removed.iter())
        .any(|id| privileged_ids.contains(id));
    let super_admin_ids: BTreeSet<Uuid> = locked_roles
        .iter()
        .filter(|role| is_super_admin_role(*role))
        .map(|role| role.id)
        .chain(
            locked_desired
                .iter()
                .filter(|role| is_super_admin_role(*role))
                .map(|role| role.id),
        )
        .collect();
    let super_admin_involved = added
        .iter()
        .chain(removed.iter())
        .any(|id| super_admin_ids.contains(id));

    replace_user_roles(
        &mut transaction,
        target_id,
        &desired,
        Some(principal.user.id),
    )
    .await
    .map_err(store_unavailable)?;

    let updated = find_user_in(&mut transaction, target_id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;
    let updated_roles = load_role_grants(&mut *transaction, target_id)
        .await
        .map_err(store_unavailable)?;

    let added: Vec<String> = added.iter().map(Uuid::to_string).collect();
    let removed: Vec<String> = removed.iter().map(Uuid::to_string).collect();
    let mut event = audit_event(
        AuditAction::UserRolesChanged,
        &principal,
        &context,
        target_id,
        &locked.username,
    )
    .detail("added_role_ids", added)
    .detail("removed_role_ids", removed)
    .detail("super_admin_involved", super_admin_involved);
    if privileged_changed {
        event = event.severity(Severity::Critical);
    }
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(store_unavailable)?;
    Ok(Json(detail_user(&updated, &role_refs(&updated_roles))))
}

#[utoipa::path(
    put,
    path = "/api/v1/users/{id}/status",
    tag = "Users",
    operation_id = "setUserStatus",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "User id (UUID)")),
    request_body = SetUserStatusRequest,
    responses(
        (
            status = 200,
            description = "User status updated; disabling revokes every session and increments the token version",
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
            description = "Missing users.set_status, out of scope, self action or stale authentication",
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
            description = "Disabling the last enabled super administrator",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Status is not `active` or `disabled`",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "User store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn set_user_status(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<SetUserStatusRequest>,
) -> Result<Json<UserDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    let existing = authorize_target(
        &state,
        services,
        &principal,
        &context,
        &id,
        PermissionKey::UsersSetStatus,
        "set_user_status",
    )
    .await?;

    require_recent_auth(
        &state,
        services,
        &principal,
        &context,
        "set_user_status",
        Some(user_target(&existing)),
    )
    .await?;

    let requested =
        UserStatus::parse(&payload.status).ok_or_else(|| validation_error("/status", "invalid"))?;
    let target_id = existing.id;

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(store_unavailable)?;

    // Lock every enabled super administrator before the target, so concurrent
    // disables and demotions cannot both observe a stale count (spec §6.3
    // rule 7).
    lock_enabled_super_admin_ids(&mut transaction)
        .await
        .map_err(store_unavailable)?;
    let (locked, locked_roles) = lock_target(&mut transaction, target_id).await?;
    let mut transaction = recheck_scope_under_lock(
        &state,
        services,
        &principal,
        &context,
        "set_user_status",
        transaction,
        LockedTarget {
            id: locked.id,
            label: &locked.username,
            roles: &locked_roles,
        },
    )
    .await?;
    // A demotion only touches `user_roles`, so the locking scan above may have
    // evaluated its qualification against a pre-wait snapshot. Re-read the set
    // now that every lock is held.
    let enabled_super_admins = find_enabled_super_admin_ids(&mut *transaction)
        .await
        .map_err(store_unavailable)?;

    let current =
        UserStatus::parse(&locked.status).ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;
    if current == requested {
        let unchanged = find_user_in(&mut transaction, target_id)
            .await
            .map_err(store_unavailable)?
            .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;
        transaction.commit().await.map_err(store_unavailable)?;
        return Ok(Json(detail_user(&unchanged, &role_refs(&locked_roles))));
    }

    let target_is_enabled_super_admin = current == UserStatus::Active
        && locked_roles.iter().any(is_super_admin_role)
        && enabled_super_admins.contains(&locked.id);
    if requested == UserStatus::Disabled
        && disables_last_super_admin(target_is_enabled_super_admin, enabled_super_admins.len())
    {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::LastSuperAdmin));
    }

    update_user_status(&mut transaction, target_id, requested)
        .await
        .map_err(store_unavailable)?;

    // Disabling ends every session and invalidates every access token;
    // enabling only restores the login eligibility (spec §5.9).
    let mut sessions_revoked = 0i64;
    if requested == UserStatus::Disabled {
        sessions_revoked = i64::try_from(
            revoke_all_sessions(&mut transaction, target_id, REVOKE_REASON_ACCOUNT_DISABLED)
                .await
                .map_err(store_unavailable)?,
        )
        .unwrap_or(i64::MAX);
        revoke_user_refresh_tokens(&mut transaction, target_id, REVOKE_REASON_ACCOUNT_DISABLED)
            .await
            .map_err(store_unavailable)?;
        bump_token_version(&mut transaction, target_id)
            .await
            .map_err(store_unavailable)?;
    }

    let updated = find_user_in(&mut transaction, target_id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;

    let event = audit_event(
        AuditAction::UserStatusChanged,
        &principal,
        &context,
        target_id,
        &locked.username,
    )
    .detail("from", current.as_str())
    .detail("to", requested.as_str())
    .detail("sessions_revoked_count", sessions_revoked);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(store_unavailable)?;
    Ok(Json(detail_user(&updated, &role_refs(&locked_roles))))
}

#[utoipa::path(
    post,
    path = "/api/v1/users/{id}/reset-password",
    tag = "Users",
    operation_id = "resetUserPassword",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "User id (UUID)")),
    request_body = ResetUserPasswordRequest,
    responses(
        (
            status = 200,
            description = "Temporary password issued; every target session revoked and the token version incremented",
            body = UserCreatedResponse,
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
            description = "Missing users.reset_password, out of scope, self action or stale authentication",
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
            status = 422,
            description = "Password policy violation",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "User store, password hashing or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn reset_user_password(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<ResetUserPasswordRequest>,
) -> Result<Json<UserCreatedResponse>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    let existing = authorize_target(
        &state,
        services,
        &principal,
        &context,
        &id,
        PermissionKey::UsersResetPassword,
        "reset_user_password",
    )
    .await?;

    require_recent_auth(
        &state,
        services,
        &principal,
        &context,
        "reset_user_password",
        Some(user_target(&existing)),
    )
    .await?;

    let manual = payload.password.as_deref();
    let temporary_password = match manual {
        Some(value) => {
            PasswordPolicy::new()
                .validate(
                    value,
                    &[&existing.username, &existing.email, &existing.display_name],
                )
                .map_err(policy_error)?;
            value.to_owned()
        }
        None => generate_temporary_password().map_err(temporary_password_generation_failed)?,
    };
    let password_hash = services
        .passwords()
        .hash(&temporary_password)
        .await
        .map_err(password_unavailable)?;

    let target_id = existing.id;
    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(store_unavailable)?;

    let (locked, locked_roles) = lock_target(&mut transaction, target_id).await?;
    let mut transaction = recheck_scope_under_lock(
        &state,
        services,
        &principal,
        &context,
        "reset_user_password",
        transaction,
        LockedTarget {
            id: locked.id,
            label: &locked.username,
            roles: &locked_roles,
        },
    )
    .await?;

    // An identifier change between validation and the lock could otherwise
    // install a password equal to the new username or email.
    if let Some(value) = manual
        && let Err(violation) = PasswordPolicy::new().validate(
            value,
            &[&locked.username, &locked.email, &locked.display_name],
        )
    {
        transaction.rollback().await.ok();
        return Err(policy_error(violation));
    }

    let sessions_revoked = i64::try_from(
        revoke_all_sessions(&mut transaction, target_id, REVOKE_REASON_ADMIN_REVOKE)
            .await
            .map_err(store_unavailable)?,
    )
    .unwrap_or(i64::MAX);
    revoke_user_refresh_tokens(&mut transaction, target_id, REVOKE_REASON_ADMIN_REVOKE)
        .await
        .map_err(store_unavailable)?;
    bump_token_version(&mut transaction, target_id)
        .await
        .map_err(store_unavailable)?;
    let expires_at = reset_temporary_password(
        &mut transaction,
        target_id,
        &password_hash,
        services.temporary_password_ttl(),
    )
    .await
    .map_err(store_unavailable)?;

    let updated = find_user_in(&mut transaction, target_id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;
    let expires_at_wire = crate::wire::format_timestamp(expires_at);

    let event = audit_event(
        AuditAction::UserPasswordReset,
        &principal,
        &context,
        target_id,
        &locked.username,
    )
    .detail(
        "method",
        if manual.is_some() {
            "manual"
        } else {
            "generated"
        },
    )
    .detail("temporary_password_expires_at", expires_at_wire.clone())
    .detail("sessions_revoked_count", sessions_revoked);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(store_unavailable)?;
    Ok(Json(UserCreatedResponse {
        user: detail_user(&updated, &role_refs(&locked_roles)),
        temporary_password,
        temporary_password_expires_at: expires_at_wire,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/users/{id}/unlock",
    tag = "Users",
    operation_id = "unlockUser",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "User id (UUID)")),
    responses(
        (
            status = 200,
            description = "Account-level login lock and failure count cleared (idempotent)",
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
            description = "Missing users.unlock, out of scope or self action",
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
            description = "User store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn unlock_user(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
) -> Result<Json<UserDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    let existing = authorize_target(
        &state,
        services,
        &principal,
        &context,
        &id,
        PermissionKey::UsersUnlock,
        "unlock_user",
    )
    .await?;
    let target_id = existing.id;

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(store_unavailable)?;

    let (locked, locked_roles) = lock_target(&mut transaction, target_id).await?;
    let mut transaction = recheck_scope_under_lock(
        &state,
        services,
        &principal,
        &context,
        "unlock_user",
        transaction,
        LockedTarget {
            id: locked.id,
            label: &locked.username,
            roles: &locked_roles,
        },
    )
    .await?;

    let cleared_failure_count = i64::from(locked.failed_login_count);
    reset_login_failures(&mut transaction, target_id)
        .await
        .map_err(store_unavailable)?;

    let updated = find_user_in(&mut transaction, target_id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::InternalError))?;

    let event = audit_event(
        AuditAction::UserUnlocked,
        &principal,
        &context,
        target_id,
        &locked.username,
    )
    .detail("cleared_failure_count", cleared_failure_count);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(store_unavailable)?;
    Ok(Json(detail_user(&updated, &role_refs(&locked_roles))))
}

#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}/sessions",
    tag = "Users",
    operation_id = "revokeUserSessions",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "User id (UUID)")),
    responses(
        (
            status = 204,
            description = "Every target session revoked and the token version incremented",
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
            description = "Missing sessions.revoke, out of scope, self action or stale authentication",
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
            description = "User store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn revoke_user_sessions(
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

    let existing = authorize_target(
        &state,
        services,
        &principal,
        &context,
        &id,
        PermissionKey::SessionsRevoke,
        "revoke_user_sessions",
    )
    .await?;

    require_recent_auth(
        &state,
        services,
        &principal,
        &context,
        "revoke_user_sessions",
        Some(user_target(&existing)),
    )
    .await?;

    let target_id = existing.id;
    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(store_unavailable)?;

    let (locked, locked_roles) = lock_target(&mut transaction, target_id).await?;
    let mut transaction = recheck_scope_under_lock(
        &state,
        services,
        &principal,
        &context,
        "revoke_user_sessions",
        transaction,
        LockedTarget {
            id: locked.id,
            label: &locked.username,
            roles: &locked_roles,
        },
    )
    .await?;

    let revoked_sessions = i64::try_from(
        revoke_all_sessions(&mut transaction, target_id, REVOKE_REASON_ADMIN_REVOKE)
            .await
            .map_err(store_unavailable)?,
    )
    .unwrap_or(i64::MAX);
    revoke_user_refresh_tokens(&mut transaction, target_id, REVOKE_REASON_ADMIN_REVOKE)
        .await
        .map_err(store_unavailable)?;
    bump_token_version(&mut transaction, target_id)
        .await
        .map_err(store_unavailable)?;

    let event = audit_event(
        AuditAction::UserSessionsRevoked,
        &principal,
        &context,
        target_id,
        &locked.username,
    )
    .detail("sessions_revoked_count", revoked_sessions);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;

    transaction.commit().await.map_err(store_unavailable)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

trait RoleIdentity {
    fn is_builtin(&self) -> bool;
    fn role_name(&self) -> &str;
}

impl RoleIdentity for UserRole {
    fn is_builtin(&self) -> bool {
        self.is_builtin
    }

    fn role_name(&self) -> &str {
        &self.name
    }
}

impl RoleIdentity for RoleRow {
    fn is_builtin(&self) -> bool {
        self.is_builtin
    }

    fn role_name(&self) -> &str {
        &self.name
    }
}

/// The code-defined role a loaded row represents, if any.
fn builtin_role(role: &impl RoleIdentity) -> Option<BuiltInRole> {
    role.is_builtin()
        .then(|| BuiltInRole::from_code(role.role_name()))
        .flatten()
}

fn is_super_admin_role(role: &impl RoleIdentity) -> bool {
    builtin_role(role) == Some(BuiltInRole::SuperAdmin)
}

fn is_privileged_role(role: &impl RoleIdentity) -> bool {
    matches!(
        builtin_role(role),
        Some(BuiltInRole::Admin | BuiltInRole::SuperAdmin)
    )
}
