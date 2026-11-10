//! Alertmanager webhook adapter (ADR-0008 §6.7).
//!
//! Alertmanager delivers critical alerts immediately and warning alerts to a
//! separate receiver. This process renders a small, fixed notification (no
//! arbitrary labels or annotations) and forwards it to a Feishu, DingTalk,
//! WeCom or generic chat webhook. Warnings are buffered and sent as one daily
//! digest; resolved warnings stay in the buffer until the next digest so a
//! short-lived warning is still reported.
//!
//! The target URL is a secret and is never logged or included in responses.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Timelike, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Default UTC hour for the daily warning digest.
pub const DEFAULT_DIGEST_HOUR_UTC: u8 = 9;
/// Default runbook location used when a rule omits its annotation.
pub const DEFAULT_RUNBOOK_BASE_URL: &str =
    "https://github.com/nasa-du/rs-admin-ds/blob/main/docs/runbooks/alerting.md";
const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:8081";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Chat webhook formats the adapter can emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebhookKind {
    Feishu,
    DingTalk,
    WeCom,
    Generic,
}

impl WebhookKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "feishu" | "lark" => Some(Self::Feishu),
            "dingtalk" | "ding" => Some(Self::DingTalk),
            "wecom" | "wechat-work" | "qywx" => Some(Self::WeCom),
            "generic" | "json" => Some(Self::Generic),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Feishu => "feishu",
            Self::DingTalk => "dingtalk",
            Self::WeCom => "wecom",
            Self::Generic => "generic",
        }
    }
}

/// Adapter configuration. Only non-secret values are represented as strings;
/// the delivery URL is optional and lives in memory but is never logged.
#[derive(Clone)]
pub struct WebhookConfig {
    pub listen_addr: SocketAddr,
    pub environment: String,
    target_url: Option<SecretString>,
    pub kind: WebhookKind,
    pub digest_hour_utc: u8,
    pub runbook_base_url: String,
    pub request_timeout: Duration,
}

impl std::fmt::Debug for WebhookConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebhookConfig")
            .field("listen_addr", &self.listen_addr)
            .field("environment", &self.environment)
            .field(
                "target_url",
                &self.target_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("kind", &self.kind)
            .field("digest_hour_utc", &self.digest_hour_utc)
            .field("runbook_base_url", &self.runbook_base_url)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WebhookConfigError {
    #[error("ALERT_WEBHOOK_LISTEN_ADDR is invalid")]
    ListenAddr,
    #[error("ALERT_WEBHOOK_KIND is invalid")]
    Kind,
    #[error("ALERT_DIGEST_HOUR_UTC must be between 0 and 23")]
    DigestHour,
    #[error("ALERT_WEBHOOK_URL is invalid")]
    TargetUrl,
    #[error("ALERT_WEBHOOK_URL_FILE cannot be read")]
    TargetUrlFile,
}

impl WebhookConfig {
    /// Read configuration from the process environment. A URL file wins over
    /// the plain environment variable; an empty file disables delivery.
    pub fn from_env() -> Result<Self, WebhookConfigError> {
        let mut config = Self::from_source(&|name| std::env::var(name).ok())?;

        if let Ok(path) = std::env::var("ALERT_WEBHOOK_URL_FILE") {
            let contents =
                std::fs::read_to_string(path).map_err(|_| WebhookConfigError::TargetUrlFile)?;
            config.target_url = normalize_url(contents.trim())?;
        }
        Ok(config)
    }

    /// Build configuration from an arbitrary lookup. Intended for tests and
    /// for the file-free development path.
    pub fn from_source<F>(lookup: &F) -> Result<Self, WebhookConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let listen_addr = lookup("ALERT_WEBHOOK_LISTEN_ADDR")
            .unwrap_or_else(|| DEFAULT_LISTEN_ADDR.to_owned())
            .parse()
            .map_err(|_| WebhookConfigError::ListenAddr)?;

        let environment = lookup("ALERT_ENVIRONMENT")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "production".to_owned());

        let kind = match lookup("ALERT_WEBHOOK_KIND") {
            Some(value) => WebhookKind::parse(&value).ok_or(WebhookConfigError::Kind)?,
            None => WebhookKind::Feishu,
        };

        let digest_hour_utc = match lookup("ALERT_DIGEST_HOUR_UTC") {
            Some(value) => value
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|hour| *hour <= 23)
                .ok_or(WebhookConfigError::DigestHour)?,
            None => DEFAULT_DIGEST_HOUR_UTC,
        };

        let runbook_base_url = lookup("ALERT_RUNBOOK_BASE_URL")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_RUNBOOK_BASE_URL.to_owned());

        let target_url = normalize_url(lookup("ALERT_WEBHOOK_URL").unwrap_or_default().trim())?;

        Ok(Self {
            listen_addr,
            environment,
            target_url,
            kind,
            digest_hour_utc,
            runbook_base_url,
            request_timeout: REQUEST_TIMEOUT,
        })
    }
}

