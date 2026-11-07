mod common;

use axum::body::Body;
use axum::http::Request;
use common::test_state;
use rs_admin_api::{build_env_filter, build_router};
use serde_json::Value;
use std::io;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("log lock")).into_owned()
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = CapturedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CapturedWriter(self.0.clone())
    }
}

struct CapturedWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for CapturedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log lock").extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn request_logs_are_structured_and_redacted() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .with_writer(captured.clone())
        .with_current_span(true)
        .with_span_list(false)
        .flatten_event(true)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let request = Request::builder()
        .method("GET")
        .uri("/health/live?token=querysecret")
        .header("authorization", "Bearer headersupersecret")
        .header("cookie", "__Host-refresh=cookiesecret")
        .body(Body::empty())
        .expect("request");

    let response = build_router(test_state())
        .oneshot(request)
        .await
        .expect("response");
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("request id")
        .to_owned();
    drop(guard);

    let logs = captured.contents();
    assert!(!logs.is_empty(), "no logs captured");

    for secret in [
        "querysecret",
        "headersupersecret",
        "cookiesecret",
        "supersecret",
        "authorization",
        "cookie",
    ] {
        assert!(!logs.contains(secret), "log leaked {secret}: {logs}");
    }

    let line = logs
        .lines()
        .find(|line| line.contains("request completed"))
        .expect("request completion log");
    let value: Value = serde_json::from_str(line).expect("json log line");

    assert_eq!(value["request_id"], request_id);
    assert_eq!(value["method"], "GET");
    assert_eq!(value["matched_path"], "/health/live");
    assert_eq!(value["status"], 200);
    assert!(value["latency_ms"].is_u64());
    assert_eq!(value["error_code"], "");
    assert!(!logs.contains("token="), "query string was logged");
}

#[test]
fn invalid_log_filters_fail() {
    assert!(build_env_filter(Some("rs_admin_api=not_a_level")).is_err());
    assert!(build_env_filter(Some("=invalid")).is_err());
}

#[test]
fn absent_or_empty_filter_uses_the_default() {
    assert!(build_env_filter(None).is_ok());
    assert!(build_env_filter(Some("")).is_ok());
}
