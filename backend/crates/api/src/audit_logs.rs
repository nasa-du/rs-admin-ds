//! Read-only audit log endpoints (docs/specs/login-system.md §5.4, §7.9,
//! §8.5).
//!
//! `GET /audit-logs` requires `audit.read` and is *not* filtered by the
//! operator's management scope: auditors must see events about users they
//! cannot manage. The list never writes an audit event. `GET
//! /audit-logs/{id}` appends `audit_log_accessed` in the same transaction as
//! the read, so a deployment without a working audit chain fails closed.

use crate::auth::{Authenticated, dependency_unavailable, write_audit_event};
use crate::authz::{RequestContext, require_permission};
use crate::client_ip::ClientIp;
use crate::error::ApiError;
use crate::middleware::RequestId;
use crate::query::{parse_id, parse_pagination, validation_error};
use crate::router::AppState;
use crate::wire::format_timestamp;
use axum::Extension;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};
use rs_admin_audit::{
    AuditAction, AuditActor, AuditEvent, AuditResult, Severity, TargetType,
    redact_forbidden_details,
};
use rs_admin_domain::ErrorCode;
use rs_admin_domain::permission::PermissionKey;
use rs_admin_security::normalize_ip;
use rs_admin_store::audit_logs::{
    AuditLogFilter, AuditLogRow, find_audit_log, list_audit_logs as store_list_audit_logs,
};
use rs_admin_store::auth::pool_clock_now;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::net::IpAddr;
use std::str::FromStr;
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::openapi::{RefOr, Schema};
use utoipa::{IntoParams, PartialSchema, ToSchema};
use uuid::Uuid;

/// Window used when the request supplies neither `from` nor `to`.
pub const DEFAULT_AUDIT_WINDOW_DAYS: i64 = 30;

/// Largest accepted span between `from` and `to`.
pub const MAX_AUDIT_WINDOW_DAYS: i64 = 90;

// ---------------------------------------------------------------------------
// Request and response types
// ---------------------------------------------------------------------------

/// Query parameters for `GET /audit-logs`. Unknown parameters are ignored.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListAuditLogsQuery {
    /// Exact actor user id (UUID).
    pub actor_id: Option<String>,
    /// Exact audit action. Unknown future actions are tolerated and return no
    /// rows; the value is never validated against the server catalog.
    #[param(value_type = Option<AuditActionSchema>)]
    pub action: Option<String>,
    /// Exact target type. Unknown future values are tolerated and return no
    /// rows.
    #[param(value_type = Option<AuditTargetTypeSchema>)]
    pub target_type: Option<String>,
    /// Exact target id (UUID).
    pub target_id: Option<String>,
    /// Closed audit result catalog.
    #[param(value_type = Option<AuditResultSchema>)]
    pub result: Option<String>,
    /// Closed audit severity catalog.
    #[param(value_type = Option<AuditSeveritySchema>)]
    pub severity: Option<String>,
    /// Exact request id / trace id (UUID).
    pub request_id: Option<String>,
    /// Exact IP address; IPv4-mapped IPv6 input is normalized to IPv4. CIDR
    /// ranges are not supported.
    pub ip: Option<String>,
    /// Inclusive start of the window, RFC3339 (for example
    /// `2026-11-08T00:00:00Z`). Defaults to `to - 30 days`.
    pub from: Option<String>,
    /// Exclusive end of the window, RFC3339. Defaults to the current database
    /// time.
    pub to: Option<String>,
    /// One-based page number; defaults to 1.
    #[param(value_type = Option<i64>)]
    pub page: Option<String>,
    /// Page size; defaults to 20 and may not exceed 200.
    #[param(value_type = Option<i64>)]
    pub page_size: Option<String>,
    /// `occurred_at` or `-occurred_at`; defaults to `-occurred_at`.
    pub sort: Option<String>,
}

/// The actor snapshot recorded on an event. `id` is null for CLI, system and
/// anonymous actions; `label` is the username snapshot or a stable marker
/// (`cli`, `system`, `anonymous`).
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AuditActorRef {
    pub id: Option<String>,
    pub label: String,
}