fn normalize_url(value: &str) -> Result<Option<SecretString>, WebhookConfigError> {
    if value.is_empty() {
        return Ok(None);
    }
    let parsed = reqwest::Url::parse(value).map_err(|_| WebhookConfigError::TargetUrl)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(WebhookConfigError::TargetUrl);
    }
    Ok(Some(SecretString::from(value.to_owned())))
}

/// Alertmanager webhook payload (v4). Unknown fields are ignored so an
/// Alertmanager upgrade does not break delivery.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AlertmanagerPayload {
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "groupKey")]
    pub group_key: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub receiver: String,
    #[serde(default, rename = "groupLabels")]
    pub group_labels: BTreeMap<String, String>,
    #[serde(default, rename = "commonLabels")]
    pub common_labels: BTreeMap<String, String>,
    #[serde(default, rename = "commonAnnotations")]
    pub common_annotations: BTreeMap<String, String>,
    #[serde(default, rename = "externalURL")]
    pub external_url: String,
    #[serde(default)]
    pub alerts: Vec<AlertmanagerAlert>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AlertmanagerAlert {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
    #[serde(default, rename = "startsAt")]
    pub starts_at: String,
    #[serde(default, rename = "endsAt")]
    pub ends_at: String,
    #[serde(default, rename = "generatorURL")]
    pub generator_url: String,
    #[serde(default)]
    pub fingerprint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Critical,
    Warning,
}

impl Severity {
    fn from_label(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "critical" => Some(Self::Critical),
            "warning" => Some(Self::Warning),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
        }
    }
}

/// Alertmanager's `status` field, reduced to the two states the digest cares
/// about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationStatus {
    Firing,
    Resolved,
}

impl NotificationStatus {
    fn from_label(value: &str) -> Self {
        if value.trim().eq_ignore_ascii_case("resolved") {
            Self::Resolved
        } else {
            Self::Firing
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Firing => "firing",
            Self::Resolved => "resolved",
        }
    }
}

/// A rendered notification. Only the fields named here may reach the chat
/// webhook; arbitrary Alertmanager labels and annotations are never copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub severity: Severity,
    pub alertname: String,
    pub instance: String,
    pub fingerprint: String,
    pub status: NotificationStatus,
    pub summary: String,
    pub started: String,
    pub runbook: String,
    pub text: String,
}

impl Notification {
    fn key(&self) -> String {
        if self.fingerprint.is_empty() {
            format!("{}|{}", self.alertname, self.instance)
        } else {
            self.fingerprint.clone()
        }
    }
}

