//! OpenAPI 3.1 document generation.
//!
//! The document is a build artifact: `openapi` writes it to
//! `backend/openapi.json`, CI diffs the result, and the frontend generates its
//! TypeScript client from the committed file.

use crate::auth::{
    ChangePasswordRequest, ChangePasswordResponse, LoginRequest, LoginResponse, LogoutRequest,
    ReauthenticateRequest, ReauthenticateResponse, RefreshRequest, RefreshResponse, RoleRef,
    UserSelfResponse,
};
use crate::error::{ErrorCodeSchema, FieldError, ProblemDetails};
use crate::roles::{
    CreateRoleRequest, PermissionGroup, PermissionItem, PermissionListResponse, RoleDetail,
    RoleListResponse, RoleSummary, SetRolePermissionsRequest, UpdateRoleRequest,
};
use crate::router::{HealthState, HealthStatus, openapi_router};
use crate::users::{
    CreateUserRequest, UpdateUserRequest, UserCreatedResponse, UserDetail, UserListResponse,
    UserSummary,
};
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
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
        (
            name = "Auth",
            description = "Password login, sessions, self-service password change, current user and reauthentication"
        ),
        (name = "Users", description = "User listing, detail, creation and profile edits"),
        (name = "Roles", description = "Role catalog and custom role management"),
        (name = "Permissions", description = "The code-owned permission catalog")
    ),
    components(schemas(
        ProblemDetails,
        FieldError,
        ErrorCodeSchema,
        HealthState,
        HealthStatus,
        ChangePasswordRequest,
        ChangePasswordResponse,
        LoginRequest,
        LoginResponse,
        LogoutRequest,
        ReauthenticateRequest,
        ReauthenticateResponse,
        RefreshRequest,
        RefreshResponse,
        RoleRef,
        UserSelfResponse,
        CreateRoleRequest,
        UpdateRoleRequest,
        SetRolePermissionsRequest,
        RoleSummary,
        RoleDetail,
        RoleListResponse,
        PermissionItem,
        PermissionGroup,
        PermissionListResponse,
        UserSummary,
        UserDetail,
        UserListResponse,
        CreateUserRequest,
        UpdateUserRequest,
        UserCreatedResponse,
    )),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

/// Declares the Bearer JWT scheme used by protected endpoints and the
/// documentation-only cookie scheme used by `/auth/refresh`.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::new);
        let mut scheme = Http::new(HttpAuthScheme::Bearer);
        scheme.bearer_format = Some("JWT".to_owned());
        components.add_security_scheme("bearerAuth", SecurityScheme::Http(scheme));
        components.add_security_scheme(
            "refreshCookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("__Host-refresh"))),
        );
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