/// The target of an event. `type` and `id` are open strings: unknown future
/// target types are rendered as stored, and `id` is null for `system` targets.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AuditTargetRef {
    #[serde(rename = "type")]
    #[schema(value_type = AuditTargetTypeSchema)]
    pub target_type: String,
    pub id: Option<String>,
    pub label: Option<String>,
}

/// An audit row as returned by the list endpoint.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AuditLogSummary {
    pub id: String,
    pub occurred_at: String,
    pub actor: AuditActorRef,
    #[schema(value_type = AuditActionSchema)]
    pub action: String,
    pub target: AuditTargetRef,
    #[schema(value_type = AuditResultSchema)]
    pub result: String,
    #[schema(value_type = AuditSeveritySchema)]
    pub severity: String,
    pub ip: Option<String>,
    pub request_id: String,
}

/// A single audit row; adds the free-form `details` object and the recorded
/// user agent. Forbidden secret keys are masked on read as defense in depth.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AuditLogDetail {
    pub id: String,
    pub occurred_at: String,
    pub actor: AuditActorRef,
    #[schema(value_type = AuditActionSchema)]
    pub action: String,
    pub target: AuditTargetRef,
    #[schema(value_type = AuditResultSchema)]
    pub result: String,
    #[schema(value_type = AuditSeveritySchema)]
    pub severity: String,
    pub ip: Option<String>,
    pub request_id: String,
    pub user_agent: Option<String>,
    #[schema(value_type = Object)]
    pub details: serde_json::Value,
}

/// Offset-paginated audit log envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditLogListResponse {
    pub items: Vec<AuditLogSummary>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// OpenAPI schema for the open `action` string. The action catalog is
/// additive, so the schema stays a plain string but documents every current
/// value (spec §7.9).
pub struct AuditActionSchema;

impl PartialSchema for AuditActionSchema {
    fn schema() -> RefOr<Schema> {
        let values: Vec<&str> = AuditAction::ALL
            .iter()
            .map(|action| action.as_str())
            .collect();
        ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(open_catalog_description(
                "Open audit action code",
                &values,
            )))
            .into()
    }
}

impl ToSchema for AuditActionSchema {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("AuditAction")
    }
}

/// OpenAPI schema for the open `target.type` string, documenting the current
/// target-type catalog.
pub struct AuditTargetTypeSchema;

impl PartialSchema for AuditTargetTypeSchema {
    fn schema() -> RefOr<Schema> {
        let values: Vec<&str> = TargetType::ALL
            .iter()
            .map(|target_type| target_type.as_str())
            .collect();
        ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(open_catalog_description(
                "Open audit target type",
                &values,
            )))
            .into()
    }
}

impl ToSchema for AuditTargetTypeSchema {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("AuditTargetType")
    }
}

/// OpenAPI schema for the closed `result` catalog.
pub struct AuditResultSchema;

impl PartialSchema for AuditResultSchema {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(AuditResult::ALL.iter().map(|result| result.as_str())))
            .description(Some(
                "Closed audit result catalog. Unknown values cannot be stored.",
            ))
            .into()
    }
}

impl ToSchema for AuditResultSchema {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("AuditResult")
    }
}

/// OpenAPI schema for the closed `severity` catalog.
pub struct AuditSeveritySchema;

impl PartialSchema for AuditSeveritySchema {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(Severity::ALL.iter().map(|severity| severity.as_str())))
            .description(Some(
                "Closed audit severity catalog. Unknown values cannot be stored.",
            ))
            .into()
    }
}

impl ToSchema for AuditSeveritySchema {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("AuditSeverity")
    }
}

fn open_catalog_description(label: &str, values: &[&str]) -> String {
    format!(
        "{label}. Current catalog: {}. Values are additive; an unknown future \
         value is returned as stored.",
        values.join(", ")
    )
}