/// Render one alert into the fixed notification shape. Alerts without an
/// `alertname`, or with a severity outside the closed set, are ignored.
pub fn render_alert(config: &WebhookConfig, alert: &AlertmanagerAlert) -> Option<Notification> {
    let alertname = alert.labels.get("alertname")?.trim();
    if alertname.is_empty() {
        return None;
    }
    let severity = Severity::from_label(alert.labels.get("severity")?)?;
    let status = NotificationStatus::from_label(&alert.status);
    let instance = alert
        .labels
        .get("instance")
        .map(String::as_str)
        .unwrap_or("-");
    let summary = alert
        .annotations
        .get("summary")
        .map(String::as_str)
        .unwrap_or("");
    let runbook = alert
        .annotations
        .get("runbook_url")
        .map(String::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}#{}", config.runbook_base_url, anchor(alertname)));
    let started = if alert.starts_at.trim().is_empty() {
        "-"
    } else {
        alert.starts_at.trim()
    };

    let state = status.as_str().to_ascii_uppercase();
    let text = format!(
        "[{}] {} {}: {}\nInstance: {}\nStarted: {}\nSummary: {}\nRunbook: {}",
        config.environment,
        severity.as_str().to_ascii_uppercase(),
        state,
        alertname,
        instance,
        started,
        summary,
        runbook
    );

    Some(Notification {
        severity,
        alertname: alertname.to_owned(),
        instance: instance.to_owned(),
        fingerprint: alert.fingerprint.trim().to_owned(),
        status,
        summary: summary.to_owned(),
        started: started.to_owned(),
        runbook,
        text,
    })
}

fn anchor(alertname: &str) -> String {
    alertname
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Active warnings awaiting the daily digest, keyed by fingerprint so a
/// recurring alert is not listed twice.
#[derive(Debug, Default)]
pub struct DigestBuffer {
    entries: Mutex<BTreeMap<String, Notification>>,
}

impl DigestBuffer {
    fn apply(&self, notification: Notification) {
        let mut entries = self.entries.lock().expect("digest buffer poisoned");
        // Resolved alerts are retained until the next flush: a warning that
        // fires and resolves between two digests must still be reported.
        entries.insert(notification.key(), notification);
    }

    fn drain(&self) -> Vec<Notification> {
        let mut entries = self.entries.lock().expect("digest buffer poisoned");
        std::mem::take(&mut *entries).into_values().collect()
    }

    fn len(&self) -> usize {
        self.entries.lock().expect("digest buffer poisoned").len()
    }
}

fn render_digest(config: &WebhookConfig, notifications: &[Notification]) -> String {
    let mut text = format!(
        "[{}] Daily warning digest: {} warning(s) since the last digest",
        config.environment,
        notifications.len()
    );
    for notification in notifications {
        text.push_str(&format!(
            "\n- {} | {} | {} since {}\n  {}\n  Runbook: {}",
            notification.alertname,
            notification.instance,
            notification.status.as_str().to_ascii_uppercase(),
            notification.started,
            notification.summary,
            notification.runbook
        ));
    }
    text
}

fn payload(kind: WebhookKind, text: &str) -> serde_json::Value {
    match kind {
        WebhookKind::Feishu => serde_json::json!({
            "msg_type": "text",
            "content": { "text": text }
        }),
        WebhookKind::DingTalk | WebhookKind::WeCom => serde_json::json!({
            "msgtype": "text",
            "text": { "content": text }
        }),
        WebhookKind::Generic => serde_json::json!({
            "source": "rs-admin",
            "text": text
        }),
    }
}

/// Delivery failures intentionally do not retain the `reqwest::Error`: its
/// display text can contain the secret webhook URL, which must never reach a
/// log line.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("webhook delivery failed")]
    Http,
    #[error("webhook returned HTTP {0}")]
    Status(StatusCode),
}

impl From<reqwest::Error> for DeliveryError {
    fn from(_: reqwest::Error) -> Self {
        Self::Http
    }
}

#[derive(Clone)]
pub struct WebhookClient {
    http: reqwest::Client,
    target_url: Option<SecretString>,
    kind: WebhookKind,
}

