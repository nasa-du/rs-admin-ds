//! Authentication endpoints: password login, the current-user endpoint,
//! self-service password change and step-up reauthentication
//! (docs/specs/login-system.md §5.1–§5.8, §7.6).
//!
//! Login is the only place a refresh token is issued in this ticket. The raw
//! value is returned exclusively through the `__Host-refresh` cookie; the
//! database stores its SHA-256 digest. Access tokens are EdDSA JWTs kept in the
//! client's memory and never persisted.
//!
//! Endpoints outside the restricted-session allowlist must extract
//! [`Authenticated`]; only the allowlisted [`get_me`] and [`change_password`]
//! may extract [`SessionPrincipal`]. Extracting the raw principal elsewhere
//! would let a temporary-password session bypass the forced change.

use crate::browser::CsrfProtected;
use crate::client_ip::ClientIp;
use crate::error::{ApiError, FieldError, identity_error};
use crate::json::ApiJson;
use crate::middleware::RequestId;
use crate::router::AppState;
use crate::users::store_unavailable;
use crate::wire::format_timestamp;
use axum::Extension;
use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{AUTHORIZATION, COOKIE, RETRY_AFTER, SET_COOKIE, USER_AGENT};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use rs_admin_audit::{
    AuditAction, AuditActor, AuditEvent, AuditEventBuilder, AuditResult, AuditWriter, TargetType,
};
use rs_admin_config::{AuthSettings, RotationKeys, SecurityConfig};
use rs_admin_domain::ErrorCode;
use rs_admin_domain::identity::{DisplayName, Email, Username};
use rs_admin_domain::permission::PermissionKey;
use rs_admin_domain::rbac::{BuiltInRole, EffectivePermissions, RoleGrant};
use rs_admin_security::{
    ClientIpResolution, HmacKey, JWT_ACCESS_TOKEN_TTL_SECONDS, JwtIssuer, PasswordError,
    PasswordPolicy, PasswordService, TokenError, TrustedProxies, VerifyOutcome,
    derive_refresh_token, exceeds_authentication_limit, generate_refresh_token,
    global_throttle_key, hash_refresh_token, identifier_hmac_hex, identifier_throttle_key,
    ip_throttle_key, passwords_equal,
};
use rs_admin_store::Database;
use rs_admin_store::audit::transaction_now;
use rs_admin_store::auth::{
    AuthSession, AuthUser, FailureRecord, NewRefreshToken, NewSession,
    REVOKE_REASON_EXCESS_SESSIONS, REVOKE_REASON_LOGOUT, REVOKE_REASON_LOGOUT_ALL,
    REVOKE_REASON_PASSWORD_CHANGE, REVOKE_REASON_TOKEN_REUSE, RefreshContext, RefreshTokenRow,
    ThrottleScope, UserRole, bump_token_version, clear_unknown_login_failures,
    complete_password_change, count_active_sessions, create_session, database_clock_now,
    find_oldest_active_sessions, find_refresh_token, find_user_by_email, find_user_by_id,
    find_user_by_username, insert_refresh_token, known_account_lock_seconds, load_active_session,
    load_login_throttle_locks, load_role_grants, load_throttle_lock, lock_active_session,
    lock_refresh_context, lock_user_row, mark_refresh_token_rotated, next_rotation_counter,
    record_known_login_failure, record_throttle_failure, record_unknown_login_failure,
    release_expired_global_throttle, replace_password_hash, reset_login_failures,
    revoke_other_user_refresh_tokens, revoke_other_user_sessions, revoke_refresh_family,
    revoke_session_family, revoke_user_refresh_tokens, revoke_user_sessions,
    touch_authenticated_at, touch_session_activity, unknown_account_lock_seconds,
    update_successful_login,
};
use rs_admin_store::users::update_user as store_update_user;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::net::IpAddr;
use std::str::FromStr;
use utoipa::ToSchema;
use uuid::Uuid;

/// Refresh cookie name; the `__Host-` prefix requires `Secure`, `Path=/` and no
/// `Domain` attribute.
pub const REFRESH_COOKIE_NAME: &str = "__Host-refresh";

/// Refresh cookie lifetime, matching the session absolute lifetime.
pub const REFRESH_COOKIE_MAX_AGE_SECONDS: i64 = 43_200;

/// Session idle timeout.
pub const SESSION_IDLE_SECONDS: i64 = 30 * 60;

/// Session absolute lifetime.
pub const SESSION_ABSOLUTE_SECONDS: i64 = 12 * 60 * 60;

/// Maximum number of active (unrevoked, unexpired) sessions a user may hold; a
/// new login evicts the least recently active session beyond this cap
/// (spec §5.3).
pub const MAX_ACTIVE_SESSIONS_PER_USER: i64 = 5;

/// Largest accepted authentication request body (spec §8.1).
pub const AUTH_JSON_BODY_BYTES: usize = 1024;

/// Window after a rotation in which the previous token may be retried and
/// returns the same successor without another rotation (spec §5.2).
pub const REFRESH_GRACE_SECONDS: i64 = 30;

/// Minimum spacing between `last_seen_at` / `idle_expires_at` updates
/// (spec §5.3).
pub const SESSION_ACTIVITY_TOUCH_SECONDS: i64 = 60;

const TOKEN_TYPE: &str = "Bearer";

/// Authentication services built once at startup.
#[derive(Clone)]
pub struct AuthServices {
    jwt: JwtIssuer,
    passwords: PasswordService,
    audit: AuditWriter,
    throttle_key: HmacKey,
    refresh_derivation: RotationKeys,
    settings: AuthSettings,
    trusted_proxies: TrustedProxies,
}

impl AuthServices {
    /// Build the JWT codec, Argon2 service and audit writer from validated
    /// configuration. Fails startup when key material cannot be used.
    pub fn new(
        security: &SecurityConfig,
        auth: &AuthSettings,
        trusted_proxies: TrustedProxies,
    ) -> Result<Self, AuthServiceError> {
        let passwords = PasswordService::new(auth.password_hash_concurrency)?;
        Self::with_password_service(security, passwords, auth.clone(), trusted_proxies)
    }

    /// Build the remaining services around an existing password service. This
    /// lets callers share one bounded hashing pool across components.
    pub fn with_password_service(
        security: &SecurityConfig,
        passwords: PasswordService,
        settings: AuthSettings,
        trusted_proxies: TrustedProxies,
    ) -> Result<Self, AuthServiceError> {
        let jwt = JwtIssuer::new(
            &security.jwt.issuer,
            &security.jwt.audience,
            &security.jwt.kid,
            &security.jwt.signing_key,
        )?;

        Ok(Self {
            jwt,
            passwords,
            audit: AuditWriter::new(security.audit_chain.keys.clone()),
            throttle_key: security.throttle_key.clone(),
            refresh_derivation: security.refresh_derivation.clone(),
            settings,
            trusted_proxies,
        })
    }

    /// Resolve the client address from the TCP peer and `X-Forwarded-For`.
    /// The caller logs a warning when the resolution fell back to the peer.
    pub fn resolve_client_ip(
        &self,
        peer: IpAddr,
        forwarded_for: Option<&str>,
    ) -> ClientIpResolution {
        self.trusted_proxies.resolve(peer, forwarded_for)
    }

    /// The tamper-evident audit writer shared by every endpoint.
    pub(crate) fn audit(&self) -> &AuditWriter {
        &self.audit
    }

    /// Maximum age of the last password authentication for sensitive actions
    /// (spec §5.8).
    pub(crate) fn reauth_max_age(&self) -> std::time::Duration {
        self.settings.reauth_max_age
    }

    /// Lifetime of an administrator-issued temporary password (spec §7.7).
    pub(crate) fn temporary_password_ttl(&self) -> std::time::Duration {
        self.settings.temp_password_ttl
    }

    /// The bounded password hashing service, shared by authentication and
    /// administrator credential issuance.
    pub(crate) fn passwords(&self) -> &PasswordService {
        &self.passwords
    }
}

/// Startup errors from the authentication services. Messages never contain key
/// material.
#[derive(Debug, thiserror::Error)]
pub enum AuthServiceError {
    #[error("invalid JWT key configuration")]
    Jwt(#[from] rs_admin_security::KeyError),
    #[error("invalid password hashing configuration")]
    Password(#[from] PasswordError),
}

/// The authenticated principal produced by the per-request pipeline. This is
/// the raw authentication result: callers on the restricted-session allowlist
/// (`GET /auth/me`, `POST /auth/change-password`) use it directly.
#[derive(Clone, Debug)]
pub struct SessionPrincipal {
    pub user: AuthUser,
    pub session: AuthSession,
    pub roles: Vec<UserRole>,
    pub effective: EffectivePermissions,
}

impl FromRequestParts<AppState> for SessionPrincipal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
        let token = bearer_token(&parts.headers).ok_or_else(ApiError::access_token_invalid)?;
        let claims = services
            .jwt
            .verify(token)
            .map_err(|_| ApiError::access_token_invalid())?;

        let user = find_user_by_id(state.database.pool(), claims.sub)
            .await
            .map_err(dependency_unavailable)?
            .ok_or_else(ApiError::session_invalid)?;
        if !user.is_active() {
            return Err(ApiError::new(ErrorCode::AccountDisabled));
        }

        let session = load_active_session(state.database.pool(), claims.sid, claims.sub)
            .await
            .map_err(dependency_unavailable)?
            .ok_or_else(ApiError::session_invalid)?;
        if user.token_version != claims.ver {
            return Err(ApiError::session_invalid());
        }

        let roles = load_role_grants(state.database.pool(), user.id)
            .await
            .map_err(dependency_unavailable)?;
        let effective = resolve_effective_permissions(&roles);

        Ok(Self {
            user,
            session,
            roles,
            effective,
        })
    }
}

