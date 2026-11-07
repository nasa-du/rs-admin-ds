//! Validated audit events.
//!
//! Event fields are private and can only be set through the builder, which
//! enforces the catalog's severity and target rules plus the "no secrets in
//! details" invariant.

use crate::catalog::{AuditAction, AuditResult, Severity, TargetRule, TargetType};
use serde_json::{Map, Value};
use std::net::IpAddr;
use uuid::Uuid;

/// Maximum recorded user-agent length (spec §8.5.2).
pub const MAX_USER_AGENT_CHARS: usize = 512;

/// Maximum actor label length; usernames are at most 32 characters.
pub const MAX_ACTOR_LABEL_CHARS: usize = 128;

/// Exact detail keys that must never be persisted.
const FORBIDDEN_DETAIL_KEYS: &[&str] = &[
    "password",
    "password_hash",
    "temporary_password",
    "access_token",
    "refresh_token",
    "authorization",
    "cookie",
    "secret",
    "api_key",
    "private_key",
    "client_secret",
];

/// Who performed the action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditActor {
    user_id: Option<Uuid>,
    label: String,
}

impl AuditActor {
    /// A user action; `label` is the username snapshot.
    pub fn user(user_id: Uuid, label: impl Into<String>) -> Self {
        Self {
            user_id: Some(user_id),
            label: label.into(),
        }
    }

    pub fn cli() -> Self {
        Self {
            user_id: None,
            label: "cli".to_owned(),
        }
    }

    pub fn system() -> Self {
        Self {
            user_id: None,
            label: "system".to_owned(),
        }
    }

    pub fn anonymous() -> Self {
        Self {
            user_id: None,
            label: "anonymous".to_owned(),
        }
    }

    pub fn user_id(&self) -> Option<Uuid> {
        self.user_id
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

/// What the action was performed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditTarget {
    target_type: TargetType,
    id: Option<Uuid>,
    label: Option<String>,
}

impl AuditTarget {
    pub fn target_type(&self) -> TargetType {
        self.target_type
    }

