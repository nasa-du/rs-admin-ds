//! Audit write failures must be visible to Prometheus (ADR-0008 §6.5).

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use rs_admin_audit::{AuditAction, AuditActor, AuditEvent, AuditResult, AuditWriter, TargetType};
use rs_admin_security::AuditKeyRing;
use sqlx::postgres::PgPoolOptions;
use std::sync::OnceLock;
use std::time::Duration;
use uuid::Uuid;

const KEY_B64: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
const KEY_ID: &str = "2026-11";

fn recorder() -> &'static PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .expect("install recorder")
    })
}

fn writer() -> AuditWriter {
    let keyring = AuditKeyRing::from_json(&format!("{{\"{KEY_ID}\":\"{KEY_B64}\"}}"), KEY_ID)
        .expect("keyring");
    AuditWriter::new(keyring)
}

fn login_fail() -> AuditEvent {
    AuditEvent::builder(
        AuditAction::AuthnLoginFail,
        AuditResult::Failure,
        AuditActor::anonymous(),
        Uuid::now_v7(),
    )
    .target(TargetType::System, None, None)
    .detail("reason_code", "bad_credentials")
    .ip("203.0.113.7".parse().expect("ip"))
    .build()
    .expect("event")
}

#[tokio::test]
async fn failed_audit_write_increments_the_failure_counter() {
    let handle = recorder();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy("postgres://rs_admin:secret@127.0.0.1:1/rs_admin")
        .expect("lazy pool");

    let result = writer().write(&pool, &login_fail()).await;

    assert!(result.is_err(), "the unreachable database must fail");
    let rendered = handle.render();
    assert!(
        rendered.contains("audit_write_failures_total"),
        "missing audit failure counter: {rendered}"
    );
}
