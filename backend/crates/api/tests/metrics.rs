mod auth_support;
mod common;

use auth_support::{NewUser, PASSWORD, app, insert_user, login};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::test_state;
use metrics_exporter_prometheus::PrometheusHandle;
use rs_admin_api::{
    build_router, install_metrics_recorder, metrics_router, record_db_pool_metrics,
};
use sqlx::PgPool;
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
async fn db_pool_metrics_report_active_idle_and_max() {
    let handle = recorder();
    let state = test_state();

    record_db_pool_metrics(state.database.pool(), 10);

    let rendered = handle.render();
    assert!(rendered.contains("db_pool_connections{state=\"active\"} 0"));
    assert!(rendered.contains("db_pool_connections{state=\"idle\"} 0"));
    assert!(rendered.contains("db_pool_connections{state=\"max\"} 10"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_metrics_record_outcomes_without_identifiers(pool: PgPool) {
    let handle = recorder();
    let app = app(&pool).await;
    let username = "metrics-alice";
    let email = "metrics-alice@example.com";
    insert_user(&pool, NewUser::active(username, email)).await;

    let (status, _, _) = login(&app, username, "wrong password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = login(&app, username, PASSWORD).await;
    assert_eq!(status, StatusCode::OK);

    let rendered = handle.render();
    assert!(
        rendered.contains("reason=\"bad_credentials\""),
        "missing bad_credentials login metric: {rendered}"
    );
    assert!(
        rendered.contains("result=\"failure\""),
        "missing failure login metric: {rendered}"
    );
    assert!(
        rendered.contains("reason=\"success\""),
        "missing success login metric: {rendered}"
    );
    assert!(
        rendered.contains("auth_password_hash_seconds_bucket"),
        "missing password hash histogram: {rendered}"
    );
    assert!(
        rendered.contains("auth_global_circuit_open"),
        "missing global circuit gauge: {rendered}"
    );
    assert!(
        !rendered.contains(username),
        "usernames must not become metric labels or values: {rendered}"
    );
    assert!(
        !rendered.contains(email),
        "emails must not become metric labels or values: {rendered}"
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