    pub fn id(&self) -> Option<Uuid> {
        self.id
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// A fully validated audit event.
#[derive(Clone, Debug)]
pub struct AuditEvent {
    action: AuditAction,
    result: AuditResult,
    severity: Severity,
    actor: AuditActor,
    target: AuditTarget,
    ip: Option<IpAddr>,
    user_agent: Option<String>,
    request_id: Uuid,
    details: Map<String, Value>,
}

impl AuditEvent {
    pub fn builder(
        action: AuditAction,
        result: AuditResult,
        actor: AuditActor,
        request_id: Uuid,
    ) -> AuditEventBuilder {
        // Flexible rules (for example a login failure for a known versus an
        // unknown identifier) have no safe default; the caller must choose.
        let target = match action.target_rule() {
            TargetRule::Fixed(target_type) => Some(AuditTarget {
                target_type,
                id: None,
                label: None,
            }),
            TargetRule::Flexible(_) => None,
        };
        AuditEventBuilder {
            action,
            result,
            severity: action.default_severity(),
            actor,
            target,
            ip: None,
            user_agent: None,
            request_id,
            details: Map::new(),
        }
    }

    pub fn action(&self) -> AuditAction {
        self.action
    }

    pub fn result(&self) -> AuditResult {
        self.result
    }

    pub fn severity(&self) -> Severity {
        self.severity
    }

    pub fn actor(&self) -> &AuditActor {
        &self.actor
    }

    pub fn target(&self) -> &AuditTarget {
        &self.target
    }

    pub fn ip(&self) -> Option<IpAddr> {
        self.ip
    }

    pub fn user_agent(&self) -> Option<&str> {
        self.user_agent.as_deref()
    }

    pub fn request_id(&self) -> Uuid {
        self.request_id
    }

    pub fn details(&self) -> &Map<String, Value> {
        &self.details
    }
}

/// Builder returned by [`AuditEvent::builder`].
#[derive(Clone, Debug)]
pub struct AuditEventBuilder {
    action: AuditAction,
    result: AuditResult,
    severity: Severity,
    actor: AuditActor,
    target: Option<AuditTarget>,
    ip: Option<IpAddr>,
    user_agent: Option<String>,
    request_id: Uuid,
    details: Map<String, Value>,
}

impl AuditEventBuilder {
    pub fn target(
        mut self,
        target_type: TargetType,
        id: Option<Uuid>,
        label: Option<String>,
    ) -> Self {
        self.target = Some(AuditTarget {
            target_type,
            id,
            label,
        });
        self
    }

    pub fn severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    pub fn ip(mut self, ip: IpAddr) -> Self {
        self.ip = Some(ip);
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(
            user_agent
                .into()
                .chars()
                .take(MAX_USER_AGENT_CHARS)
                .collect(),
        );
        self
    }

    pub fn detail(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    pub fn details(mut self, details: Map<String, Value>) -> Self {
        self.details = details;
        self
    }

    pub fn build(self) -> Result<AuditEvent, AuditEventError> {
        let actor_label = self.actor.label();
        if actor_label.trim().is_empty() || actor_label.chars().count() > MAX_ACTOR_LABEL_CHARS {
            return Err(AuditEventError::InvalidActorLabel);
        }
        if self.request_id.is_nil() {
            return Err(AuditEventError::NilRequestId);
        }
        let target = self.target.ok_or(AuditEventError::MissingTarget)?;
        if !self.action.target_rule().allows(target.target_type) {
            return Err(AuditEventError::InvalidTargetType(target.target_type));
        }
        if target.target_type != TargetType::System && target.id.is_none() {
            return Err(AuditEventError::MissingTargetId(target.target_type));
        }
        if target
            .label
            .as_deref()
            .is_some_and(|label| label.trim().is_empty())
        {
            return Err(AuditEventError::EmptyTargetLabel);
        }
        if !self.action.result_rule().allows(self.result) {
            return Err(AuditEventError::InvalidResult(self.result));
        }
        if !self.action.severity_rule().allows(self.severity) {
            return Err(AuditEventError::InvalidSeverity(self.severity));
        }
        if contains_forbidden_detail_key(&self.details) {
            return Err(AuditEventError::ForbiddenDetailKey);
        }

        Ok(AuditEvent {
            action: self.action,
            result: self.result,
            severity: self.severity,
            actor: self.actor,
            target,
            ip: self.ip,
            user_agent: self.user_agent,
            request_id: self.request_id,
            details: self.details,
        })
    }
}

/// Validation failures for audit events. Messages never include field values.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuditEventError {
    #[error("audit actor label must be non-empty and at most {MAX_ACTOR_LABEL_CHARS} characters")]
    InvalidActorLabel,
    #[error("audit request id must not be nil")]
    NilRequestId,
    #[error("audit action requires an explicit target type")]
    MissingTarget,
    #[error("target type {0} is not valid for this audit action")]
    InvalidTargetType(TargetType),
    #[error("target id is required for target type {0}")]
    MissingTargetId(TargetType),
    #[error("target label must not be empty")]
    EmptyTargetLabel,
    #[error("result {0} is not valid for this audit action")]
    InvalidResult(AuditResult),
    #[error("severity {0} is not valid for this audit action")]
    InvalidSeverity(Severity),
    #[error("audit details contain a forbidden secret key")]
    ForbiddenDetailKey,
}

fn contains_forbidden_detail_key(details: &Map<String, Value>) -> bool {
    details.iter().any(|(key, value)| {
        FORBIDDEN_DETAIL_KEYS
            .iter()
            .any(|forbidden| key.eq_ignore_ascii_case(forbidden))
            || contains_forbidden_key(value)
    })
}

fn contains_forbidden_key(value: &Value) -> bool {
    match value {
        Value::Object(map) => contains_forbidden_detail_key(map),
        Value::Array(items) => items.iter().any(contains_forbidden_key),
        _ => false,
    }
}