impl AuditLogSummary {
    fn from_row(row: &AuditLogRow) -> Self {
        Self {
            id: row.id.to_string(),
            occurred_at: format_timestamp(row.occurred_at),
            actor: AuditActorRef {
                id: row.actor_user_id.map(|id| id.to_string()),
                label: row.actor_label.clone(),
            },
            action: row.action.clone(),
            target: AuditTargetRef {
                target_type: row.target_type.clone(),
                id: row.target_id.map(|id| id.to_string()),
                label: row.target_label.clone(),
            },
            result: row.result.clone(),
            severity: row.severity.clone(),
            ip: row.ip.clone(),
            request_id: row.request_id.to_string(),
        }
    }
}

impl AuditLogDetail {
    fn from_row(row: &AuditLogRow) -> Self {
        let summary = AuditLogSummary::from_row(row);
        Self {
            id: summary.id,
            occurred_at: summary.occurred_at,
            actor: summary.actor,
            action: summary.action,
            target: summary.target,
            result: summary.result,
            severity: summary.severity,
            ip: summary.ip,
            request_id: summary.request_id,
            user_agent: row.user_agent.clone(),
            details: redact_forbidden_details(&row.details.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Query parsing
// ---------------------------------------------------------------------------

/// Server-side page coordinates plus the validated store filter.
struct ListParams {
    filter: AuditLogFilter,
    page: i64,
    page_size: i64,
}

async fn parse_list_query(
    state: &AppState,
    query: ListAuditLogsQuery,
) -> Result<ListParams, ApiError> {
    let (page, page_size) = parse_pagination(query.page.as_deref(), query.page_size.as_deref())?;

    let actor_id = parse_optional_uuid(query.actor_id.as_deref(), "/actor_id")?;
    let target_id = parse_optional_uuid(query.target_id.as_deref(), "/target_id")?;
    let request_id = parse_optional_uuid(query.request_id.as_deref(), "/request_id")?;
    let result = parse_result(query.result.as_deref())?;
    let severity = parse_severity(query.severity.as_deref())?;
    let ip = match non_empty(query.ip.as_deref()) {
        Some(raw) => Some(normalize_ip(
            raw.parse::<IpAddr>()
                .map_err(|_| validation_error("/ip", "invalid"))?,
        )),
        None => None,
    };

    let from = parse_time(query.from.as_deref())?;
    let to = parse_time(query.to.as_deref())?;
    let (from, to) = match (from, to) {
        (Some(from), Some(to)) => (from, to),
        (from, to) => {
            // The database clock is authoritative for the default window so it
            // agrees with the `occurred_at` values written by the audit chain.
            let now = pool_clock_now(state.database.pool())
                .await
                .map_err(dependency_unavailable)?;
            let to = to.unwrap_or(now);
            let from = from.unwrap_or(to - Duration::days(DEFAULT_AUDIT_WINDOW_DAYS));
            (from, to)
        }
    };
    if from >= to {
        return Err(ApiError::new(ErrorCode::InvalidTimeRange));
    }
    if to - from > Duration::days(MAX_AUDIT_WINDOW_DAYS) {
        return Err(ApiError::new(ErrorCode::TimeRangeTooLarge));
    }

    Ok(ListParams {
        filter: AuditLogFilter {
            actor_id,
            action: non_empty(query.action.as_deref()).map(str::to_owned),
            target_type: non_empty(query.target_type.as_deref()).map(str::to_owned),
            target_id,
            result,
            severity,
            request_id,
            ip,
            from,
            to,
            ascending: parse_sort(query.sort.as_deref())?,
            limit: page_size,
            offset: (page - 1) * page_size,
        },
        page,
        page_size,
    })
}

fn non_empty(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_optional_uuid(raw: Option<&str>, pointer: &'static str) -> Result<Option<Uuid>, ApiError> {
    match non_empty(raw) {
        None => Ok(None),
        Some(value) => Uuid::parse_str(value)
            .map(Some)
            .map_err(|_| validation_error(pointer, "invalid")),
    }
}

fn parse_result(raw: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = non_empty(raw) else {
        return Ok(None);
    };
    AuditResult::from_str(value)
        .map(|result| Some(result.as_str().to_owned()))
        .map_err(|_| validation_error("/result", "invalid"))
}

fn parse_severity(raw: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = non_empty(raw) else {
        return Ok(None);
    };
    Severity::from_str(value)
        .map(|severity| Some(severity.as_str().to_owned()))
        .map_err(|_| validation_error("/severity", "invalid"))
}

fn parse_time(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, ApiError> {
    match non_empty(raw) {
        None => Ok(None),
        Some(value) => DateTime::parse_from_rfc3339(value)
            .map(|time| Some(time.with_timezone(&Utc)))
            .map_err(|_| ApiError::new(ErrorCode::InvalidTimeRange)),
    }
}

/// `occurred_at` is the only sortable field; `-` means descending.
fn parse_sort(raw: Option<&str>) -> Result<bool, ApiError> {
    let Some(raw) = non_empty(raw) else {
        return Ok(false);
    };
    let (ascending, field) = match raw.strip_prefix('-') {
        Some(field) => (false, field),
        None => (true, raw),
    };
    if field == "occurred_at" {
        Ok(ascending)
    } else {
        Err(ApiError::new(ErrorCode::InvalidSort))
    }
}

fn store_unavailable(error: impl std::fmt::Display) -> ApiError {
    tracing::warn!(error = %error, "audit log store operation failed");
    ApiError::service_unavailable()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/audit-logs",
    tag = "Audit Logs",
    operation_id = "listAuditLogs",
    security(("bearerAuth" = [])),
    params(ListAuditLogsQuery),
    responses(
        (
            status = 200,
            description = "Audit rows inside the absolute time window; not filtered by management scope",
            body = AuditLogListResponse,
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
            description = "Missing audit.read",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Invalid filter, pagination, sort or time range",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Audit log store or clock unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Query(query): Query<ListAuditLogsQuery>,
) -> Result<Json<AuditLogListResponse>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal.0,
        &context,
        PermissionKey::AuditRead,
        "list_audit_logs",
        None,
    )
    .await?;

    let ListParams {
        filter,
        page: current_page,
        page_size,
    } = parse_list_query(&state, query).await?;
    let page = store_list_audit_logs(state.database.pool(), filter)
        .await
        .map_err(store_unavailable)?;

    Ok(Json(AuditLogListResponse {
        items: page.items.iter().map(AuditLogSummary::from_row).collect(),
        total: page.total,
        page: current_page,
        page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/audit-logs/{id}",
    tag = "Audit Logs",
    operation_id = "getAuditLog",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "Audit log id (UUID)")),
    responses(
        (
            status = 200,
            description = "Audit log detail; each successful read appends audit_log_accessed",
            body = AuditLogDetail,
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
            description = "Missing audit.read",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 404,
            description = "Audit log does not exist or has been purged",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Audit log store or audit chain unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
    )
)]
pub async fn get_audit_log(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    Path(id): Path<String>,
) -> Result<Json<AuditLogDetail>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let context = RequestContext::new(request_id, &headers, client_ip.0);

    require_permission(
        &state,
        services,
        &principal,
        &context,
        PermissionKey::AuditRead,
        "get_audit_log",
        None,
    )
    .await?;

    let id = parse_id(&id)?;
    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let row = find_audit_log(&mut *transaction, id)
        .await
        .map_err(store_unavailable)?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;

    // Audit access is written in the same transaction as the read and the
    // read-only API fails closed when the chain is unavailable (spec §8.5.3).
    let event = AuditEvent::builder(
        AuditAction::AuditLogAccessed,
        AuditResult::Success,
        AuditActor::user(principal.user.id, principal.user.username.clone()),
        context.request_id,
    )
    .target(TargetType::AuditLog, Some(row.id), None)
    .detail("view", "detail")
    .ip(context.client_ip);
    write_audit_event(
        &mut transaction,
        services,
        event,
        context.user_agent.clone(),
    )
    .await?;
    transaction.commit().await.map_err(dependency_unavailable)?;

    Ok(Json(AuditLogDetail::from_row(&row)))
}
