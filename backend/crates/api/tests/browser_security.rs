//! Browser security boundary: CORS, CSRF, cookie attributes and response
//! headers (docs/specs/login-system.md §7.11, §8.6).

mod auth_support;

use auth_support::{
    NewUser, PASSWORD, TEST_ORIGIN, app, authorized_request, change_password_request, insert_user,
    json, login, logout_request, problem_code, refresh_cookie, refresh_request, send,
};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sqlx::PgPool;

fn header_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn assert_vary_origin(headers: &axum::http::HeaderMap, label: &str) {
    assert!(
        header_value(headers, "vary").is_some_and(|vary| vary
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case("origin"))),
        "{label}: Vary must include Origin"
    );
}

fn preflight(uri: &str, method: &str, origin: &str) -> Request<Body> {
    Request::builder()
        .method("OPTIONS")
        .uri(uri)
        .header(header::ORIGIN, origin)
        .header("access-control-request-method", method)
        .header(
            "access-control-request-headers",
            "authorization,content-type,x-requested-with",
        )
        .body(Body::empty())
        .expect("preflight request")
}

#[sqlx::test(migrations = "../../migrations")]
async fn preflight_from_an_allowed_origin_describes_the_cors_policy(pool: PgPool) {
    let app = app(&pool).await;

    for _ in 0..2 {
        let (status, headers, body) = send(
            app.clone(),
            preflight("/api/v1/auth/login", "POST", TEST_ORIGIN),
        )
        .await;

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty(), "a preflight response has no body");
        assert_eq!(
            header_value(&headers, "access-control-allow-origin"),
            Some(TEST_ORIGIN)
        );
        assert_eq!(
            header_value(&headers, "access-control-allow-credentials"),
            Some("true")
        );
        let methods =
            header_value(&headers, "access-control-allow-methods").expect("allow-methods");
        for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
            assert!(methods.contains(method), "missing {method}: {methods}");
        }
        let allowed_headers =
            header_value(&headers, "access-control-allow-headers").expect("allow-headers");
        for name in [
            "Authorization",
            "Content-Type",
            "X-Requested-With",
            "X-Request-Id",
        ] {
            assert!(
                allowed_headers
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case(name)),
                "missing {name}: {allowed_headers}"
            );
        }
        let exposed =
            header_value(&headers, "access-control-expose-headers").expect("expose-headers");
        for name in ["X-Request-Id", "Retry-After", "WWW-Authenticate"] {
            assert!(
                exposed
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case(name)),
                "missing {name}: {exposed}"
            );
        }
        assert_eq!(
            header_value(&headers, "access-control-max-age"),
            Some("600")
        );
        assert_vary_origin(&headers, "preflight");
        assert!(
            headers.get("x-request-id").is_some(),
            "preflight responses carry a request id"
        );
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn credentials_are_allowed_only_on_the_cookie_endpoints(pool: PgPool) {
    let app = app(&pool).await;

    for (uri, method) in [
        ("/api/v1/auth/login", "POST"),
        ("/api/v1/auth/refresh", "POST"),
        ("/api/v1/auth/logout", "POST"),
        ("/api/v1/auth/logout-all", "POST"),
        ("/api/v1/auth/change-password", "POST"),
    ] {
        let (_status, headers, _body) =
            send(app.clone(), preflight(uri, method, TEST_ORIGIN)).await;
        assert_eq!(
            header_value(&headers, "access-control-allow-credentials"),
            Some("true"),
            "{uri} may set the refresh cookie"
        );
    }

    for (uri, method) in [
        ("/api/v1/auth/me", "GET"),
        ("/api/v1/auth/reauthenticate", "POST"),
    ] {
        let (_status, headers, _body) =
            send(app.clone(), preflight(uri, method, TEST_ORIGIN)).await;
        assert_eq!(
            header_value(&headers, "access-control-allow-credentials"),
            None,
            "{uri} must not be credentialed"
        );
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn error_responses_keep_the_cors_headers(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    // 401 from a credentialed endpoint: the SPA must be able to read the
    // problem body, so origin and credentials survive the error rebuild.
    let (status, headers, body) = login(&app, "alice", "wrong password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&body), "invalid_credentials");
    assert_credentialed_cors(&headers, "login 401");

    // 403 csrf_failed with an allowed Origin: the response still explains
    // itself to the SPA.
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, TEST_ORIGIN)
        .body(Body::from(
            serde_json::json!({ "identifier": "alice", "password": PASSWORD }).to_string(),
        ))
        .expect("login request");
    let (status, headers, body) = send(app.clone(), request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&json(&body)), "csrf_failed");
    assert_credentialed_cors(&headers, "login 403 csrf");

    // 401 from a Bearer endpoint.
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(header::ORIGIN, TEST_ORIGIN)
        .body(Body::empty())
        .expect("request");
    let (status, headers, body) = send(app.clone(), request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "access_token_invalid");
    assert_eq!(
        header_value(&headers, "access-control-allow-origin"),
        Some(TEST_ORIGIN)
    );
    assert_eq!(
        header_value(&headers, "access-control-allow-credentials"),
        None,
        "a Bearer endpoint is not credentialed"
    );

    // 404 from the fallback.
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/missing")
        .header(header::ORIGIN, TEST_ORIGIN)
        .body(Body::empty())
        .expect("request");
    let (status, headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        header_value(&headers, "access-control-allow-origin"),
        Some(TEST_ORIGIN)
    );
    assert_vary_origin(&headers, "not found");
}

#[tokio::test]
async fn server_errors_keep_the_cors_headers() {
    use rs_admin_api::{AppState, build_router};
    use rs_admin_store::{Database, PoolSettings};

    let database = Database::connect_lazy(
        "postgres://rs_admin:supersecret@127.0.0.1:1/rs_admin",
        &PoolSettings::default(),
    )
    .expect("lazy pool");
    let app =
        build_router(AppState::new(database).with_allowed_origins(auth_support::allowed_origins()));
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(header::ORIGIN, TEST_ORIGIN)
        .body(Body::empty())
        .expect("request");

    let (status, headers, body) = send(app, request).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(problem_code(&json(&body)), "service_unavailable");
    assert_eq!(
        header_value(&headers, "access-control-allow-origin"),
        Some(TEST_ORIGIN)
    );
    assert_vary_origin(&headers, "service unavailable");
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_untrusted_origin_never_receives_cors_grants(pool: PgPool) {
    let app = app(&pool).await;

    for origin in ["null", "*", "https://evil.example.com"] {
        let (status, headers, _body) =
            send(app.clone(), preflight("/api/v1/auth/login", "POST", origin)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{origin}");
        assert_eq!(
            header_value(&headers, "access-control-allow-origin"),
            None,
            "{origin} must not be echoed"
        );
        assert_eq!(
            header_value(&headers, "access-control-allow-credentials"),
            None,
            "{origin} must not be granted credentials"
        );
        assert_vary_origin(&headers, origin);
    }
}

const NEW_PASSWORD: &str = "a completely different passphrase";

fn assert_credentialed_cors(headers: &axum::http::HeaderMap, label: &str) {
    assert_eq!(
        header_value(headers, "access-control-allow-origin"),
        Some(TEST_ORIGIN),
        "{label}"
    );
    assert_eq!(
        header_value(headers, "access-control-allow-credentials"),
        Some("true"),
        "{label}"
    );
    assert_vary_origin(headers, label);
}

#[sqlx::test(migrations = "../../migrations")]
async fn cookie_endpoint_responses_grant_credentials_to_the_allowed_origin(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_credentialed_cors(&headers, "login");
    let cookie = refresh_cookie(&headers);

    let (status, headers, body) = send(app.clone(), refresh_request(Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    assert_credentialed_cors(&headers, "refresh");
    let cookie = refresh_cookie(&headers);

    let (status, headers, _body) = send(app.clone(), logout_request(Some(&cookie))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_credentialed_cors(&headers, "logout");

    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let (status, headers, _body) = send(
        app.clone(),
        authorized_request(
            "POST",
            "/api/v1/auth/logout-all",
            &token,
            Some(&serde_json::json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_credentialed_cors(&headers, "logout-all");

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let cookie = refresh_cookie(&headers);
    let (status, headers, body) = send(
        app,
        change_password_request(&token, Some(&cookie), PASSWORD, NEW_PASSWORD),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
    assert_credentialed_cors(&headers, "change-password");
}

#[sqlx::test(migrations = "../../migrations")]
async fn bearer_endpoints_never_read_the_refresh_cookie(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cookie = refresh_cookie(&headers);

    for (method, uri, payload) in [
        ("GET", "/api/v1/auth/me", None),
        (
            "POST",
            "/api/v1/auth/reauthenticate",
            Some(serde_json::json!({ "password": PASSWORD })),
        ),
        (
            "POST",
            "/api/v1/auth/logout-all",
            Some(serde_json::json!({})),
        ),
    ] {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::ORIGIN, TEST_ORIGIN)
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::COOKIE, format!("__Host-refresh={cookie}"));
        if payload.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        let body = payload.map_or_else(Body::empty, |payload| Body::from(payload.to_string()));
        let (status, _headers, body) =
            send(app.clone(), builder.body(body).expect("request")).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(problem_code(&json(&body)), "access_token_invalid", "{uri}");
    }

    // Refresh authenticates by cookie only: a valid access token alone is not
    // enough, which also proves the bearer path does not substitute.
    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"].as_str().expect("access token");
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/refresh")
        .header(header::ORIGIN, TEST_ORIGIN)
        .header("x-requested-with", "XMLHttpRequest")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .expect("refresh request");
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem_code(&json(&body)), "refresh_token_invalid");
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_refresh_cookie_uses_the_exact_host_cookie_attributes(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("Set-Cookie");
    let (pair, attributes) = cookie.split_once(';').expect("cookie attributes");
    assert!(pair.starts_with("__Host-refresh="), "{cookie}");
    assert!(
        !pair["__Host-refresh=".len()..].contains([';', ' ']),
        "the value is opaque: {cookie}"
    );
    let mut attributes: Vec<&str> = attributes.split(';').map(str::trim).collect();
    attributes.sort_unstable();
    assert_eq!(
        attributes,
        [
            "HttpOnly",
            "Max-Age=43200",
            "Path=/",
            "SameSite=Strict",
            "Secure"
        ],
        "{cookie}"
    );
    assert!(!cookie.contains("Domain"), "{cookie}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn auth_responses_carry_the_security_headers(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, login_headers, _body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header(header::ORIGIN, TEST_ORIGIN)
        .body(Body::empty())
        .expect("request");
    let (status, unauthorized_headers, _body) = send(app.clone(), request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, csrf_headers, _body) = send(app, login_request()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    for (label, headers) in [
        ("login success", &login_headers),
        ("401 problem", &unauthorized_headers),
        ("403 csrf problem", &csrf_headers),
    ] {
        assert_eq!(
            header_value(headers, "cache-control"),
            Some("no-store"),
            "{label}"
        );
        assert_eq!(
            header_value(headers, "x-content-type-options"),
            Some("nosniff"),
            "{label}"
        );
        assert_eq!(
            header_value(headers, "referrer-policy"),
            Some("no-referrer"),
            "{label}"
        );
        assert_eq!(
            header_value(headers, "x-frame-options"),
            Some("DENY"),
            "{label}"
        );
        assert_eq!(
            header_value(headers, "content-security-policy"),
            Some("default-src 'none'; frame-ancestors 'none'; base-uri 'none'"),
            "{label}"
        );
        assert_eq!(
            header_value(headers, "permissions-policy"),
            Some("camera=(), microphone=(), geolocation=()"),
            "{label}"
        );
    }
}

fn login_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-requested-with", "XMLHttpRequest")
        .body(Body::from(
            serde_json::json!({ "identifier": "alice", "password": PASSWORD }).to_string(),
        ))
        .expect("login request")
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_without_an_origin_header_is_refused_as_csrf_failure(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let (status, headers, body) = send(app, login_request()).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&json(&body)), "csrf_failed");
    assert!(
        headers.get(header::SET_COOKIE).is_none(),
        "a refused login must not issue a refresh cookie"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_without_a_json_content_type_is_refused(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ORIGIN, TEST_ORIGIN)
        .header("x-requested-with", "XMLHttpRequest")
        .body(Body::from(
            serde_json::json!({ "identifier": "alice", "password": PASSWORD }).to_string(),
        ))
        .expect("login request");
    let (status, _headers, body) = send(app, request).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&json(&body)), "csrf_failed");
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_from_a_disallowed_origin_is_refused(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    for origin in ["null", "*", "https://evil.example.com"] {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, origin)
            .header("x-requested-with", "XMLHttpRequest")
            .body(Body::from(
                serde_json::json!({ "identifier": "alice", "password": PASSWORD }).to_string(),
            ))
            .expect("login request");
        let (status, headers, body) = send(app.clone(), request).await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{origin}");
        assert_eq!(problem_code(&json(&body)), "csrf_failed", "{origin}");
        assert!(
            headers.get(header::SET_COOKIE).is_none(),
            "{origin} must not receive a refresh cookie"
        );
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn every_cookie_endpoint_enforces_the_csrf_boundary(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"].as_str().expect("access token");

    // Each request is otherwise well-formed, including a valid Bearer token
    // for logout-all; only the CSRF headers are missing.
    for (uri, authorization) in [
        ("/api/v1/auth/refresh", None),
        ("/api/v1/auth/logout", None),
        ("/api/v1/auth/logout-all", Some(token)),
    ] {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = authorization {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let (status, _headers, body) = send(
            app.clone(),
            builder.body(Body::from("{}")).expect("request"),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(problem_code(&json(&body)), "csrf_failed", "{uri}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn reauthentication_does_not_require_csrf_headers(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;
    let (status, _headers, body) = login(&app, "alice", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["access_token"].as_str().expect("access token");

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/reauthenticate")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "password": PASSWORD }).to_string(),
        ))
        .expect("reauth request");
    let (status, _headers, body) = send(app, request).await;

    assert_eq!(status, StatusCode::OK, "{:?}", json(&body));
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_without_the_csrf_header_is_refused(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("alice", "alice@example.com")).await;

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, TEST_ORIGIN)
        .body(Body::from(
            serde_json::json!({ "identifier": "alice", "password": PASSWORD }).to_string(),
        ))
        .expect("login request");
    let (status, _headers, body) = send(app, request).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem_code(&json(&body)), "csrf_failed");
}