impl WebhookClient {
    pub fn new(config: &WebhookConfig) -> Result<Self, WebhookConfigError> {
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|_| WebhookConfigError::TargetUrl)?;
        Ok(Self {
            http,
            target_url: config.target_url.clone(),
            kind: config.kind,
        })
    }

    /// Deliver one chat message. An absent target URL is a deliberate disabled
    /// mode and succeeds without sending.
    pub async fn send(&self, text: &str) -> Result<(), DeliveryError> {
        let Some(url) = &self.target_url else {
            return Ok(());
        };
        let response = self
            .http
            .post(url.expose_secret())
            .json(&payload(self.kind, text))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(DeliveryError::Status(response.status()));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct WebhookState {
    config: Arc<WebhookConfig>,
    client: WebhookClient,
    digest: Arc<DigestBuffer>,
}

impl WebhookState {
    pub fn new(config: WebhookConfig) -> Result<Self, WebhookConfigError> {
        let client = WebhookClient::new(&config)?;
        Ok(Self {
            config: Arc::new(config),
            client,
            digest: Arc::new(DigestBuffer::default()),
        })
    }

    pub fn config(&self) -> &WebhookConfig {
        &self.config
    }
}

pub fn build_router(state: WebhookState) -> Router {
    Router::new()
        .route("/health/live", get(health))
        .route("/alertmanager/critical", post(ingest_critical))
        .route("/alertmanager/warning", post(ingest_warning))
        .with_state(state)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn ingest_critical(
    State(state): State<WebhookState>,
    Json(payload): Json<AlertmanagerPayload>,
) -> Result<StatusCode, StatusCode> {
    let mut delivered = 0usize;
    for alert in &payload.alerts {
        let Some(notification) = render_alert(&state.config, alert) else {
            continue;
        };
        if notification.severity != Severity::Critical {
            continue;
        }
        state
            .client
            .send(&notification.text)
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "critical alert delivery failed");
                StatusCode::BAD_GATEWAY
            })?;
        delivered += 1;
    }
    tracing::info!(alerts = delivered, "critical alert batch processed");
    Ok(StatusCode::OK)
}

async fn ingest_warning(
    State(state): State<WebhookState>,
    Json(payload): Json<AlertmanagerPayload>,
) -> StatusCode {
    for alert in &payload.alerts {
        let Some(notification) = render_alert(&state.config, alert) else {
            continue;
        };
        if notification.severity != Severity::Warning {
            continue;
        }
        state.digest.apply(notification);
    }
    tracing::info!(
        pending = state.digest.len(),
        "warning alert batch buffered for the daily digest"
    );
    StatusCode::ACCEPTED
}

async fn flush_digest(state: &WebhookState) -> Result<(), DeliveryError> {
    let notifications = state.digest.drain();
    if notifications.is_empty() {
        return Ok(());
    }
    let text = render_digest(&state.config, &notifications);
    match state.client.send(&text).await {
        Ok(()) => Ok(()),
        Err(error) => {
            // Keep the warnings for the next scheduler tick instead of losing
            // a day of warnings to a transient webhook failure.
            for notification in notifications {
                state.digest.apply(notification);
            }
            Err(error)
        }
    }
}

/// Flush buffered warnings at the configured UTC hour, once per day.
pub fn spawn_digest_scheduler(state: WebhookState) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_flush: Option<chrono::NaiveDate> = None;
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let now = Utc::now();
            let today = now.date_naive();
            if now.hour() == u32::from(state.config.digest_hour_utc) && last_flush != Some(today) {
                match flush_digest(&state).await {
                    Ok(()) => last_flush = Some(today),
                    Err(error) => {
                        tracing::error!(error = %error, "warning digest delivery failed; retrying");
                    }
                }
            }
        }
    })
}

