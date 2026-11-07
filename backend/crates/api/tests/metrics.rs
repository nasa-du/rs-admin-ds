mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::test_state;
use metrics_exporter_prometheus::PrometheusHandle;
use rs_admin_api::{build_router, install_metrics_recorder, metrics_router};
use std::sync::OnceLock;
use tower::ServiceExt;

fn recorder() -> &'static PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| install_metrics_recorder().expect("install recorder"))
}

async fn get(app: axum::Router, uri: &str) -> StatusCode {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    app.oneshot(request).await.expect("response").status()
}

#[tokio::test]
async fn http_metrics_use_matched_paths_and_bounded_labels() {
    let handle = recorder();
    let app = build_router(test_state());

    assert_eq!(get(app.clone(), "/health/live").await, StatusCode::OK);

    let rendered = handle.render();
    assert!(rendered.contains("http_requests_total"));
    assert!(rendered.contains("route=\"/health/live\""));
    assert!(rendered.contains("status=\"200\""));
    assert!(rendered.contains("http_request_duration_seconds_bucket"));

    let uuid = "018f0000-0000-7000-8000-000000000000";
    assert_eq!(
        get(app.clone(), &format!("/api/v1/{uuid}")).await,
        StatusCode::NOT_FOUND
    );

    let secret_request = Request::builder()
        .method("GET")
        .uri("/api/v1/missing?token=metricsecret")
        .header("authorization", "Bearer metricsecret")
        .header("cookie", "__Host-refresh=metricsecret")
        .body(Body::empty())
        .expect("request");
    assert_eq!(
        app.oneshot(secret_request)
            .await
            .expect("response")
            .status(),
        StatusCode::NOT_FOUND
    );

    let rendered = handle.render();
    assert!(rendered.contains("route=\"unmatched\""));
    assert!(
        !rendered.contains(uuid),
        "raw paths must not become metric labels"
    );
    assert!(
        !rendered.contains("metricsecret"),
        "secrets and headers must not become metric labels"
    );
}

#[tokio::test]
async fn metrics_listener_only_serves_metrics() {
    let handle = recorder();
    metrics::counter!(
        "http_requests_total",
        "method" => "GET",
        "route" => "/health/live",
        "status" => "200"
    )
    .increment(1);
    let app = metrics_router(handle.clone());

    let request = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .expect("content type");
    assert!(content_type.starts_with("text/plain"));

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert!(String::from_utf8_lossy(&body).contains("http_requests_total"));

    assert_eq!(get(app, "/health/live").await, StatusCode::NOT_FOUND);
}
