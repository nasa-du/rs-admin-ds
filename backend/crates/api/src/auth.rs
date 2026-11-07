//! Authentication endpoints: password login, the current-user endpoint and
//! step-up reauthentication (docs/specs/login-system.md §5.1–§5.8, §7.6).
//!
//! Login is the only place a refresh token is issued in this ticket. The raw
//! value is returned exclusively through the `__Host-refresh` cookie; the
//! database stores its SHA-256 digest. Access tokens are EdDSA JWTs kept in the
//! client's memory and never persisted.

use crate::error::ApiError;
use crate::json::ApiJson;
use crate::middleware::RequestId;
use crate::router::AppState;
use axum::Extension;
use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{AUTHORIZATION, SET_COOKIE, USER_AGENT};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rs_admin_audit::{
    AuditAction, AuditActor, AuditEvent, AuditEventBuilder, AuditResult, AuditWriter, TargetType,
};
use rs_admin_config::{AuthSettings, SecurityConfig};
use rs_admin_domain::ErrorCode;
use rs_admin_domain::identity::{Email, Username};
use rs_admin_domain::permission::PermissionKey;
use rs_admin_domain::rbac::{BuiltInRole, EffectivePermissions, RoleGrant};
use rs_admin_security::{
    HmacKey, JWT_ACCESS_TOKEN_TTL_SECONDS, JwtIssuer, PasswordError, PasswordService, TokenError,
    VerifyOutcome, exceeds_authentication_limit, generate_refresh_token, hash_refresh_token,
    identifier_hmac_hex,
};
use rs_admin_store::Database;
use rs_admin_store::audit::transaction_now;
use rs_admin_store::auth::{
    AuthSession, AuthUser, NewRefreshToken, NewSession, UserRole, create_session,
    find_user_by_email, find_user_by_id, find_user_by_username, insert_refresh_token,
    load_active_session, load_role_grants, lock_user_for_login, record_login_failure,
    replace_password_hash, reset_login_failures, touch_authenticated_at, update_successful_login,
};
use serde::{Deserialize, Serialize};
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

/// Largest accepted authentication request body (spec §8.1).
pub const AUTH_JSON_BODY_BYTES: usize = 1024;

const TOKEN_TYPE: &str = "Bearer";

/// Authentication services built once at startup.
#[derive(Clone)]
pub struct AuthServices {
    jwt: JwtIssuer,
    passwords: PasswordService,
    audit: AuditWriter,
    throttle_key: HmacKey,
}

impl AuthServices {
    /// Build the JWT codec, Argon2 service and audit writer from validated
    /// configuration. Fails startup when key material cannot be used.
    pub fn new(security: &SecurityConfig, auth: &AuthSettings) -> Result<Self, AuthServiceError> {
        let passwords = PasswordService::new(auth.password_hash_concurrency)?;
        Self::with_password_service(security, passwords)
    }

    /// Build the remaining services around an existing password service. This
    /// lets callers share one bounded hashing pool across components.
    pub fn with_password_service(
        security: &SecurityConfig,
        passwords: PasswordService,
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
        })
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

/// The authenticated principal produced by the per-request pipeline.
#[derive(Clone, Debug)]
pub struct Authenticated {
    pub user: AuthUser,
    pub session: AuthSession,
    pub roles: Vec<UserRole>,
    pub effective: EffectivePermissions,
}

impl FromRequestParts<AppState> for Authenticated {
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
pub struct RoleRefResponse {
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
    pub roles: Vec<RoleRefResponse>,
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

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "Auth",
    operation_id = "login",
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
            description = "Account disabled or temporary password expired",
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
    ApiJson(payload): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    let user_agent = user_agent(&headers);
    let identifier = payload.identifier.trim().to_owned();
    let normalized = normalize_identifier(&identifier);

    let user = find_user(&state.database, normalized.as_ref()).await?;

    let outcome = verify_password(services, &payload.password, user.as_ref()).await?;
    let password_matches = outcome.as_ref().is_some_and(|outcome| outcome.valid);