/// A full-access principal. Every endpoint outside the restricted-session
/// allowlist extracts this type, so a session that must change its password is
/// refused with 403 `password_change_required` before the handler runs.
#[derive(Clone, Debug)]
pub struct Authenticated(pub SessionPrincipal);

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let principal = SessionPrincipal::from_request_parts(parts, state).await?;
        if principal.user.must_change_password {
            return Err(ApiError::new(ErrorCode::PasswordChangeRequired));
        }
        Ok(Self(principal))
    }
}

/// `POST /auth/login` request body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// Username or email address.
    pub identifier: String,
    pub password: String,
}

/// A role as exposed by `/auth/me` and the login response.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RoleRef {
    pub id: String,
    pub name: String,
    pub is_builtin: bool,
}

/// The current user's profile, roles and effective permissions.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UserSelfResponse {
    pub id: String,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub phone: Option<String>,
    pub status: String,
    pub roles: Vec<RoleRef>,
    pub permissions: Vec<String>,
    pub is_super_admin: bool,
    pub must_change_password: bool,
    pub temporary_password_expires_at: Option<String>,
}

/// `POST /auth/login` success body.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub user: UserSelfResponse,
}

/// `POST /auth/reauthenticate` request body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReauthenticateRequest {
    pub password: String,
}

/// `POST /auth/reauthenticate` success body.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ReauthenticateResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub auth_time: i64,
}

/// `POST /auth/change-password` request body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// `POST /auth/change-password` success body; the rotated refresh token is
/// only in the `Set-Cookie` header.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ChangePasswordResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

/// `POST /auth/refresh` request body. The refresh token travels only in the
/// `__Host-refresh` cookie, so the body is an empty JSON object.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {}

/// `POST /auth/refresh` success body; the rotated refresh token is only in the
/// `Set-Cookie` header.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RefreshResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

/// Request body for `POST /auth/logout` and `POST /auth/logout-all`. Both
/// identify the caller by cookie or Bearer token, so the body is an empty JSON
/// object.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LogoutRequest {}

