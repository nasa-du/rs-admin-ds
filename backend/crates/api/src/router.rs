//! Router assembly, health endpoints and the common middleware stack.

use crate::auth::{AUTH_JSON_BODY_BYTES, AuthServices};
use crate::browser::BrowserSecurity;
use crate::error::ApiError;
use crate::middleware;
use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use rs_admin_domain::ErrorCode;
use rs_admin_store::{Database, Readiness};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Maximum JSON request body for regular endpoints.
pub const MAX_JSON_BODY_BYTES: usize = 64 * 1024;

/// Total request timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum in-flight requests before backpressure is applied.
pub const MAX_IN_FLIGHT_REQUESTS: usize = 100;

/// Readiness probes must answer within this budget.
pub const READINESS_TIMEOUT: Duration = Duration::from_secs(2);

/// Shared application state.
///
/// The authentication services are optional so the walking-skeleton health
/// routes keep working in tests that do not provide key material; auth
/// handlers fail closed with 503 when they are absent.
#[derive(Clone)]
pub struct AppState {
    pub database: Database,
    auth: Option<Arc<AuthServices>>,
    browser: BrowserSecurity,
}

impl AppState {
    pub fn new(database: Database) -> Self {
        Self {
            database,
            auth: None,
            browser: BrowserSecurity::default(),
        }
    }

    pub fn with_auth(database: Database, auth: AuthServices) -> Self {
        Self {
            database,
            auth: Some(Arc::new(auth)),
            browser: BrowserSecurity::default(),
        }
    }

    /// Install the canonicalized CORS allowlist (spec §7.11). Without it every
    /// browser request is refused because no origin is trusted.
    pub fn with_allowed_origins(mut self, origins: Vec<String>) -> Self {
        self.browser = BrowserSecurity::new(origins);
        self
    }

    pub(crate) fn auth(&self) -> Option<&AuthServices> {
        self.auth.as_deref()
    }

    pub(crate) fn browser(&self) -> &BrowserSecurity {
        &self.browser
    }
}

/// Probe state returned by the health endpoints.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    Live,
    Ready,
    Unready,
}

/// Body returned by the health probes.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct HealthStatus {
    pub status: HealthState,
}

/// OpenAPI-aware router shared by the server and the document generator.
///
/// Keeping route registration and documentation in one table prevents a
/// handler from being served without appearing in `openapi.json`, or vice
/// versa.
pub(crate) fn openapi_router() -> OpenApiRouter<AppState> {
    // Authentication request bodies are tiny; the 1 KiB cap limits the cost of
    // unauthenticated JSON parsing (spec §8.1).
    let auth = OpenApiRouter::new()
        .routes(routes!(crate::auth::login))
        .routes(routes!(crate::auth::logout))
        .routes(routes!(crate::auth::logout_all))
        .routes(routes!(crate::auth::refresh))
        .routes(routes!(crate::auth::get_me))
        .routes(routes!(crate::auth::reauthenticate))
        .routes(routes!(crate::auth::change_password))
        .layer(DefaultBodyLimit::max(AUTH_JSON_BODY_BYTES));

    let roles = OpenApiRouter::new()
        .routes(routes!(crate::roles::list_roles, crate::roles::create_role))
        .routes(routes!(
            crate::roles::get_role,
            crate::roles::update_role_metadata,
            crate::roles::delete_role
        ))
        .routes(routes!(crate::roles::set_role_permissions))
        .routes(routes!(crate::roles::list_permissions));

    let users = OpenApiRouter::new()
        .routes(routes!(crate::users::list_users))
        .routes(routes!(crate::users::create_user))
        .routes(routes!(crate::users::get_user, crate::users::update_user));

    OpenApiRouter::new()
        .routes(routes!(health_live))
        .routes(routes!(health_ready))
        .merge(auth)
        .merge(roles)
        .merge(users)
}

/// Build the production router.
pub fn build_router(state: AppState) -> Router {
    let browser = state.browser().clone();
    let router: Router<AppState> = openapi_router().into();
    with_common_middleware_for(
        router.fallback(not_found).with_state(state),
        REQUEST_TIMEOUT,
        browser,
    )
}

/// Apply the common middleware stack. Exposed so integration tests can wrap
/// additional routes and exercise the same contract.
///
/// These helpers install an empty CORS allowlist, so they grant no browser
/// origin. [`build_router`] installs the policy configured on [`AppState`].
pub fn with_common_middleware(router: Router) -> Router {
    with_common_middleware_and_timeout(router, REQUEST_TIMEOUT)
}

/// Same stack with an explicit request timeout. Exposed for timeout tests.
#[doc(hidden)]
pub fn with_common_middleware_and_timeout(router: Router, timeout: Duration) -> Router {
    with_common_middleware_for(router, timeout, BrowserSecurity::default())
}

fn with_common_middleware_for(
    router: Router,
    timeout: Duration,
    browser: BrowserSecurity,
) -> Router {
    router.layer(
        ServiceBuilder::new()
            .layer(from_fn(middleware::observe))
            .layer(from_fn(middleware::security_headers))
            .layer(from_fn(middleware::normalize_problem))
            .layer(DefaultBodyLimit::max(MAX_JSON_BODY_BYTES))
            .layer(ConcurrencyLimitLayer::new(MAX_IN_FLIGHT_REQUESTS))
            .layer(from_fn(middleware::request_id))
            .layer(from_fn_with_state(browser, crate::browser::cors))
            .layer(from_fn_with_state(timeout, middleware::request_timeout)),
    )
}

#[utoipa::path(
    get,
    path = "/health/live",
    tag = "Health",
    operation_id = "healthLive",
    responses((
        status = 200,
        description = "Process is alive",
        body = HealthStatus,
        headers(("X-Request-Id" = String, description = "Server-generated request id"))
    ))
)]
async fn health_live() -> Json<HealthStatus> {
    Json(HealthStatus {
        status: HealthState::Live,
    })
}

#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "Health",
    operation_id = "healthReady",
    responses(
        (
            status = 200,
            description = "Service is ready",
            body = HealthStatus,
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Service is not ready",
            body = HealthStatus,
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
async fn health_ready(State(state): State<AppState>) -> Response {
    match state.database.check_ready(READINESS_TIMEOUT).await {
        Readiness::Ready => Json(HealthStatus {
            status: HealthState::Ready,
        })
        .into_response(),
        Readiness::Unready(reason) => {
            tracing::warn!(reason = ?reason, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthStatus {
                    status: HealthState::Unready,
                }),
            )
                .into_response()
        }
    }
}

async fn not_found() -> ApiError {
    ApiError::new(ErrorCode::NotFound)
}
