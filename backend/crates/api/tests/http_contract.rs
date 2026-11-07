mod common;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::test_state;
use rs_admin_api::{build_router, with_common_middleware, with_common_middleware_and_timeout};
use serde_json::Value;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

async fn send(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
    let response = app.oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, headers, body)
}

fn request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

fn problem(body: &[u8]) -> Value {
    serde_json::from_slice(body).expect("problem json")
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .expect("header present")
}

#[tokio::test]
async fn live_is_ok_without_database_access() {
    let (status, headers, body) =
        send(build_router(test_state()), request("GET", "/health/live")).await;

    assert_eq!(status, StatusCode::OK);
    assert!(header(&headers, "content-type").starts_with("application/json"));
    assert_eq!(problem(&body)["status"], "live");
}

#[tokio::test]
async fn ready_reports_unready_without_leaking_connection_details() {
    let (status, headers, body) =
        send(build_router(test_state()), request("GET", "/health/ready")).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(header(&headers, "content-type").starts_with("application/json"));
    assert_eq!(problem(&body)["status"], "unready");

    let text = String::from_utf8_lossy(&body).to_ascii_lowercase();
    for forbidden in ["postgres", "supersecret", "connection refused", "sql"] {
        assert!(!text.contains(forbidden), "leaked {forbidden}: {text}");
    }
}

#[tokio::test]
async fn unknown_route_returns_consistent_problem_details() {
    let (status, headers, body) = send(
        build_router(test_state()),
        request("GET", "/api/v1/missing?token=supersecret"),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(header(&headers, "content-type"), "application/problem+json");

    let request_id = header(&headers, "x-request-id").to_owned();
    let payload = problem(&body);
    assert_eq!(payload["type"], "urn:rs-admin:problem:not_found");
    assert_eq!(payload["code"], "not_found");
    assert_eq!(payload["title"], "Not found");
    assert_eq!(payload["status"], 404);
    assert_eq!(payload["instance"], "/api/v1/missing");
    assert_eq!(payload["request_id"], request_id);
    assert!(payload["detail"].as_str().is_some_and(|d| !d.is_empty()));

    let text = String::from_utf8_lossy(&body);
    assert!(!text.contains("supersecret"));
    assert!(!text.to_ascii_lowercase().contains("sql"));
    assert!(!text.to_ascii_lowercase().contains("stack"));
}

#[tokio::test]
async fn unknown_method_returns_problem_details_and_allow_header() {
    let (status, headers, body) =
        send(build_router(test_state()), request("PUT", "/health/live")).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header(&headers, "content-type"), "application/problem+json");
    assert!(
        header(&headers, "allow").contains("GET"),
        "allow header missing GET"
    );

    let payload = problem(&body);
    assert_eq!(payload["code"], "method_not_allowed");
    assert_eq!(payload["status"], 405);
    assert_eq!(payload["instance"], "/health/live");
    assert_eq!(payload["request_id"], header(&headers, "x-request-id"));
}

#[tokio::test]
async fn request_id_is_server_generated_uuid_v7_and_ignores_client_value() {
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/missing")
        .header("x-request-id", "client-controlled")
        .body(Body::empty())
        .expect("request");

    let (status, headers, body) = send(build_router(test_state()), request).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    let request_id = header(&headers, "x-request-id");
    assert_ne!(request_id, "client-controlled");
    let uuid = Uuid::parse_str(request_id).expect("uuid request id");
    assert_eq!(uuid.get_version_num(), 7);
    assert_eq!(problem(&body)["request_id"], request_id);
}

#[tokio::test]
async fn security_headers_are_present_on_success_and_error() {
    for uri in ["/health/live", "/api/v1/missing"] {
        let (_status, headers, _body) = send(build_router(test_state()), request("GET", uri)).await;

        assert_eq!(header(&headers, "cache-control"), "no-store", "{uri}");
        assert_eq!(
            header(&headers, "x-content-type-options"),
            "nosniff",
            "{uri}"
        );
        assert_eq!(header(&headers, "referrer-policy"), "no-referrer", "{uri}");
        assert_eq!(header(&headers, "x-frame-options"), "DENY", "{uri}");
        assert_eq!(
            header(&headers, "content-security-policy"),
            "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
            "{uri}"
        );
        assert_eq!(
            header(&headers, "permissions-policy"),
            "camera=(), microphone=(), geolocation=()",
            "{uri}"
        );
    }
}

async fn echo_body(_body: Bytes) -> &'static str {
    "ok"
}

async fn echo_json(Json(value): Json<Value>) -> Json<Value> {
    Json(value)
}

fn test_router() -> Router {
    with_common_middleware(
        Router::new()
            .route("/echo", post(echo_body))
            .route("/echo-json", post(echo_json))
            .with_state(()),
    )
}

#[tokio::test]
async fn oversized_body_returns_problem_payload_too_large() {
    let body = vec![b'a'; 70 * 1024];
    let request = Request::builder()
        .method("POST")
        .uri("/echo")
        .body(Body::from(body))
        .expect("request");

    let (status, headers, body) = send(test_router(), request).await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(header(&headers, "content-type"), "application/problem+json");
    assert_eq!(problem(&body)["code"], "payload_too_large");
    assert_eq!(
        problem(&body)["request_id"],
        header(&headers, "x-request-id")
    );
}

async fn slow_handler() -> &'static str {
    tokio::time::sleep(Duration::from_millis(100)).await;
    "late"
}

#[tokio::test]
async fn request_timeout_returns_service_unavailable_problem() {
    let app = with_common_middleware_and_timeout(
        Router::new()
            .route("/slow", get(slow_handler))
            .with_state(()),
        Duration::from_millis(10),
    );

    let (status, headers, body) = send(app, request("GET", "/slow")).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(header(&headers, "content-type"), "application/problem+json");
    assert_eq!(problem(&body)["code"], "service_unavailable");
    assert_eq!(
        problem(&body)["request_id"],
        header(&headers, "x-request-id")
    );
}

#[tokio::test]
async fn body_under_limit_is_accepted() {
    let request = Request::builder()
        .method("POST")
        .uri("/echo")
        .body(Body::from("hello"))
        .expect("request");

    let (status, _headers, body) = send(test_router(), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn unsupported_media_type_and_invalid_json_use_problem_details() {
    let unsupported = Request::builder()
        .method("POST")
        .uri("/echo-json")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(r#"{"ok":true}"#))
        .expect("request");
    let (status, headers, body) = send(test_router(), unsupported).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(header(&headers, "content-type"), "application/problem+json");
    assert_eq!(problem(&body)["code"], "unsupported_media_type");

    let invalid = Request::builder()
        .method("POST")
        .uri("/echo-json")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not json"))
        .expect("request");
    let (status, headers, body) = send(test_router(), invalid).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(header(&headers, "content-type"), "application/problem+json");
    assert_eq!(problem(&body)["code"], "invalid_json");
    assert_eq!(
        problem(&body)["request_id"],
        header(&headers, "x-request-id")
    );
}

#[tokio::test]
async fn valid_json_is_accepted() {
    let request = Request::builder()
        .method("POST")
        .uri("/echo-json")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"ok":true}"#))
        .expect("request");

    let (status, _headers, body) = send(test_router(), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(problem(&body)["ok"], true);
}

#[tokio::test]
async fn every_response_carries_a_request_id_header() {
    let (_, headers, _) = send(build_router(test_state()), request("GET", "/health/live")).await;

    let request_id = header(&headers, "x-request-id");
    assert_eq!(
        Uuid::parse_str(request_id).expect("uuid").get_version_num(),
        7
    );
}