/// `PATCH /auth/me` request body. Self-service profile editing is limited to
/// the display name; every other profile field is administrator-managed.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateMeRequest {
    #[serde(default, deserialize_with = "crate::json::non_null_display_name")]
    #[schema(nullable = false)]
    pub display_name: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "Auth",
    operation_id = "login",
    params((
        "X-Requested-With" = String,
        Header,
        description = "Required CSRF header; must be exactly `XMLHttpRequest`"
    )),
    request_body = LoginRequest,
    responses(
        (
            status = 200,
            description = "Authenticated; access token and refresh cookie issued",
            body = LoginResponse,
            headers(
                ("Set-Cookie" = String, description = "Host-only refresh cookie"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 401,
            description = "Invalid credentials",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Account disabled, temporary password expired or CSRF validation failed",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 429,
            description = "Login throttled",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("Retry-After" = String, description = "Seconds until the next attempt"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 503,
            description = "Authentication service unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    _csrf: CsrfProtected,
    ApiJson(payload): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let audit = AuditContext {
        request_id: request_id.0,
        user_agent: user_agent(&headers),
        client_ip: client_ip.0,
    };
    let identifier = payload.identifier.trim().to_owned();
    let normalized = normalize_identifier(&identifier);

    // Throttle state is keyed by the normalized identifier when it parses,
    // otherwise by the trimmed input, so that case and Unicode variants cannot
    // be used to rotate the key.
    let throttle_identifier = match &normalized {
        Some(LoginIdentifier::Username(value)) | Some(LoginIdentifier::Email(value)) => {
            value.clone()
        }
        None => identifier.clone(),
    };
    let identifier_key = identifier_throttle_key(&services.throttle_key, &throttle_identifier);

    // The IP and global layers are checked before any hashing: a locked layer
    // refuses without paying for Argon2 (spec §8.3).
    ensure_shared_layers_unlocked(services, &state, audit.client_ip).await?;

    let user = find_user(&state.database, normalized.as_ref()).await?;

    // The account layer still runs one dummy verification while locked so the
    // response cannot distinguish a locked account from a merely wrong
    // password (spec §8.3).
    let account_lock = match &user {
        Some(user) => known_account_lock_seconds(state.database.pool(), user.id)
            .await
            .map_err(dependency_unavailable)?,
        None => unknown_account_lock_seconds(state.database.pool(), &identifier_key)
            .await
            .map_err(dependency_unavailable)?,
    };
    if let Some(seconds) = account_lock {
        services
            .passwords
            .verify_dummy()
            .await
            .map_err(password_unavailable)?;
        return Err(login_locked(seconds));
    }

    let outcome = verify_password(services, &payload.password, user.as_ref()).await?;
    let password_matches = outcome.as_ref().is_some_and(|outcome| outcome.valid);

    match (user, password_matches) {
        (Some(user), true) => {
            let outcome = outcome.expect("verified password has an outcome");
            complete_login(&state, services, user, outcome.replacement_hash, audit).await
        }
        (Some(user), false) => {
            let lock = record_counted_failure(
                &state,
                services,
                FailureIdentity {
                    user: Some(&user),
                    identifier_key: None,
                    identifier_hmac: None,
                },
                &audit,
                AuditAction::AuthnLoginFail,
                FailureSource::Login,
            )
            .await?;
            Err(lock.map_or_else(|| ApiError::new(ErrorCode::InvalidCredentials), lock_error))
        }
        (None, _) => {
            let identifier_hmac = identifier_hmac_hex(&services.throttle_key, &throttle_identifier);
            let lock = record_counted_failure(
                &state,
                services,
                FailureIdentity {
                    user: None,
                    identifier_key: Some(&identifier_key),
                    identifier_hmac: Some(&identifier_hmac),
                },
                &audit,
                AuditAction::AuthnLoginFail,
                FailureSource::Login,
            )
            .await?;
            Err(lock.map_or_else(|| ApiError::new(ErrorCode::InvalidCredentials), lock_error))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "Auth",
    operation_id = "getMe",
    security(("bearerAuth" = [])),
    responses(
        (
            status = 200,
            description = "Current user profile and effective permissions",
            body = UserSelfResponse,
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Account disabled",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
pub async fn get_me(principal: SessionPrincipal) -> Json<UserSelfResponse> {
    Json(user_self(
        &principal.user,
        &principal.roles,
        &principal.effective,
    ))
}

#[utoipa::path(
    patch,
    path = "/api/v1/auth/me",
    tag = "Auth",
    operation_id = "updateMe",
    security(("bearerAuth" = [])),
    request_body = UpdateMeRequest,
    responses(
        (
            status = 200,
            description = "Display name updated; the updated current-user profile is returned",
            body = UserSelfResponse,
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Account disabled or a password change is required",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Empty update or invalid display name",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Authentication service unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
pub async fn update_me(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    ApiJson(payload): ApiJson<UpdateMeRequest>,
) -> Result<Json<UserSelfResponse>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let Some(raw_display_name) = payload.display_name.as_deref() else {
        return Err(ApiError::new(ErrorCode::EmptyUpdate));
    };
    let display_name = DisplayName::parse(raw_display_name)
        .map_err(|error| identity_error("/display_name", error))?;

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let locked = lock_user_row(&mut transaction, principal.user.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;
    if locked.token_version != principal.user.token_version {
        transaction.rollback().await.ok();
        return Err(ApiError::session_invalid());
    }
    if !locked.is_active() {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::AccountDisabled));
    }

    // An unchanged value is idempotent and writes no audit row, matching the
    // administrator profile endpoint.
    if locked.display_name == display_name.as_str() {
        let roles = load_role_grants(&mut *transaction, locked.id)
            .await
            .map_err(dependency_unavailable)?;
        transaction.rollback().await.ok();
        let effective = resolve_effective_permissions(&roles);
        return Ok(Json(user_self(&locked, &roles, &effective)));
    }

    store_update_user(
        &mut transaction,
        locked.id,
        None,
        None,
        Some(display_name.as_str()),
        None,
    )
    .await
    .map_err(store_unavailable)?;

    let mut updated = locked;
    updated.display_name = display_name.as_str().to_owned();
    let roles = load_role_grants(&mut *transaction, updated.id)
        .await
        .map_err(dependency_unavailable)?;
    let effective = resolve_effective_permissions(&roles);

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::UserUpdated,
            AuditResult::Success,
            AuditActor::user(updated.id, updated.username.clone()),
            request_id.0,
        )
        .target(
            TargetType::User,
            Some(updated.id),
            Some(updated.username.clone()),
        )
        .detail(
            "changed_fields",
            serde_json::json!({ "display_name": true }),
        )
        // `user_updated` is shared with administrator edits; this marks the
        // self-service path without widening the closed audit catalog.
        .detail("source", "self")
        .ip(client_ip.0),
        user_agent(&headers),
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(&mut transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    transaction.commit().await.map_err(dependency_unavailable)?;

    Ok(Json(user_self(&updated, &roles, &effective)))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/reauthenticate",
    tag = "Auth",
    operation_id = "reauthenticate",
    security(("bearerAuth" = [])),
    request_body = ReauthenticateRequest,
    responses(
        (
            status = 200,
            description = "Reauthenticated; access token reissued without rotating the session",
            body = ReauthenticateResponse,
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Account disabled or a password change is required",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Current password is invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 429,
            description = "Login throttled",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("Retry-After" = String, description = "Seconds until the next attempt"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 503,
            description = "Authentication service unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
pub async fn reauthenticate(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: Authenticated,
    ApiJson(payload): ApiJson<ReauthenticateRequest>,
) -> Result<Json<ReauthenticateResponse>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let audit = AuditContext {
        request_id: request_id.0,
        user_agent: user_agent(&headers),
        client_ip: client_ip.0,
    };

    // Reauthentication is subject to the IP/global layers and, like login,
    // counts failures at the account layer (spec §8.3). The global circuit
    // deliberately covers login and reauthentication only.
    ensure_shared_layers_unlocked(services, &state, audit.client_ip).await?;
    if let Some(seconds) = known_account_lock_seconds(state.database.pool(), principal.user.id)
        .await
        .map_err(dependency_unavailable)?
    {
        services
            .passwords
            .verify_dummy()
            .await
            .map_err(password_unavailable)?;
        return Err(login_locked(seconds));
    }

    let outcome = verify_password(services, &payload.password, Some(&principal.user)).await?;
    if !outcome.as_ref().is_some_and(|outcome| outcome.valid) {
        let lock = record_counted_failure(
            &state,
            services,
            FailureIdentity {
                user: Some(&principal.user),
                identifier_key: None,
                identifier_hmac: None,
            },
            &audit,
            AuditAction::AuthnReauthFail,
            FailureSource::Reauth,
        )
        .await?;
        return Err(lock.map_or_else(
            || ApiError::new(ErrorCode::InvalidCurrentPassword),
            lock_error,
        ));
    }
    let outcome = outcome.expect("verified password has an outcome");

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let locked = lock_user_row(&mut transaction, principal.user.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;

    // Close the same race as login: a lock established after the pre-check
    // still refuses the step-up.
    if let Some(seconds) = known_account_lock_seconds(&mut *transaction, locked.id)
        .await
        .map_err(dependency_unavailable)?
    {
        transaction.rollback().await.ok();
        return Err(login_locked(seconds));
    }
    if locked.token_version != principal.user.token_version
        || locked.password_hash != principal.user.password_hash
    {
        transaction.rollback().await.ok();
        return Err(ApiError::session_invalid());
    }
    if !locked.is_active() {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::AccountDisabled));
    }

    let authenticated_at = touch_authenticated_at(&mut transaction, principal.session.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;

    reset_login_failures(&mut transaction, locked.id)
        .await
        .map_err(dependency_unavailable)?;

    if let Some(replacement) = outcome.replacement_hash.as_deref() {
        replace_password_hash(
            &mut transaction,
            locked.id,
            &principal.user.password_hash,
            replacement,
        )
        .await
        .map_err(dependency_unavailable)?;
    }

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnReauthSuccess,
            AuditResult::Success,
            AuditActor::user(locked.id, locked.username.clone()),
            request_id.0,
        )
        .target(
            TargetType::User,
            Some(locked.id),
            Some(locked.username.clone()),
        )
        .detail("auth_method", "password")
        .detail("session_id", principal.session.id.to_string())
        .detail("auth_time", authenticated_at.timestamp())
        .ip(audit.client_ip),
        audit.user_agent,
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(&mut transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    transaction.commit().await.map_err(dependency_unavailable)?;

    let issued_at = Utc::now().timestamp();
    let (access_token, _claims) = services
        .jwt
        .sign_access_token(
            locked.id,
            principal.session.id,
            locked.token_version,
            authenticated_at.timestamp(),
            issued_at,
        )
        .map_err(|error| internal_error("access token signing failed", error))?;

    Ok(Json(ReauthenticateResponse {
        access_token,
        token_type: TOKEN_TYPE.to_owned(),
        expires_in: JWT_ACCESS_TOKEN_TTL_SECONDS,
        auth_time: authenticated_at.timestamp(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/change-password",
    tag = "Auth",
    operation_id = "changePassword",
    security(("bearerAuth" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (
            status = 200,
            description = "Password changed; other sessions revoked and the current refresh token rotated",
            body = ChangePasswordResponse,
            headers(
                ("Set-Cookie" = String, description = "Rotated host-only refresh cookie"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Account disabled",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 422,
            description = "Current password invalid, password policy violation or unchanged password",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 429,
            description = "Login throttled",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("Retry-After" = String, description = "Seconds until the next attempt"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 503,
            description = "Authentication service unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    principal: SessionPrincipal,
    ApiJson(payload): ApiJson<ChangePasswordRequest>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let audit = AuditContext {
        request_id: request_id.0,
        user_agent: user_agent(&headers),
        client_ip: client_ip.0,
    };

    // The account and IP layers guard this password check; the global circuit
    // covers login and reauthentication only (spec §8.3).
    ensure_change_password_layers_unlocked(services, &state, audit.client_ip, principal.user.id)
        .await?;

    let outcome =
        verify_password(services, &payload.current_password, Some(&principal.user)).await?;
    if !outcome.as_ref().is_some_and(|outcome| outcome.valid) {
        let lock = record_counted_failure(
            &state,
            services,
            FailureIdentity {
                user: Some(&principal.user),
                identifier_key: None,
                identifier_hmac: None,
            },
            &audit,
            AuditAction::AuthnPasswordChangeFail,
            FailureSource::ChangePassword,
        )
        .await?;
        return Err(lock.map_or_else(
            || ApiError::new(ErrorCode::InvalidCurrentPassword),
            lock_error,
        ));
    }

    // Spec §5.7 step 2 evaluates the policy before the unchanged check.
    if let Err(violation) = PasswordPolicy::new().validate(
        &payload.new_password,
        &[
            &principal.user.username,
            &principal.user.email,
            &principal.user.display_name,
        ],
    ) {
        return Err(
            ApiError::new(ErrorCode::PasswordPolicyViolation).with_errors(vec![FieldError {
                pointer: "/new_password".to_owned(),
                code: ErrorCode::PasswordPolicyViolation.as_str().to_owned(),
                params: Some(serde_json::json!({ "reason": violation.as_reason() })),
            }]),
        );
    }
    if passwords_equal(&payload.current_password, &payload.new_password) {
        return Err(ApiError::new(ErrorCode::PasswordUnchanged));
    }

    let new_hash = services
        .passwords
        .hash(&payload.new_password)
        .await
        .map_err(password_unavailable)?;

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    // Lock the user row first, matching login, reauthentication and
    // logout-all, so the token-version and session sweep are serialized per
    // user.
    let locked = lock_user_row(&mut transaction, principal.user.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;
    // A lock established after the pre-check still refuses the change.
    if let Some(seconds) = known_account_lock_seconds(&mut *transaction, locked.id)
        .await
        .map_err(dependency_unavailable)?
    {
        transaction.rollback().await.ok();
        return Err(login_locked(seconds));
    }
    if !locked.is_active() {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::AccountDisabled));
    }
    // A concurrent credential reset or password change invalidates the
    // verification performed above.
    if locked.token_version != principal.user.token_version
        || locked.password_hash != principal.user.password_hash
    {
        transaction.rollback().await.ok();
        return Err(ApiError::session_invalid());
    }

    // The session lock serializes this rotation against a concurrent refresh
    // of the same token family.
    let Some(session) = lock_active_session(&mut transaction, principal.session.id, locked.id)
        .await
        .map_err(dependency_unavailable)?
    else {
        transaction.rollback().await.ok();
        return Err(ApiError::session_invalid());
    };
    let now = database_clock_now(&mut transaction)
        .await
        .map_err(dependency_unavailable)?;

    let sessions_revoked = revoke_other_user_sessions(
        &mut transaction,
        locked.id,
        session.id,
        REVOKE_REASON_PASSWORD_CHANGE,
    )
    .await
    .map_err(dependency_unavailable)?;
    revoke_other_user_refresh_tokens(
        &mut transaction,
        locked.id,
        session.id,
        REVOKE_REASON_PASSWORD_CHANGE,
    )
    .await
    .map_err(dependency_unavailable)?;

    let Some(new_version) = complete_password_change(
        &mut transaction,
        locked.id,
        &principal.user.password_hash,
        &new_hash,
    )
    .await
    .map_err(dependency_unavailable)?
    else {
        transaction.rollback().await.ok();
        return Err(ApiError::session_invalid());
    };

    let authenticated_at = touch_authenticated_at(&mut transaction, session.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;

    // The response always carries a usable refresh cookie. Without a usable
    // presentation the old family is retired and a fresh root is issued.
    let rotated_refresh = match refresh_token_from_cookie(&headers) {
        Some(presented_raw) => {
            match rotate_current_refresh_token(
                &mut transaction,
                services,
                session.id,
                presented_raw,
                now,
                session.absolute_expires_at,
            )
            .await?
            {
                Some(raw) => raw,
                None => {
                    reset_current_refresh_token(
                        &mut transaction,
                        session.id,
                        session.absolute_expires_at,
                    )
                    .await?
                }
            }
        }
        None => {
            reset_current_refresh_token(&mut transaction, session.id, session.absolute_expires_at)
                .await?
        }
    };

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnPasswordChanged,
            AuditResult::Success,
            AuditActor::user(locked.id, locked.username.clone()),
            request_id.0,
        )
        .target(
            TargetType::User,
            Some(locked.id),
            Some(locked.username.clone()),
        )
        .detail("sessions_revoked_count", sessions_revoked as i64)
        .ip(audit.client_ip),
        audit.user_agent,
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(&mut transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    transaction.commit().await.map_err(dependency_unavailable)?;

    let (access_token, _claims) = services
        .jwt
        .sign_access_token(
            locked.id,
            session.id,
            new_version,
            authenticated_at.timestamp(),
            Utc::now().timestamp(),
        )
        .map_err(|error| internal_error("access token signing failed", error))?;

    let mut response = Json(ChangePasswordResponse {
        access_token,
        token_type: TOKEN_TYPE.to_owned(),
        expires_in: JWT_ACCESS_TOKEN_TTL_SECONDS,
    })
    .into_response();
    let max_age = (session.absolute_expires_at - now)
        .num_seconds()
        .clamp(0, REFRESH_COOKIE_MAX_AGE_SECONDS);
    response.headers_mut().insert(
        SET_COOKIE,
        refresh_cookie_with_max_age(&rotated_refresh, max_age),
    );
    Ok(response)
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    tag = "Auth",
    operation_id = "refreshToken",
    security(("refreshCookie" = [])),
    params((
        "X-Requested-With" = String,
        Header,
        description = "Required CSRF header; must be exactly `XMLHttpRequest`"
    )),
    request_body = RefreshRequest,
    responses(
        (
            status = 200,
            description = "Rotated refresh token; new access token and refresh cookie issued",
            body = RefreshResponse,
            headers(
                ("Set-Cookie" = String, description = "Rotated host-only refresh cookie"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 401,
            description = "Refresh token invalid, session invalid or reuse detected",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("Set-Cookie" = String, description = "Cleared refresh cookie on reuse"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Account disabled or CSRF validation failed",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Authentication service unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    _csrf: CsrfProtected,
    ApiJson(_payload): ApiJson<RefreshRequest>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let user_agent = user_agent(&headers);
    let presented_raw = refresh_token_from_cookie(&headers)
        .ok_or_else(|| ApiError::new(ErrorCode::RefreshTokenInvalid))?
        .to_owned();
    let presented_hash = hash_refresh_token(&presented_raw);

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let Some(context) = lock_refresh_context(&mut transaction, &presented_hash)
        .await
        .map_err(dependency_unavailable)?
    else {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::RefreshTokenInvalid));
    };

    // The database clock is authoritative for every expiry comparison. This is
    // read after the session lock, so it is not a pre-wait transaction-start
    // time.
    let now = database_clock_now(&mut transaction)
        .await
        .map_err(dependency_unavailable)?;

    // Refresh tokens carry no access-token `ver` claim. Every action that
    // bumps `users.token_version` also revokes the session, so the session
    // check below is the version-consistency check required by spec §5.2.
    //
    // Account state is checked before token state so a disabled user's next
    // refresh reliably returns 403 even when disabling also revoked sessions.
    if !context.user_is_active() {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::AccountDisabled));
    }
    if context.session_revoked_at.is_some()
        || context.idle_expires_at <= now
        || context.absolute_expires_at <= now
    {
        transaction.rollback().await.ok();
        return Err(ApiError::session_invalid());
    }
    if context.token.revoked_at.is_some() || context.token.expires_at <= now {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::RefreshTokenInvalid));
    }

    match context.token.used_at {
        None => {
            let successor_raw =
                rotate_refresh_token(&mut transaction, services, &context, &presented_raw, now)
                    .await?;
            transaction.commit().await.map_err(dependency_unavailable)?;
            refresh_response(services, &context, &successor_raw, now)
        }
        Some(used_at) if now - used_at <= Duration::seconds(REFRESH_GRACE_SECONDS) => {
            match walk_grace_chain(&mut transaction, services, &presented_raw, &context.token)
                .await?
            {
                Some(successor_raw) => {
                    // A grace retry writes nothing: no rotation, no audit and
                    // no activity update.
                    transaction.rollback().await.ok();
                    refresh_response(services, &context, &successor_raw, now)
                }
                None => {
                    let error = reject_refresh_reuse(
                        &mut transaction,
                        services,
                        &context,
                        client_ip.0,
                        request_id.0,
                        user_agent,
                    )
                    .await?;
                    transaction.commit().await.map_err(dependency_unavailable)?;
                    Err(error)
                }
            }
        }
        Some(_) => {
            let error = reject_refresh_reuse(
                &mut transaction,
                services,
                &context,
                client_ip.0,
                request_id.0,
                user_agent,
            )
            .await?;
            transaction.commit().await.map_err(dependency_unavailable)?;
            Err(error)
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "Auth",
    operation_id = "logout",
    security(("refreshCookie" = [])),
    params((
        "X-Requested-With" = String,
        Header,
        description = "Required CSRF header; must be exactly `XMLHttpRequest`"
    )),
    request_body = LogoutRequest,
    responses(
        (
            status = 204,
            description = "Current session revoked and refresh cookie cleared (idempotent)",
            headers(
                ("Set-Cookie" = String, description = "Cleared host-only refresh cookie"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "CSRF validation failed",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Authentication service unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    _csrf: CsrfProtected,
    ApiJson(_payload): ApiJson<LogoutRequest>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let Some(presented) = refresh_token_from_cookie(&headers) else {
        return Ok(logout_response());
    };
    let presented_hash = hash_refresh_token(presented);

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    // Locking the session serializes logout against a concurrent refresh: once
    // this transaction commits, the family can no longer rotate.
    let Some(context) = lock_refresh_context(&mut transaction, &presented_hash)
        .await
        .map_err(dependency_unavailable)?
    else {
        transaction.rollback().await.ok();
        return Ok(logout_response());
    };
    let now = database_clock_now(&mut transaction)
        .await
        .map_err(dependency_unavailable)?;
    if context.session_revoked_at.is_some()
        || context.idle_expires_at <= now
        || context.absolute_expires_at <= now
    {
        // A revoked or expired session is already terminal; the audit event is
        // written only when this call actually ended a usable session.
        transaction.rollback().await.ok();
        return Ok(logout_response());
    }

    let session_id = context.token.session_id;
    revoke_session_family(&mut transaction, session_id, REVOKE_REASON_LOGOUT)
        .await
        .map_err(dependency_unavailable)?;

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnLogout,
            AuditResult::Success,
            AuditActor::user(context.user_id, context.username.clone()),
            request_id.0,
        )
        .target(TargetType::Session, Some(session_id), None)
        .detail("session_id", session_id.to_string())
        .ip(client_ip.0),
        user_agent(&headers),
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(&mut transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    transaction.commit().await.map_err(dependency_unavailable)?;
    Ok(logout_response())
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout-all",
    tag = "Auth",
    operation_id = "logoutAll",
    security(("bearerAuth" = [])),
    params((
        "X-Requested-With" = String,
        Header,
        description = "Required CSRF header; must be exactly `XMLHttpRequest`"
    )),
    request_body = LogoutRequest,
    responses(
        (
            status = 204,
            description = "Every session revoked and the token version incremented; the refresh cookie is cleared",
            headers(
                ("Set-Cookie" = String, description = "Cleared host-only refresh cookie"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 401,
            description = "Access token or session invalid",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(
                ("WWW-Authenticate" = String, description = "Bearer challenge"),
                ("X-Request-Id" = String, description = "Server-generated request id")
            )
        ),
        (
            status = 403,
            description = "Account disabled, a password change is required or CSRF validation failed",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        ),
        (
            status = 503,
            description = "Authentication service unavailable",
            body = crate::error::ProblemDetails,
            content_type = "application/problem+json",
            headers(("X-Request-Id" = String, description = "Server-generated request id"))
        )
    )
)]
pub async fn logout_all(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    client_ip: ClientIp,
    _csrf: CsrfProtected,
    principal: Authenticated,
    ApiJson(_payload): ApiJson<LogoutRequest>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let principal = principal.0;
    let user_agent = user_agent(&headers);

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    // Locking the user row serializes logout-all against concurrent logins:
    // either their session commits before the sweep, or they read the bumped
    // token version after it.
    let locked = lock_user_row(&mut transaction, principal.user.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;
    if locked.token_version != principal.user.token_version {
        transaction.rollback().await.ok();
        return Err(ApiError::session_invalid());
    }
    if !locked.is_active() {
        transaction.rollback().await.ok();
        return Err(ApiError::new(ErrorCode::AccountDisabled));
    }

    let sessions_revoked =
        revoke_user_sessions(&mut transaction, locked.id, REVOKE_REASON_LOGOUT_ALL)
            .await
            .map_err(dependency_unavailable)?;
    revoke_user_refresh_tokens(&mut transaction, locked.id, REVOKE_REASON_LOGOUT_ALL)
        .await
        .map_err(dependency_unavailable)?;
    bump_token_version(&mut transaction, locked.id)
        .await
        .map_err(dependency_unavailable)?;

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnLogoutAll,
            AuditResult::Success,
            AuditActor::user(locked.id, locked.username.clone()),
            request_id.0,
        )
        .target(
            TargetType::User,
            Some(locked.id),
            Some(locked.username.clone()),
        )
        .detail("sessions_revoked_count", sessions_revoked as i64)
        .ip(client_ip.0),
        user_agent,
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(&mut transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    transaction.commit().await.map_err(dependency_unavailable)?;
    Ok(logout_response())
}

/// 204 with a cleared refresh cookie. Logout never returns a body.
fn logout_response() -> Response {
    (
        StatusCode::NO_CONTENT,
        [(SET_COOKIE, clear_refresh_cookie())],
    )
        .into_response()
}

/// Rotate the refresh token presented by the browser during a self-service
/// password change. A token consumed by a concurrent refresh inside the grace
/// window is followed to the family's current tail, so the browser keeps
/// receiving a live cookie. Missing, revoked, expired or out-of-grace values
/// return `None`; the caller then retires the family and mints a fresh root.
async fn rotate_current_refresh_token(
    transaction: &mut Transaction<'_, Postgres>,
    services: &AuthServices,
    session_id: Uuid,
    presented_raw: &str,
    now: DateTime<Utc>,
    absolute_expires_at: DateTime<Utc>,
) -> Result<Option<String>, ApiError> {
    let presented_hash = hash_refresh_token(presented_raw);
    let Some(token) = find_refresh_token(transaction, &presented_hash)
        .await
        .map_err(dependency_unavailable)?
    else {
        return Ok(None);
    };
    if token.session_id != session_id || token.revoked_at.is_some() || token.expires_at <= now {
        return Ok(None);
    }

    let (tail, tail_raw) = match token.used_at {
        None => (token, presented_raw.to_owned()),
        Some(used_at) if now - used_at <= Duration::seconds(REFRESH_GRACE_SECONDS) => {
            let Some(tail_raw) =
                walk_grace_chain(transaction, services, presented_raw, &token).await?
            else {
                return Ok(None);
            };
            let tail_hash = hash_refresh_token(&tail_raw);
            let Some(tail) = find_refresh_token(transaction, &tail_hash)
                .await
                .map_err(dependency_unavailable)?
            else {
                return Ok(None);
            };
            if tail.used_at.is_some() || tail.revoked_at.is_some() || tail.expires_at <= now {
                return Ok(None);
            }
            (tail, tail_raw)
        }
        Some(_) => return Ok(None),
    };

    rotate_token(
        transaction,
        services,
        &tail,
        &tail_raw,
        now,
        absolute_expires_at,
    )
    .await
}

/// Derive the successor of `raw`, mark the presented token row used and
/// persist the successor's SHA-256. Callers hold the session lock. Returns
/// `None` when the row was already consumed or revoked.
async fn rotate_token(
    transaction: &mut Transaction<'_, Postgres>,
    services: &AuthServices,
    token: &RefreshTokenRow,
    raw: &str,
    now: DateTime<Utc>,
    absolute_expires_at: DateTime<Utc>,
) -> Result<Option<String>, ApiError> {
    let counter = token.rotation_counter + 1;
    let successor_raw = derive_refresh_token(&services.refresh_derivation.current, raw, counter);
    let successor_hash = hash_refresh_token(&successor_raw);
    let marked =
        mark_refresh_token_rotated(transaction, &hash_refresh_token(raw), &successor_hash, now)
            .await
            .map_err(dependency_unavailable)?;
    if !marked {
        return Ok(None);
    }
    insert_refresh_token(
        transaction,
        NewRefreshToken {
            token_hash: &successor_hash,
            session_id: token.session_id,
            rotation_counter: counter,
            expires_at: absolute_expires_at,
        },
    )
    .await
    .map_err(dependency_unavailable)?;
    Ok(Some(successor_raw))
}

/// Replace a session's refresh family with a fresh root when no usable token
/// was presented. The new token has no predecessor, so stale values fail as
/// invalid rather than as reuse.
async fn reset_current_refresh_token(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    absolute_expires_at: DateTime<Utc>,
) -> Result<String, ApiError> {
    revoke_refresh_family(transaction, session_id, REVOKE_REASON_PASSWORD_CHANGE)
        .await
        .map_err(dependency_unavailable)?;
    let counter = next_rotation_counter(transaction, session_id)
        .await
        .map_err(dependency_unavailable)?;
    let raw = generate_refresh_token()
        .map_err(|error: TokenError| internal_error("refresh token generation failed", error))?;
    let hash = hash_refresh_token(&raw);
    insert_refresh_token(
        transaction,
        NewRefreshToken {
            token_hash: &hash,
            session_id,
            rotation_counter: counter,
            expires_at: absolute_expires_at,
        },
    )
    .await
    .map_err(dependency_unavailable)?;
    Ok(raw)
}

/// Rotate an unused token: mark it used, link it to the HMAC-derived
/// successor, persist the successor's SHA-256 and slide the idle deadline.
/// Returns the raw successor; the caller commits.
async fn rotate_refresh_token(
    transaction: &mut Transaction<'_, Postgres>,
    services: &AuthServices,
    context: &RefreshContext,
    presented_raw: &str,
    now: DateTime<Utc>,
) -> Result<String, ApiError> {
    let Some(successor_raw) = rotate_token(
        transaction,
        services,
        &context.token,
        presented_raw,
        now,
        context.absolute_expires_at,
    )
    .await?
    else {
        // The session lock serializes rotations, so this only happens if
        // another writer bypassed the refresh path. Fail closed as invalid.
        return Err(ApiError::new(ErrorCode::RefreshTokenInvalid));
    };

    // Activity updates are throttled so a chatty client cannot turn every
    // refresh into a session write (spec §5.3).
    if now - context.last_seen_at >= Duration::seconds(SESSION_ACTIVITY_TOUCH_SECONDS) {
        touch_session_activity(
            transaction,
            context.token.session_id,
            now,
            now + Duration::seconds(SESSION_IDLE_SECONDS),
        )
        .await
        .map_err(dependency_unavailable)?;
    }

    Ok(successor_raw)
}

/// Walk `replaced_by_hash` from a token used inside its grace window and return
/// the raw value of the current tail token. Any missing, revoked or
/// inconsistent step returns `None`: refresh treats that as reuse, while a
/// password change retires the family and mints a fresh root.
async fn walk_grace_chain(
    transaction: &mut Transaction<'_, Postgres>,
    services: &AuthServices,
    presented_raw: &str,
    presented: &RefreshTokenRow,
) -> Result<Option<String>, ApiError> {
    let mut current_raw = presented_raw.to_owned();
    let mut current_counter = presented.rotation_counter;
    let mut current_successor = presented.replaced_by_hash.clone();

    // The chain is acyclic: every step increments the counter, and a step is
    // accepted only when the HMAC-derived token hashes to the stored pointer.
    // A cycle would require an HMAC collision, so the walk terminates at the
    // family's current (unused) token.
    loop {
        let Some(expected) = current_successor.as_deref() else {
            return Ok(None);
        };
        let counter = current_counter + 1;
        let Some(next_raw) = derive_matching_successor(services, &current_raw, counter, expected)
        else {
            return Ok(None);
        };
        let next_hash = hash_refresh_token(&next_raw);
        let Some(next) = find_refresh_token(transaction, &next_hash)
            .await
            .map_err(dependency_unavailable)?
        else {
            return Ok(None);
        };
        if next.session_id != presented.session_id
            || next.rotation_counter != counter
            || next.revoked_at.is_some()
        {
            return Ok(None);
        }
        if next.used_at.is_none() {
            return Ok(Some(next_raw));
        }
        current_raw = next_raw;
        current_counter = next.rotation_counter;
        current_successor = next.replaced_by_hash;
    }
}

/// Recompute a successor with the current derivation key, then the previous
/// key during a key rotation window, and accept it only when its SHA-256
/// matches the stored `replaced_by_hash`.
fn derive_matching_successor(
    services: &AuthServices,
    previous_raw: &str,
    counter: i64,
    expected: &[u8],
) -> Option<String> {
    services
        .refresh_derivation
        .candidates()
        .map(|key| derive_refresh_token(key, previous_raw, counter))
        .find(|candidate| hash_refresh_token(candidate).as_slice() == expected)
}

/// Revoke the whole token family and the session, write the critical audit
/// event and clear the browser cookie. `token_version` is deliberately left
/// unchanged: the blast radius is this session only (spec §5.2).
async fn reject_refresh_reuse(
    transaction: &mut Transaction<'_, Postgres>,
    services: &AuthServices,
    context: &RefreshContext,
    client_ip: IpAddr,
    request_id: Uuid,
    user_agent: Option<String>,
) -> Result<ApiError, ApiError> {
    revoke_session_family(
        transaction,
        context.token.session_id,
        REVOKE_REASON_TOKEN_REUSE,
    )
    .await
    .map_err(dependency_unavailable)?;

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnRefreshReuseDetected,
            AuditResult::Failure,
            AuditActor::user(context.user_id, context.username.clone()),
            request_id,
        )
        .target(TargetType::Session, Some(context.token.session_id), None)
        .detail("session_id", context.token.session_id.to_string())
        .detail("rotation_counter", context.token.rotation_counter)
        .detail("family_revoked", true)
        .ip(client_ip),
        user_agent,
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    let mut error = ApiError::new(ErrorCode::RefreshReuseDetected);
    error = error.with_header(SET_COOKIE, clear_refresh_cookie());
    Ok(error)
}

/// Sign a fresh access token for the same session and attach the rotated
/// refresh cookie. The response never contains the user or the raw token in
/// JSON.
fn refresh_response(
    services: &AuthServices,
    context: &RefreshContext,
    raw_refresh: &str,
    now: DateTime<Utc>,
) -> Result<Response, ApiError> {
    let (access_token, _claims) = services
        .jwt
        .sign_access_token(
            context.user_id,
            context.token.session_id,
            context.token_version,
            context.authenticated_at.timestamp(),
            Utc::now().timestamp(),
        )
        .map_err(|error| internal_error("access token signing failed", error))?;

    let max_age = (context.absolute_expires_at - now)
        .num_seconds()
        .clamp(0, REFRESH_COOKIE_MAX_AGE_SECONDS);

    Ok((
        StatusCode::OK,
        [(
            SET_COOKIE,
            refresh_cookie_with_max_age(raw_refresh, max_age),
        )],
        Json(RefreshResponse {
            access_token,
            token_type: TOKEN_TYPE.to_owned(),
            expires_in: JWT_ACCESS_TOKEN_TTL_SECONDS,
        }),
    )
        .into_response())
}

/// Verify a login or reauthentication password. Unknown identifiers and
/// over-long inputs pay for a dummy Argon2 verification so the response cannot
/// reveal whether the account exists; a known identifier pays for exactly one
/// verification against its stored hash.
async fn verify_password(
    services: &AuthServices,
    password: &str,
    user: Option<&AuthUser>,
) -> Result<Option<VerifyOutcome>, ApiError> {
    if exceeds_authentication_limit(password) {
        services
            .passwords
            .verify_dummy()
            .await
            .map_err(password_unavailable)?;
        return Ok(None);
    }

    match user {
        Some(user) => Ok(Some(
            services
                .passwords
                .verify(password, &user.password_hash)
                .await
                .map_err(password_unavailable)?,
        )),
        None => {
            services
                .passwords
                .verify_dummy()
                .await
                .map_err(password_unavailable)?;
            Ok(None)
        }
    }
}

async fn find_user(
    database: &Database,
    identifier: Option<&LoginIdentifier>,
) -> Result<Option<AuthUser>, ApiError> {
    match identifier {
        Some(LoginIdentifier::Email(email)) => find_user_by_email(database.pool(), email)
            .await
            .map_err(dependency_unavailable),
        Some(LoginIdentifier::Username(username)) => {
            find_user_by_username(database.pool(), username)
                .await
                .map_err(dependency_unavailable)
        }
        None => Ok(None),
    }
}

async fn complete_login(
    state: &AppState,
    services: &AuthServices,
    user: AuthUser,
    replacement_hash: Option<String>,
    audit: AuditContext,
) -> Result<Response, ApiError> {
    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let locked = lock_user_row(&mut transaction, user.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;

    // A concurrent failure may have locked the account after the pre-check but
    // before this transaction acquired the row. The lock always wins, even
    // over a correct password (spec §8.3).
    if let Some(seconds) = known_account_lock_seconds(&mut *transaction, locked.id)
        .await
        .map_err(dependency_unavailable)?
    {
        transaction.rollback().await.ok();
        return Err(login_locked(seconds));
    }

    // A concurrent credential reset invalidates the verification we just
    // performed.
    if locked.password_hash != user.password_hash {
        transaction.rollback().await.ok();
        let lock = record_counted_failure(
            state,
            services,
            FailureIdentity {
                user: Some(&user),
                identifier_key: None,
                identifier_hmac: None,
            },
            &audit,
            AuditAction::AuthnLoginFail,
            FailureSource::Login,
        )
        .await?;
        return Err(lock.map_or_else(|| ApiError::new(ErrorCode::InvalidCredentials), lock_error));
    }

    // Account-state refusals are only revealed after a successful password
    // check, and never increment the failure counter.
    if !locked.is_active() {
        transaction.rollback().await.ok();
        record_refused_login(
            state,
            services,
            &locked,
            "account_disabled",
            AuditResult::Denied,
            &audit,
        )
        .await?;
        return Err(ApiError::new(ErrorCode::AccountDisabled));
    }

    // The database clock is authoritative for the expiry comparison and is
    // reused as the session authentication time.
    let authenticated_at = transaction_now(&mut transaction)
        .await
        .map_err(dependency_unavailable)?;
    if locked.temporary_password_expired(authenticated_at) {
        transaction.rollback().await.ok();
        record_refused_login(
            state,
            services,
            &locked,
            "temporary_password_expired",
            AuditResult::Denied,
            &audit,
        )
        .await?;
        return Err(ApiError::new(ErrorCode::TemporaryPasswordExpired));
    }

    // Enforce the per-user session cap while the user row is locked, so two
    // concurrent logins cannot both observe a free slot. Evicted sessions are
    // revoked before the new one is inserted; their families die with them.
    let active_sessions = count_active_sessions(&mut transaction, locked.id)
        .await
        .map_err(dependency_unavailable)?;
    let excess = (active_sessions + 1 - MAX_ACTIVE_SESSIONS_PER_USER).max(0);
    let mut evicted_sessions = Vec::new();
    if excess > 0 {
        evicted_sessions = find_oldest_active_sessions(&mut transaction, locked.id, excess)
            .await
            .map_err(dependency_unavailable)?;
        for evicted_session_id in &evicted_sessions {
            revoke_session_family(
                &mut transaction,
                *evicted_session_id,
                REVOKE_REASON_EXCESS_SESSIONS,
            )
            .await
            .map_err(dependency_unavailable)?;
        }
    }

    let idle_expires_at = authenticated_at + Duration::seconds(SESSION_IDLE_SECONDS);
    let absolute_expires_at = authenticated_at + Duration::seconds(SESSION_ABSOLUTE_SECONDS);

    let refresh_token = generate_refresh_token()
        .map_err(|error: TokenError| internal_error("refresh token generation failed", error))?;
    let refresh_hash = hash_refresh_token(&refresh_token);

    let session_id = create_session(
        &mut transaction,
        NewSession {
            user_id: locked.id,
            idle_expires_at,
            absolute_expires_at,
            authenticated_at,
            ip: Some(audit.client_ip),
            user_agent: audit.user_agent.clone(),
        },
    )
    .await
    .map_err(dependency_unavailable)?;

    insert_refresh_token(
        &mut transaction,
        NewRefreshToken {
            token_hash: &refresh_hash,
            session_id,
            rotation_counter: 0,
            expires_at: absolute_expires_at,
        },
    )
    .await
    .map_err(dependency_unavailable)?;

    update_successful_login(&mut transaction, locked.id, replacement_hash.as_deref())
        .await
        .map_err(dependency_unavailable)?;
    // A successful login resets the account layer only. Stale
    // unknown-identifier rows for either login identifier are account-layer
    // state too.
    for identifier in [&locked.username, &locked.email] {
        let key = identifier_throttle_key(&services.throttle_key, identifier);
        clear_unknown_login_failures(&mut transaction, &key)
            .await
            .map_err(dependency_unavailable)?;
    }

    let roles = load_role_grants(&mut *transaction, locked.id)
        .await
        .map_err(dependency_unavailable)?;
    let effective = resolve_effective_permissions(&roles);

    for evicted_session_id in &evicted_sessions {
        let event = with_user_agent(
            AuditEvent::builder(
                AuditAction::AuthnSessionEvicted,
                AuditResult::Success,
                AuditActor::user(locked.id, locked.username.clone()),
                audit.request_id,
            )
            .target(TargetType::Session, Some(*evicted_session_id), None)
            .detail("evicted_session_id", evicted_session_id.to_string())
            .detail("new_session_id", session_id.to_string())
            .ip(audit.client_ip),
            audit.user_agent.clone(),
        )
        .build()
        .map_err(audit_event_invalid)?;
        services
            .audit
            .write_in_tx(&mut transaction, &event)
            .await
            .map_err(audit_unavailable)?;
    }

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnLoginSuccess,
            AuditResult::Success,
            AuditActor::user(locked.id, locked.username.clone()),
            audit.request_id,
        )
        .target(TargetType::Session, Some(session_id), None)
        .detail("auth_method", "password")
        .detail("session_id", session_id.to_string())
        .detail("must_change_password", locked.must_change_password)
        .ip(audit.client_ip),
        audit.user_agent,
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(&mut transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    transaction.commit().await.map_err(dependency_unavailable)?;

    let (access_token, _claims) = services
        .jwt
        .sign_access_token(
            locked.id,
            session_id,
            locked.token_version,
            authenticated_at.timestamp(),
            Utc::now().timestamp(),
        )
        .map_err(|error| internal_error("access token signing failed", error))?;

    let body = LoginResponse {
        access_token,
        token_type: TOKEN_TYPE.to_owned(),
        expires_in: JWT_ACCESS_TOKEN_TTL_SECONDS,
        user: user_self(&locked, &roles, &effective),
    };

    let cookie = refresh_cookie(&refresh_token);
    Ok((StatusCode::OK, [(SET_COOKIE, cookie)], Json(body)).into_response())
}

/// Which account or IP row a counted failure belongs to.
struct FailureIdentity<'a> {
    /// Known user, or `None` for an unknown identifier.
    user: Option<&'a AuthUser>,
    /// Throttle key for the unknown-identifier account row; required when
    /// `user` is `None`.
    identifier_key: Option<&'a [u8; 32]>,
    /// HMAC recorded on the audit event for unknown identifiers.
    identifier_hmac: Option<&'a str>,
}

#[derive(Clone, Copy)]
enum FailureSource {
    Login,
    Reauth,
    ChangePassword,
}

impl FailureSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Reauth => "reauth",
            Self::ChangePassword => "change_password",
        }
    }

    /// The global circuit deliberately covers login and reauthentication only,
    /// not self-service password changes (spec §8.3).
    const fn counts_global(self) -> bool {
        matches!(self, Self::Login | Self::Reauth)
    }
}

/// Per-request audit fields shared by every event written for one attempt.
#[derive(Clone, Debug)]
struct AuditContext {
    request_id: Uuid,
    user_agent: Option<String>,
    client_ip: IpAddr,
}

/// A refusal caused by a lock, ready to become a 429 problem response.
struct LockRefusal {
    code: ErrorCode,
    retry_after_seconds: i64,
}

fn lock_error(refusal: LockRefusal) -> ApiError {
    let seconds = refusal.retry_after_seconds.max(1).to_string();
    let value = HeaderValue::from_str(&seconds).expect("retry seconds are header-safe");
    ApiError::new(refusal.code).with_header(RETRY_AFTER, value)
}

fn rate_limited(retry_after_seconds: i64) -> ApiError {
    lock_error(LockRefusal {
        code: ErrorCode::RateLimited,
        retry_after_seconds,
    })
}

fn login_locked(retry_after_seconds: i64) -> ApiError {
    lock_error(LockRefusal {
        code: ErrorCode::LoginLocked,
        retry_after_seconds,
    })
}

/// Enforce the IP and global layers before any password hashing so a locked
/// shared layer refuses cheaply. An elapsed global circuit is announced and
/// cleared here, giving the required release alert.
async fn ensure_shared_layers_unlocked(
    services: &AuthServices,
    state: &AppState,
    client_ip: IpAddr,
) -> Result<(), ApiError> {
    if release_expired_global_throttle(state.database.pool())
        .await
        .map_err(dependency_unavailable)?
    {
        // Application log fields are restricted to the ADR-0005 §9.2
        // whitelist; the message alone is the alert.
        tracing::warn!("global login throttle released");
    }

    // Blocked attempts return 429 without another audit row. The catalog
    // defines the lock events as threshold-crossing moments, and one row per
    // blocked request would serialize every login on the audit chain head
    // (exactly the DoS amplification the spec warns about).

    let ip_key = ip_throttle_key(&services.throttle_key, client_ip);
    let global_key = global_throttle_key(&services.throttle_key);
    let locks = load_login_throttle_locks(state.database.pool(), &ip_key, &global_key)
        .await
        .map_err(dependency_unavailable)?;
    metrics::gauge!("auth_global_circuit_open").set(
        if locks.global_retry_after_seconds.is_some() {
            1.0
        } else {
            0.0
        },
    );
    match locks.retry_after_seconds() {
        Some(seconds) => Err(rate_limited(seconds)),
        None => Ok(()),
    }
}

/// Enforce the IP and account layers before hashing a current password for a
/// self-service change. The global circuit does not cover this endpoint, so
/// its state is neither read nor cleared here.
async fn ensure_change_password_layers_unlocked(
    services: &AuthServices,
    state: &AppState,
    client_ip: IpAddr,
    user_id: Uuid,
) -> Result<(), ApiError> {
    let ip_key = ip_throttle_key(&services.throttle_key, client_ip);
    // A locked IP layer refuses without hashing (spec §8.3); only the account
    // layer pays for a dummy verification while locked.
    if let Some(seconds) = load_throttle_lock(state.database.pool(), ThrottleScope::Ip, &ip_key)
        .await
        .map_err(dependency_unavailable)?
    {
        return Err(rate_limited(seconds));
    }
    if let Some(seconds) = known_account_lock_seconds(state.database.pool(), user_id)
        .await
        .map_err(dependency_unavailable)?
    {
        services
            .passwords
            .verify_dummy()
            .await
            .map_err(password_unavailable)?;
        return Err(login_locked(seconds));
    }
    Ok(())
}

/// Count one bad credential at the account and IP layers — plus the global
/// layer for login and reauthentication — and write the failure plus any
/// threshold audit events in the same transaction. Returns a refusal when one
/// of the counted layers is (or just became) locked.
async fn record_counted_failure(
    state: &AppState,
    services: &AuthServices,
    identity: FailureIdentity<'_>,
    audit: &AuditContext,
    audit_action: AuditAction,
    source: FailureSource,
) -> Result<Option<LockRefusal>, ApiError> {
    let settings = &services.settings;
    let ip_key = ip_throttle_key(&services.throttle_key, audit.client_ip);

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let account = match identity.user {
        Some(user) => {
            record_known_login_failure(&mut transaction, user.id, settings.account_failure_decay)
                .await
        }
        None => {
            let key = identity
                .identifier_key
                .expect("unknown identifiers carry a throttle key");
            record_unknown_login_failure(&mut transaction, key, settings.account_failure_decay)
                .await
        }
    }
    .map_err(dependency_unavailable)?;

    let ip = record_throttle_failure(
        &mut transaction,
        ThrottleScope::Ip,
        &ip_key,
        settings.login_ip_window,
        settings.login_ip_lock,
        i64::from(settings.login_ip_failure_limit),
    )
    .await
    .map_err(dependency_unavailable)?;
    let global = if source.counts_global() {
        let global_key = global_throttle_key(&services.throttle_key);
        Some(
            record_throttle_failure(
                &mut transaction,
                ThrottleScope::Global,
                &global_key,
                settings.login_global_window,
                settings.login_global_lock,
                i64::from(settings.login_global_circuit_limit),
            )
            .await
            .map_err(dependency_unavailable)?,
        )
    } else {
        None
    };

    let (actor, target) = match identity.user {
        Some(user) => (
            AuditActor::user(user.id, user.username.clone()),
            (TargetType::User, Some(user.id), Some(user.username.clone())),
        ),
        None => (AuditActor::anonymous(), (TargetType::System, None, None)),
    };
    let (lock_target_type, lock_target_id, lock_target_label) =
        (target.0, target.1, target.2.clone());

    // The failure itself. Unknown identifiers record only a keyed HMAC.
    let mut failure = AuditEvent::builder(
        audit_action,
        AuditResult::Failure,
        actor.clone(),
        audit.request_id,
    )
    .target(target.0, target.1, target.2)
    .detail("reason_code", "bad_credentials")
    .ip(audit.client_ip);
    if let Some(identifier_hmac) = identity.identifier_hmac {
        failure = failure.detail("identifier_hmac", identifier_hmac);
    }
    write_audit_event(
        &mut transaction,
        services,
        failure,
        audit.user_agent.clone(),
    )
    .await?;

    // An account-layer threshold reached by this failure.
    if let FailureRecord::Counted {
        failed_count,
        retry_after_seconds: Some(seconds),
    } = account
    {
        let lock = AuditEvent::builder(
            AuditAction::AuthnLoginLock,
            AuditResult::Denied,
            actor.clone(),
            audit.request_id,
        )
        .target(lock_target_type, lock_target_id, lock_target_label)
        .detail("lock_seconds", seconds)
        .detail("failure_count", failed_count)
        .detail("source", source.as_str())
        .detail("retry_after_seconds", seconds)
        .ip(audit.client_ip);
        write_audit_event(&mut transaction, services, lock, audit.user_agent.clone()).await?;
        metrics::counter!("auth_login_lockouts_total", "scope" => "account").increment(1);
    }

    // The first failure that crosses an IP/global lock threshold.
    let shared = std::iter::once((ThrottleScope::Ip, ip))
        .chain(global.map(|record| (ThrottleScope::Global, record)));
    for (scope, record) in shared {
        if let FailureRecord::Counted {
            failed_count,
            retry_after_seconds: Some(seconds),
        } = record
        {
            let event = AuditEvent::builder(
                AuditAction::AuthnLoginRateLimited,
                AuditResult::Denied,
                actor.clone(),
                audit.request_id,
            )
            .detail("scope", scope.as_str())
            .detail("lock_seconds", seconds)
            .detail("failure_count", failed_count)
            .detail("retry_after_seconds", seconds)
            .ip(audit.client_ip);
            write_audit_event(&mut transaction, services, event, audit.user_agent.clone()).await?;
            metrics::counter!("auth_login_lockouts_total", "scope" => scope.as_str()).increment(1);
            if scope == ThrottleScope::Global {
                metrics::gauge!("auth_global_circuit_open").set(1.0);
            }
        }
    }

    if let Some(FailureRecord::Counted { failed_count, .. }) = global
        && i64::from(failed_count) == i64::from(settings.login_global_alert_limit)
    {
        // ADR-0005 §9.2 restricts application log fields; the alert is the
        // stable message itself.
        tracing::warn!("global login failure alert threshold reached");
    }

    transaction.commit().await.map_err(dependency_unavailable)?;

    // IP/global refusals take precedence over an account lock so the client
    // backs off once for the whole address or service.
    let shared_retry = [retry_seconds(&ip), global.as_ref().and_then(retry_seconds)]
        .into_iter()
        .flatten()
        .max();
    Ok(match shared_retry {
        Some(seconds) => Some(LockRefusal {
            code: ErrorCode::RateLimited,
            retry_after_seconds: seconds,
        }),
        None => retry_seconds(&account).map(|seconds| LockRefusal {
            code: ErrorCode::LoginLocked,
            retry_after_seconds: seconds,
        }),
    })
}

/// Audit a refusal that is revealed only after a correct password (disabled
/// account, expired temporary password). These do not touch any counter.
async fn record_refused_login(
    state: &AppState,
    services: &AuthServices,
    user: &AuthUser,
    reason: &'static str,
    result: AuditResult,
    audit: &AuditContext,
) -> Result<(), ApiError> {
    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let event = AuditEvent::builder(
        AuditAction::AuthnLoginFail,
        result,
        AuditActor::user(user.id, user.username.clone()),
        audit.request_id,
    )
    .target(TargetType::User, Some(user.id), Some(user.username.clone()))
    .detail("reason_code", reason)
    .ip(audit.client_ip);
    write_audit_event(&mut transaction, services, event, audit.user_agent.clone()).await?;

    transaction.commit().await.map_err(dependency_unavailable)
}

pub(crate) async fn write_audit_event(
    transaction: &mut Transaction<'_, Postgres>,
    services: &AuthServices,
    builder: AuditEventBuilder,
    user_agent: Option<String>,
) -> Result<(), ApiError> {
    let event = with_user_agent(builder, user_agent)
        .build()
        .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(transaction, &event)
        .await
        .map_err(audit_unavailable)?;
    Ok(())
}

fn retry_seconds(record: &FailureRecord) -> Option<i64> {
    match *record {
        FailureRecord::Counted {
            retry_after_seconds,
            ..
        } => retry_after_seconds,
        FailureRecord::AlreadyLocked {
            retry_after_seconds,
        } => Some(retry_after_seconds),
    }
}

fn user_self(
    user: &AuthUser,
    roles: &[UserRole],
    effective: &EffectivePermissions,
) -> UserSelfResponse {
    UserSelfResponse {
        id: user.id.to_string(),
        username: user.username.clone(),
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        phone: user.phone.clone(),
        status: user.status.clone(),
        roles: roles
            .iter()
            .map(|role| RoleRef {
                id: role.id.to_string(),
                name: role.name.clone(),
                is_builtin: role.is_builtin,
            })
            .collect(),
        permissions: PermissionKey::ALL
            .iter()
            .filter(|permission| effective.contains(**permission))
            .map(|permission| permission.as_str().to_owned())
            .collect(),
        is_super_admin: effective.is_super_admin(),
        must_change_password: user.must_change_password,
        temporary_password_expires_at: user.temporary_password_expires_at.map(format_timestamp),
    }
}

/// Resolve built-in role permission sets from code and custom role permissions
/// from the persisted grant rows.
fn resolve_effective_permissions(roles: &[UserRole]) -> EffectivePermissions {
    let mut builtins = Vec::new();
    let mut custom: Vec<Vec<PermissionKey>> = Vec::new();

    for role in roles {
        match (role.is_builtin, BuiltInRole::from_code(&role.name)) {
            (true, Some(builtin)) => builtins.push(builtin),
            _ => custom.push(
                role.permission_keys
                    .iter()
                    // Catalog entries removed in a later release may still be
                    // persisted (sync never deletes them); they grant nothing.
                    .filter_map(|key| PermissionKey::from_str(key).ok())
                    .collect(),
            ),
        }
    }

    EffectivePermissions::from_grants(
        builtins
            .into_iter()
            .map(RoleGrant::BuiltIn)
            .chain(custom.iter().map(|keys| RoleGrant::Custom(keys.as_slice()))),
    )
}

fn normalize_identifier(raw: &str) -> Option<LoginIdentifier> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('@') {
        Email::parse(trimmed)
            .ok()
            .map(|email| LoginIdentifier::Email(email.as_str().to_owned()))
    } else {
        Username::parse(trimmed)
            .ok()
            .map(|username| LoginIdentifier::Username(username.as_str().to_owned()))
    }
}

enum LoginIdentifier {
    Username(String),
    Email(String),
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

pub(crate) fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(crate) fn with_user_agent(
    builder: AuditEventBuilder,
    user_agent: Option<String>,
) -> AuditEventBuilder {
    match user_agent {
        Some(user_agent) => builder.user_agent(user_agent),
        None => builder,
    }
}

fn refresh_cookie(raw_token: &str) -> HeaderValue {
    refresh_cookie_with_max_age(raw_token, REFRESH_COOKIE_MAX_AGE_SECONDS)
}

/// Refresh cookie with a lifetime capped by the session's absolute expiry
/// (spec §5.2).
fn refresh_cookie_with_max_age(raw_token: &str, max_age: i64) -> HeaderValue {
    let value = format!(
        "{REFRESH_COOKIE_NAME}={raw_token}; Path=/; Secure; HttpOnly; SameSite=Strict; \
         Max-Age={max_age}"
    );
    HeaderValue::from_str(&value).expect("refresh tokens are header-safe")
}

fn clear_refresh_cookie() -> HeaderValue {
    refresh_cookie_with_max_age("", 0)
}

/// Read the `__Host-refresh` value from the request's cookie header. Other
/// cookies are ignored.
fn refresh_token_from_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == REFRESH_COOKIE_NAME)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

pub(crate) fn dependency_unavailable(error: impl std::fmt::Display) -> ApiError {
    tracing::warn!(error = %error, "authentication dependency failed");
    ApiError::service_unavailable()
}

pub(crate) fn password_unavailable(error: PasswordError) -> ApiError {
    match error {
        PasswordError::Busy => {
            let mut error = ApiError::service_unavailable();
            error = error.with_header(
                axum::http::header::RETRY_AFTER,
                HeaderValue::from_static("2"),
            );
            error
        }
        other => internal_error("password hashing failed", other),
    }
}

pub(crate) fn audit_unavailable(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "audit write failed; failing closed");
    ApiError::service_unavailable()
}

pub(crate) fn audit_event_invalid(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "audit event is invalid");
    ApiError::new(ErrorCode::InternalError)
}

fn internal_error(context: &'static str, error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "{context}");
    ApiError::new(ErrorCode::InternalError)
}
