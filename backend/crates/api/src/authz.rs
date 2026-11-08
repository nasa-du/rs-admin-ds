//! Shared authorization gates, denial auditing and step-up checks
//! (docs/specs/login-system.md §5.4, §5.8, §8.5).
//!
//! The per-request pipeline rejects in a fixed order: endpoint permission,
//! target eligibility and management scope, then step-up freshness. Every
//! refusal writes `authz_denied` or `authz_step_up_denied`; deployments where
//! the audit chain is unavailable fail closed.

use crate::auth::{AuthServices, SessionPrincipal, audit_event_invalid, audit_unavailable};
use crate::auth::{user_agent, with_user_agent};
use crate::error::ApiError;
use crate::middleware::RequestId;
use crate::router::AppState;
use axum::http::HeaderMap;
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, TargetType};
use rs_admin_domain::ErrorCode;
use rs_admin_domain::permission::PermissionKey;
use rs_admin_store::auth::pool_clock_now;
use std::net::IpAddr;
use uuid::Uuid;

/// Per-request values needed to write authorization audit events.
#[derive(Clone, Debug)]
pub(crate) struct RequestContext {
    pub request_id: Uuid,
    pub client_ip: IpAddr,
    pub user_agent: Option<String>,
}

impl RequestContext {
    pub fn new(request_id: RequestId, headers: &HeaderMap, client_ip: IpAddr) -> Self {
        Self {
            request_id: request_id.0,
            client_ip,
            user_agent: user_agent(headers),
        }
    }
}

/// The object a refusal was aimed at, if it could be identified.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DenialTarget<'a> {
    User { id: Uuid, label: Option<&'a str> },
    Role { id: Uuid, label: Option<&'a str> },
}

impl<'a> DenialTarget<'a> {
    pub fn user(id: Uuid, label: Option<&'a str>) -> Self {
        Self::User { id, label }
    }

    pub fn role(id: Uuid, label: Option<&'a str>) -> Self {
        Self::Role { id, label }
    }

    fn audit_target(self) -> (TargetType, Option<Uuid>, Option<String>) {
        match self {
            Self::User { id, label } => (TargetType::User, Some(id), label.map(str::to_owned)),
            Self::Role { id, label } => (TargetType::Role, Some(id), label.map(str::to_owned)),
        }
    }
}

/// A refusal to record in the audit chain.
pub(crate) struct Denial<'a> {
    action: AuditAction,
    operation: &'static str,
    reason: ErrorCode,
    target: Option<DenialTarget<'a>>,
    max_age_seconds: Option<i64>,
}

impl<'a> Denial<'a> {
    pub fn authorization(
        operation: &'static str,
        reason: ErrorCode,
        target: Option<DenialTarget<'a>>,
    ) -> Self {
        Self {
            action: AuditAction::AuthzDenied,
            operation,
            reason,
            target,
            max_age_seconds: None,
        }
    }

    pub fn reauth(
        operation: &'static str,
        target: Option<DenialTarget<'a>>,
        max_age_seconds: i64,
    ) -> Self {
        Self {
            action: AuditAction::AuthzStepUpDenied,
            operation,
            reason: ErrorCode::ReauthRequired,
            target,
            max_age_seconds: Some(max_age_seconds),
        }
    }
}

/// Write an `authz_denied` or `authz_step_up_denied` event. Deployments where
/// the audit chain is unavailable fail closed with 503.
pub(crate) async fn write_denial(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    denial: Denial<'_>,
) -> Result<(), ApiError> {
    let mut builder = AuditEvent::builder(
        denial.action,
        AuditResult::Denied,
        AuditActor::user(principal.user.id, principal.user.username.clone()),
        context.request_id,
    );
    let (target_type, target_id, target_label) = match denial.target {
        Some(target) => target.audit_target(),
        None => (TargetType::System, None, None),
    };
    builder = builder
        .target(target_type, target_id, target_label)
        .detail("operation", denial.operation)
        .detail("reason_code", denial.reason.as_str())
        .ip(context.client_ip);
    if let Some(max_age_seconds) = denial.max_age_seconds {
        builder = builder.detail("max_age_seconds", max_age_seconds);
    }

    let event = with_user_agent(builder, context.user_agent.clone())
        .build()
        .map_err(audit_event_invalid)?;
    services
        .audit()
        .write(state.database.pool(), &event)
        .await
        .map_err(audit_unavailable)?;
    Ok(())
}

/// Permission gate. A missing permission produces 403 `forbidden` and an
/// `authz_denied` audit event.
pub(crate) async fn require_permission(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    permission: PermissionKey,
    operation: &'static str,
    target: Option<DenialTarget<'_>>,
) -> Result<(), ApiError> {
    if principal.effective.contains(permission) {
        return Ok(());
    }
    write_denial(
        state,
        services,
        principal,
        context,
        Denial::authorization(operation, ErrorCode::Forbidden, target),
    )
    .await?;
    Err(ApiError::new(ErrorCode::Forbidden))
}

/// Reauthentication gate for sensitive writes (spec §5.8). The session's
/// `authenticated_at` is authoritative.
pub(crate) async fn require_recent_auth(
    state: &AppState,
    services: &AuthServices,
    principal: &SessionPrincipal,
    context: &RequestContext,
    operation: &'static str,
    target: Option<DenialTarget<'_>>,
) -> Result<(), ApiError> {
    let max_age = services.reauth_max_age();
    let max_age_seconds = i64::try_from(max_age.as_secs()).unwrap_or(i64::MAX);
    let now = pool_clock_now(state.database.pool())
        .await
        .map_err(crate::auth::dependency_unavailable)?;
    if now.signed_duration_since(principal.session.authenticated_at)
        <= chrono::Duration::seconds(max_age_seconds)
    {
        return Ok(());
    }

    write_denial(
        state,
        services,
        principal,
        context,
        Denial::reauth(operation, target, max_age_seconds),
    )
    .await?;

    Err(ApiError::new(ErrorCode::ReauthRequired)
        .with_details(serde_json::json!({ "max_age": max_age_seconds })))
}
