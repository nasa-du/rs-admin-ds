//! Browser security boundary: the CORS origin policy and the CSRF guard for
//! cookie-authenticated endpoints (docs/specs/login-system.md §7.11).

use crate::error::ApiError;
use crate::json::has_json_content_type;
use crate::router::AppState;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
    ACCESS_CONTROL_REQUEST_METHOD, ORIGIN, VARY,
};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rs_admin_domain::ErrorCode;
use std::sync::Arc;

/// Methods the browser may use on the API (spec §7.11).
const ALLOW_METHODS: &str = "GET, POST, PUT, PATCH, DELETE, OPTIONS";

/// Request headers the browser may send.
const ALLOW_HEADERS: &str = "Authorization, Content-Type, X-Requested-With, X-Request-Id";

/// Response headers JavaScript may read.
const EXPOSE_HEADERS: &str = "X-Request-Id, Retry-After, WWW-Authenticate";

/// How long a browser may cache the preflight result.
const MAX_AGE_SECONDS: &str = "600";

/// POST endpoints that set or clear the `__Host-refresh` cookie. Only these
/// responses may carry `Access-Control-Allow-Credentials: true`.
///
/// Keep in sync with the route table in `router::openapi_router`: a renamed
/// path here silently narrows the credentialed surface (fail-closed).
const CREDENTIAL_ENDPOINTS: [&str; 5] = [
    "/api/v1/auth/login",
    "/api/v1/auth/refresh",
    "/api/v1/auth/logout",
    "/api/v1/auth/logout-all",
    "/api/v1/auth/change-password",
];

/// The only accepted `X-Requested-With` value. Browsers cannot set custom
/// headers cross-origin without a CORS preflight, so requiring it forces the
/// request through the origin allowlist.
pub const CSRF_HEADER: &str = "x-requested-with";
pub const CSRF_HEADER_VALUE: &str = "XMLHttpRequest";

/// Origins allowed to call the API from a browser.
///
/// The list is validated at startup (`*` and `null` are rejected), canonicalized
/// and deduplicated by [`rs_admin_config::OriginConfig`]. An empty list denies
/// every cross-origin request; the walking-skeleton tests use that default.
#[derive(Clone, Debug, Default)]
pub struct BrowserSecurity {
    allowed_origins: Arc<[String]>,
}

impl BrowserSecurity {
    pub fn new(origins: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed_origins: origins.into_iter().collect(),
        }
    }

    /// Compare the request `Origin` byte-for-byte with the allowlist. Browsers
    /// serialize origins canonically (lowercase scheme/host, no default port,
    /// no trailing slash), matching the startup canonicalization.
    pub fn allows_origin(&self, origin: &[u8]) -> bool {
        self.allowed_origins
            .iter()
            .any(|allowed| allowed.as_bytes() == origin)
    }
}

/// CORS middleware. It runs inside the request-id layer so even a short-circuited
/// preflight response carries `X-Request-Id`, and outside the timeout layer so
/// timeout problem responses still carry the CORS headers.
///
/// A disallowed or missing origin simply receives no grant; responses are never
/// rewritten to echo an untrusted origin. `Vary: Origin` is always present so
/// no shared cache can serve one origin's response to another.
pub(crate) async fn cors(
    State(browser): State<BrowserSecurity>,
    request: Request,
    next: Next,
) -> Response {
    let origin = request.headers().get(ORIGIN).cloned();
    let allowed_origin = origin.filter(|value| browser.allows_origin(value.as_bytes()));

    let preflight = request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(ACCESS_CONTROL_REQUEST_METHOD);
    let intended_method = if preflight {
        request
            .headers()
            .get(ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Method::from_bytes(value.as_bytes()).ok())
    } else {
        Some(request.method().clone())
    };
    let credentials = intended_method
        .as_ref()
        .is_some_and(|method| allows_credentials(method, request.uri().path()));

    let mut response = if preflight {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };

    append_vary_origin(response.headers_mut());
    if let Some(origin) = allowed_origin {
        let headers = response.headers_mut();
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(
            ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static(EXPOSE_HEADERS),
        );
        if credentials {
            headers.insert(
                ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
        }
        if preflight {
            headers.insert(
                ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static(ALLOW_METHODS),
            );
            headers.insert(
                ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static(ALLOW_HEADERS),
            );
            headers.insert(
                ACCESS_CONTROL_MAX_AGE,
                HeaderValue::from_static(MAX_AGE_SECONDS),
            );
        }
    }

    response
}

fn allows_credentials(method: &Method, path: &str) -> bool {
    method == Method::POST && CREDENTIAL_ENDPOINTS.contains(&path)
}

fn append_vary_origin(headers: &mut axum::http::HeaderMap) {
    let present = headers.get_all(VARY).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("origin"))
        })
    });
    if !present {
        headers.append(VARY, HeaderValue::from_static("Origin"));
    }
}

/// Extractor that refuses a cookie-authenticated request unless it carries an
/// allowed exact `Origin`.
pub struct CsrfProtected;

impl FromRequestParts<AppState> for CsrfProtected {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let origin_allowed = parts
            .headers
            .get(ORIGIN)
            .is_some_and(|origin| state.browser().allows_origin(origin.as_bytes()));
        let csrf_header_present = parts.headers.get(CSRF_HEADER).is_some_and(|value| {
            value
                .as_bytes()
                .eq_ignore_ascii_case(CSRF_HEADER_VALUE.as_bytes())
        });
        if !origin_allowed || !csrf_header_present || !has_json_content_type(&parts.headers) {
            return Err(ApiError::new(ErrorCode::CsrfFailed));
        }
        Ok(Self)
    }
}
