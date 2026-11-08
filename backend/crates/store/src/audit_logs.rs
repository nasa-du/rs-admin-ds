//! Read-only audit log queries (docs/specs/login-system.md §7.9).
//!
//! The API projects only the event fields clients may see: `chain_seq`,
//! `prev_hash` and `row_hash` deliberately never leave the database. Filters
//! are applied in SQL so offset pagination and `total` describe the same set.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Executor, PgPool, Postgres, QueryBuilder};
use std::fmt;
use std::net::IpAddr;
use uuid::Uuid;

/// A stored audit row projected for the query API.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AuditLogRow {
    pub id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub actor_user_id: Option<Uuid>,
    pub actor_label: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub target_label: Option<String>,
    pub result: String,
    pub severity: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub request_id: Uuid,
    pub details: sqlx::types::Json<Value>,
}

/// Validated audit list filters and offset pagination. `from` is inclusive and
/// `to` is exclusive.
#[derive(Clone, Debug)]
pub struct AuditLogFilter {
    pub actor_id: Option<Uuid>,
    pub action: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<Uuid>,
    pub result: Option<String>,
    pub severity: Option<String>,
    pub request_id: Option<Uuid>,
    pub ip: Option<IpAddr>,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// `true` for `occurred_at asc`, `false` for the default descending order.
    pub ascending: bool,
    pub limit: i64,
    pub offset: i64,
}

/// One page of audit rows plus the filtered total.
#[derive(Clone, Debug)]
pub struct AuditLogPage {
    pub items: Vec<AuditLogRow>,
    pub total: i64,
}

/// Errors from the audit query store. Display text never includes SQL.
#[derive(thiserror::Error)]
pub enum AuditLogStoreError {
    #[error("audit log query failed")]
    Database(#[source] sqlx::Error),
}

impl fmt::Debug for AuditLogStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AuditLogStoreError")
            .field(&self.to_string())
            .finish()
    }
}

const AUDIT_LOG_SELECT: &str = "SELECT id, occurred_at, actor_user_id, actor_label, action, \
     target_type, target_id, target_label, result, severity, host(ip) AS ip, user_agent, \
     request_id, details FROM audit_logs WHERE 1 = 1";

/// List audit rows in the absolute time window, applying every filter in SQL.
pub async fn list_audit_logs(
    pool: &PgPool,
    filter: AuditLogFilter,
) -> Result<AuditLogPage, AuditLogStoreError> {
    let mut query = QueryBuilder::<Postgres>::new(AUDIT_LOG_SELECT);
    push_filters(&mut query, &filter);
    query.push(" ORDER BY occurred_at");
    query.push(if filter.ascending { " ASC" } else { " DESC" });
    // `chain_seq` is unique, so every page has a total order even when rows
    // share `occurred_at`; offset pagination can then neither repeat nor skip.
    query.push(", chain_seq");
    query.push(if filter.ascending { " ASC" } else { " DESC" });
    query.push(" LIMIT ").push_bind(filter.limit);
    query.push(" OFFSET ").push_bind(filter.offset);

    let items = query
        .build_query_as::<AuditLogRow>()
        .fetch_all(pool)
        .await
        .map_err(AuditLogStoreError::Database)?;

    let mut count = QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM audit_logs WHERE 1 = 1");
    push_filters(&mut count, &filter);
    let total = count
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await
        .map_err(AuditLogStoreError::Database)?;

    Ok(AuditLogPage { items, total })
}

/// Load one audit row by id. The caller decides whether a missing row is a
/// 404; scope never hides audit rows from `audit.read` holders.
pub async fn find_audit_log<'e, E>(
    executor: E,
    id: Uuid,
) -> Result<Option<AuditLogRow>, AuditLogStoreError>
where
    E: Executor<'e, Database = Postgres>,
{
    let mut query = QueryBuilder::<Postgres>::new(AUDIT_LOG_SELECT);
    query.push(" AND id = ").push_bind(id);
    query
        .build_query_as::<AuditLogRow>()
        .fetch_optional(executor)
        .await
        .map_err(AuditLogStoreError::Database)
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &AuditLogFilter) {
    query
        .push(" AND occurred_at >= ")
        .push_bind(filter.from)
        .push(" AND occurred_at < ")
        .push_bind(filter.to);

    if let Some(actor_id) = filter.actor_id {
        query.push(" AND actor_user_id = ").push_bind(actor_id);
    }
    if let Some(action) = filter.action.as_deref() {
        query.push(" AND action = ").push_bind(action.to_owned());
    }
    if let Some(target_type) = filter.target_type.as_deref() {
        query
            .push(" AND target_type = ")
            .push_bind(target_type.to_owned());
    }
    if let Some(target_id) = filter.target_id {
        query.push(" AND target_id = ").push_bind(target_id);
    }
    if let Some(result) = filter.result.as_deref() {
        query.push(" AND result = ").push_bind(result.to_owned());
    }
    if let Some(severity) = filter.severity.as_deref() {
        query
            .push(" AND severity = ")
            .push_bind(severity.to_owned());
    }
    if let Some(request_id) = filter.request_id {
        query.push(" AND request_id = ").push_bind(request_id);
    }
    if let Some(ip) = filter.ip {
        query
            .push(" AND ip = ")
            .push_bind(ip.to_string())
            .push("::inet");
    }
}