    match (user, password_matches) {
        (Some(user), true) => {
            let outcome = outcome.expect("verified password has an outcome");
            complete_login(
                &state,
                services,
                user,
                outcome.replacement_hash,
                request_id.0,
                user_agent,
            )
            .await
        }
        (Some(user), false) => {
            record_failed_login(
                &state,
                services,
                &user,
                LoginFailure {
                    request_id: request_id.0,
                    user_agent,
                    reason: "bad_credentials",
                    result: AuditResult::Failure,
                    increment_counter: true,
                },
            )
            .await?;
            Err(ApiError::new(ErrorCode::InvalidCredentials))
        }
        (None, _) => {
            // Throttle state is keyed by the normalized identifier when it
            // parses, otherwise by the trimmed input, so that case and Unicode
            // variants cannot be used to rotate the key.
            let hmac_identifier = match &normalized {
                Some(LoginIdentifier::Username(value)) | Some(LoginIdentifier::Email(value)) => {
                    value.clone()
                }
                None => identifier.clone(),
            };
            record_unknown_login_failure(
                &state,
                services,
                &hmac_identifier,
                request_id.0,
                user_agent,
            )
            .await?;
            Err(ApiError::new(ErrorCode::InvalidCredentials))
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
pub async fn get_me(principal: Authenticated) -> Json<UserSelfResponse> {
    Json(user_self(
        &principal.user,
        &principal.roles,
        &principal.effective,
    ))
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
    principal: Authenticated,
    ApiJson(payload): ApiJson<ReauthenticateRequest>,
) -> Result<Json<ReauthenticateResponse>, ApiError> {
    let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
    if principal.user.must_change_password {
        return Err(ApiError::new(ErrorCode::PasswordChangeRequired));
    }
    let user_agent = user_agent(&headers);

    let outcome = verify_password(services, &payload.password, Some(&principal.user)).await?;
    if !outcome.as_ref().is_some_and(|outcome| outcome.valid) {
        record_failed_reauth(&state, services, &principal.user, request_id.0, user_agent).await?;
        return Err(ApiError::new(ErrorCode::InvalidCurrentPassword));
    }
    let outcome = outcome.expect("verified password has an outcome");

    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let locked = lock_user_for_login(&mut transaction, principal.user.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;
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
        .detail("auth_time", authenticated_at.timestamp()),
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
    request_id: Uuid,
    user_agent: Option<String>,
) -> Result<Response, ApiError> {
    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    let locked = lock_user_for_login(&mut transaction, user.id)
        .await
        .map_err(dependency_unavailable)?
        .ok_or_else(ApiError::session_invalid)?;

    // A concurrent credential reset invalidates the verification we just
    // performed.
    if locked.password_hash != user.password_hash {
        transaction.rollback().await.ok();
        record_failed_login(
            state,
            services,
            &user,
            LoginFailure {
                request_id,
                user_agent,
                reason: "bad_credentials",
                result: AuditResult::Failure,
                increment_counter: true,
            },
        )
        .await?;
        return Err(ApiError::new(ErrorCode::InvalidCredentials));
    }

    // Account-state refusals are only revealed after a successful password
    // check, and never increment the failure counter.
    if !locked.is_active() {
        transaction.rollback().await.ok();
        record_failed_login(
            state,
            services,
            &locked,
            LoginFailure {
                request_id,
                user_agent,
                reason: "account_disabled",
                result: AuditResult::Denied,
                increment_counter: false,
            },
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
        record_failed_login(
            state,
            services,
            &locked,
            LoginFailure {
                request_id,
                user_agent,
                reason: "temporary_password_expired",
                result: AuditResult::Denied,
                increment_counter: false,
            },
        )
        .await?;
        return Err(ApiError::new(ErrorCode::TemporaryPasswordExpired));
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
            ip: None,
            user_agent: user_agent.clone(),
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

    let roles = load_role_grants(&mut *transaction, locked.id)
        .await
        .map_err(dependency_unavailable)?;
    let effective = resolve_effective_permissions(&roles);

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnLoginSuccess,
            AuditResult::Success,
            AuditActor::user(locked.id, locked.username.clone()),
            request_id,
        )
        .target(TargetType::Session, Some(session_id), None)
        .detail("auth_method", "password")
        .detail("session_id", session_id.to_string())
        .detail("must_change_password", locked.must_change_password),
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

struct LoginFailure {
    request_id: Uuid,
    user_agent: Option<String>,
    reason: &'static str,
    result: AuditResult,
    increment_counter: bool,
}

async fn record_failed_login(
    state: &AppState,
    services: &AuthServices,
    user: &AuthUser,
    failure: LoginFailure,
) -> Result<(), ApiError> {
    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;

    if failure.increment_counter {
        record_login_failure(&mut transaction, user.id)
            .await
            .map_err(dependency_unavailable)?;
    }

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnLoginFail,
            failure.result,
            AuditActor::user(user.id, user.username.clone()),
            failure.request_id,
        )
        .target(TargetType::User, Some(user.id), Some(user.username.clone()))
        .detail("reason_code", failure.reason),
        failure.user_agent,
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(&mut transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    transaction.commit().await.map_err(dependency_unavailable)
}

async fn record_unknown_login_failure(
    state: &AppState,
    services: &AuthServices,
    identifier: &str,
    request_id: Uuid,
    user_agent: Option<String>,
) -> Result<(), ApiError> {
    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnLoginFail,
            AuditResult::Failure,
            AuditActor::anonymous(),
            request_id,
        )
        .target(TargetType::System, None, None)
        .detail("reason_code", "bad_credentials")
        .detail(
            "identifier_hmac",
            identifier_hmac_hex(&services.throttle_key, identifier),
        ),
        user_agent,
    )
    .build()
    .map_err(audit_event_invalid)?;

    services
        .audit
        .write(state.database.pool(), &event)
        .await
        .map_err(audit_unavailable)?;
    Ok(())
}

async fn record_failed_reauth(
    state: &AppState,
    services: &AuthServices,
    user: &AuthUser,
    request_id: Uuid,
    user_agent: Option<String>,
) -> Result<(), ApiError> {
    let mut transaction = state
        .database
        .pool()
        .begin()
        .await
        .map_err(dependency_unavailable)?;
    record_login_failure(&mut transaction, user.id)
        .await
        .map_err(dependency_unavailable)?;

    let event = with_user_agent(
        AuditEvent::builder(
            AuditAction::AuthnReauthFail,
            AuditResult::Failure,
            AuditActor::user(user.id, user.username.clone()),
            request_id,
        )
        .target(TargetType::User, Some(user.id), Some(user.username.clone()))
        .detail("reason_code", "bad_credentials"),
        user_agent,
    )
    .build()
    .map_err(audit_event_invalid)?;
    services
        .audit
        .write_in_tx(&mut transaction, &event)
        .await
        .map_err(audit_unavailable)?;

    transaction.commit().await.map_err(dependency_unavailable)
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
            .map(|role| RoleRefResponse {
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

fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
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

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn with_user_agent(builder: AuditEventBuilder, user_agent: Option<String>) -> AuditEventBuilder {
    match user_agent {
        Some(user_agent) => builder.user_agent(user_agent),
        None => builder,
    }
}

fn refresh_cookie(raw_token: &str) -> HeaderValue {
    let value = format!(
        "{REFRESH_COOKIE_NAME}={raw_token}; Path=/; Secure; HttpOnly; SameSite=Strict; \
         Max-Age={REFRESH_COOKIE_MAX_AGE_SECONDS}"
    );
    HeaderValue::from_str(&value).expect("refresh tokens are header-safe")
}

fn dependency_unavailable(error: impl std::fmt::Display) -> ApiError {
    tracing::warn!(error = %error, "authentication dependency failed");
    ApiError::service_unavailable()
}

fn password_unavailable(error: PasswordError) -> ApiError {
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

fn audit_unavailable(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "audit write failed; failing closed");
    ApiError::service_unavailable()
}

fn audit_event_invalid(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "audit event is invalid");
    ApiError::new(ErrorCode::InternalError)
}

fn internal_error(context: &'static str, error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "{context}");
    ApiError::new(ErrorCode::InternalError)
}