pub async fn run() -> anyhow::Result<()> {
    let config = WebhookConfig::from_env()?;
    if config.target_url.is_none() {
        tracing::warn!("ALERT_WEBHOOK_URL is empty; alerts are accepted but not delivered");
    }
    tracing::info!(
        kind = config.kind.as_str(),
        environment = %config.environment,
        listen = %config.listen_addr,
        "starting alert webhook adapter"
    );

    let state = WebhookState::new(config)?;
    let listener = TcpListener::bind(state.config.listen_addr).await?;
    let scheduler = spawn_digest_scheduler(state.clone());

    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(crate::shutdown_signal())
        .await?;
    scheduler.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::State as TestState;
    use axum::http::{Request, header};
    use axum::routing::post as test_post;
    use std::net::SocketAddr;
    use tower::ServiceExt;

    fn config(target_url: Option<String>) -> WebhookConfig {
        WebhookConfig {
            listen_addr: "127.0.0.1:0".parse().expect("addr"),
            environment: "production".to_owned(),
            target_url: target_url.map(SecretString::from),
            kind: WebhookKind::Feishu,
            digest_hour_utc: 9,
            runbook_base_url: DEFAULT_RUNBOOK_BASE_URL.to_owned(),
            request_timeout: Duration::from_secs(5),
        }
    }

    fn alert(
        alertname: &str,
        severity: &str,
        status: &str,
        fingerprint: &str,
        extra_labels: &[(&str, &str)],
    ) -> AlertmanagerAlert {
        let mut labels = BTreeMap::from([
            ("alertname".to_owned(), alertname.to_owned()),
            ("severity".to_owned(), severity.to_owned()),
            ("instance".to_owned(), "api:9100".to_owned()),
        ]);
        for (key, value) in extra_labels {
            labels.insert((*key).to_owned(), (*value).to_owned());
        }
        AlertmanagerAlert {
            status: status.to_owned(),
            labels,
            annotations: BTreeMap::from([
                ("summary".to_owned(), "API is down".to_owned()),
                (
                    "runbook_url".to_owned(),
                    "https://example.test/runbook#apidown".to_owned(),
                ),
            ]),
            starts_at: "2026-11-10T12:00:00Z".to_owned(),
            fingerprint: fingerprint.to_owned(),
            ..AlertmanagerAlert::default()
        }
    }

    #[test]
    fn critical_notification_is_fixed_and_excludes_identifying_labels() {
        let alert = alert(
            "ApiDown",
            "critical",
            "firing",
            "fp-1",
            &[
                ("username", "alice"),
                ("email", "alice@example.com"),
                ("ip", "203.0.113.7"),
                ("request_id", "request-secret"),
            ],
        );

        let notification = render_alert(&config(None), &alert).expect("notification");

        assert_eq!(notification.severity, Severity::Critical);
        assert!(
            notification
                .text
                .contains("[production] CRITICAL FIRING: ApiDown")
        );
        assert!(notification.text.contains("Instance: api:9100"));
        assert!(notification.text.contains("Started: 2026-11-10T12:00:00Z"));
        assert!(
            notification
                .text
                .contains("Runbook: https://example.test/runbook#apidown")
        );
        for forbidden in [
            "alice",
            "alice@example.com",
            "203.0.113.7",
            "request-secret",
        ] {
            assert!(
                !notification.text.contains(forbidden),
                "notification leaked {forbidden}: {}",
                notification.text
            );
        }
    }

    #[test]
    fn alerts_without_a_known_severity_are_ignored() {
        let unknown = alert("ApiDown", "notice", "firing", "fp-1", &[]);
        assert!(render_alert(&config(None), &unknown).is_none());
    }

    #[test]
    fn digest_deduplicates_and_keeps_resolved_alerts_until_flush() {
        let buffer = DigestBuffer::default();
        let first = render_alert(
            &config(None),
            &alert("Warn", "warning", "firing", "fp-1", &[]),
        )
        .expect("notification");
        buffer.apply(first.clone());
        buffer.apply(first);
        assert_eq!(buffer.len(), 1, "same fingerprint must not duplicate");

        let resolved = render_alert(
            &config(None),
            &alert("Warn", "warning", "resolved", "fp-1", &[]),
        )
        .expect("notification");
        buffer.apply(resolved);
        assert_eq!(
            buffer.len(),
            1,
            "a short-lived warning must survive until the digest"
        );
        let entries = buffer.drain();
        assert_eq!(entries[0].status, NotificationStatus::Resolved);
    }

    #[tokio::test]
    async fn failed_digest_delivery_keeps_warnings_for_the_next_retry() {
        let state =
            WebhookState::new(config(Some("http://127.0.0.1:1/hook".to_owned()))).expect("state");
        let warning = render_alert(
            &config(None),
            &alert("Warn", "warning", "firing", "fp-retry", &[]),
        )
        .expect("notification");
        state.digest.apply(warning);

        assert!(flush_digest(&state).await.is_err());
        assert_eq!(
            state.digest.len(),
            1,
            "a transient failure must not drop the daily digest"
        );
    }

    #[test]
    fn provider_payloads_have_the_expected_shape() {
        let feishu = payload(WebhookKind::Feishu, "hello");
        assert_eq!(feishu["msg_type"], "text");
        assert_eq!(feishu["content"]["text"], "hello");

        for kind in [WebhookKind::DingTalk, WebhookKind::WeCom] {
            let body = payload(kind, "hello");
            assert_eq!(body["msgtype"], "text");
            assert_eq!(body["text"]["content"], "hello");
        }

        let generic = payload(WebhookKind::Generic, "hello");
        assert_eq!(generic["source"], "rs-admin");
        assert_eq!(generic["text"], "hello");
    }

    async fn post_json(app: Router, uri: &str, body: &AlertmanagerPayload) -> StatusCode {
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(body).expect("serialize payload"),
            ))
            .expect("request");
        app.oneshot(request).await.expect("response").status()
    }

    #[tokio::test]
    async fn critical_alerts_send_immediately_and_warnings_wait_for_the_digest() {
        let captured: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let sink = captured.clone();
        let mock = Router::new()
            .route(
                "/hook",
                test_post(
                    |TestState(sink): TestState<Arc<Mutex<Vec<serde_json::Value>>>>,
                     Json(body): Json<serde_json::Value>| async move {
                        sink.lock().expect("sink").push(body);
                        StatusCode::OK
                    },
                ),
            )
            .with_state(sink);
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });

        let state =
            WebhookState::new(config(Some(format!("http://{address}/hook")))).expect("state");
        let app = build_router(state.clone());

        let critical = AlertmanagerPayload {
            status: "firing".to_owned(),
            alerts: vec![alert("ApiDown", "critical", "firing", "fp-critical", &[])],
            ..AlertmanagerPayload::default()
        };
        assert_eq!(
            post_json(app.clone(), "/alertmanager/critical", &critical).await,
            StatusCode::OK
        );
        assert_eq!(captured.lock().expect("sink").len(), 1);
        let delivered = captured.lock().expect("sink")[0].clone();
        assert!(
            delivered["content"]["text"]
                .as_str()
                .expect("text")
                .contains("CRITICAL FIRING: ApiDown")
        );

        let warning = AlertmanagerPayload {
            status: "firing".to_owned(),
            alerts: vec![alert(
                "LoginLockoutSpike",
                "warning",
                "firing",
                "fp-warning",
                &[],
            )],
            ..AlertmanagerPayload::default()
        };
        assert_eq!(
            post_json(app.clone(), "/alertmanager/warning", &warning).await,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            captured.lock().expect("sink").len(),
            1,
            "warnings must not be delivered immediately"
        );

        flush_digest(&state).await.expect("digest delivery");
        let bodies = captured.lock().expect("sink");
        assert_eq!(bodies.len(), 2);
        let digest = bodies[1]["content"]["text"].as_str().expect("text");
        assert!(digest.contains("Daily warning digest: 1 warning(s) since the last digest"));
        assert!(digest.contains("LoginLockoutSpike"));
        assert!(digest.contains("Runbook: https://example.test/runbook#apidown"));

        server.abort();
    }

    #[test]
    fn empty_target_url_is_a_disabled_delivery_mode() {
        let state = WebhookState::new(config(None)).expect("state");
        assert!(state.config().target_url.is_none());
    }

    #[test]
    fn config_debug_redacts_the_secret_webhook_url() {
        let config = config(Some("https://hooks.example.test/secret-token".to_owned()));
        let debug = format!("{config:?}");
        assert!(!debug.contains("hooks.example.test"));
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn listen_address_is_read_from_the_environment() {
        let config = WebhookConfig::from_source(&|name| {
            if name == "ALERT_WEBHOOK_LISTEN_ADDR" {
                Some("127.0.0.1:9999".to_owned())
            } else {
                None
            }
        })
        .expect("config");
        assert_eq!(
            config.listen_addr,
            "127.0.0.1:9999".parse::<SocketAddr>().expect("addr")
        );
    }
}
