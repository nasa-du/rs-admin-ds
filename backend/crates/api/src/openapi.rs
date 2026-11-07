//! OpenAPI 3.1 document generation.
//!
//! The document is a build artifact: `openapi` writes it to
//! `backend/openapi.json`, CI diffs the result, and the frontend generates its
//! TypeScript client from the committed file.

use crate::auth::{
    LoginRequest, LoginResponse, ReauthenticateRequest, ReauthenticateResponse, RoleRefResponse,
    UserSelfResponse,
};
use crate::error::{ErrorCodeSchema, FieldError, ProblemDetails};
use crate::router::{HealthState, HealthStatus, openapi_router};
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "rs-admin API",
        version = env!("CARGO_PKG_VERSION"),
        description = "API for the rs-admin login and permission system. \
                       Health probes are served at the root; business endpoints \
                       are served under `/api/v1`."
    ),
    servers((url = "/", description = "Same-origin server")),
    tags(
        (name = "Health", description = "Liveness and readiness probes"),
        (name = "Auth", description = "Password login, current user and reauthentication")
    ),
    components(schemas(
        ProblemDetails,
        FieldError,
        ErrorCodeSchema,
        HealthState,
        HealthStatus,
        LoginRequest,
        LoginResponse,
        ReauthenticateRequest,
        ReauthenticateResponse,
        RoleRefResponse,
        UserSelfResponse,
    )),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

/// Declares the Bearer JWT scheme used by `/auth/me` and
/// `/auth/reauthenticate`.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::new);
        let mut scheme = Http::new(HttpAuthScheme::Bearer);
        scheme.bearer_format = Some("JWT".to_owned());
        components.add_security_scheme("bearerAuth", SecurityScheme::Http(scheme));
    }
}

/// Build the OpenAPI 3.1 document for the running API surface.
///
/// Paths are collected from the same [`crate::router::openapi_router`] table
/// that serves traffic, so route registration and documentation cannot drift
/// apart within the application.
pub fn openapi_document() -> utoipa::openapi::OpenApi {
    let (_router, paths) = openapi_router().split_for_parts();
    let mut document = ApiDoc::openapi();
    document.merge(paths);
    document
}
