//! Break-glass recovery is visible through the HTTP contract: every old
//! session dies, the old password stops working and the new temporary
//! password lands in the restricted-session state (ADR-0003 §6,
//! docs/specs/login-system.md §5.7).

mod auth_support;

use auth_support::{
    app, auth_settings, change_password_request, json, login, password_service, problem_code,
    refresh_cookie, refresh_request, security_config, send,
};
use axum::http::StatusCode;
use rs_admin_cli::{
    BootstrapInput, CliServices, RecoverInput, bootstrap_super_admin, recover_super_admin,
};
use secrecy::SecretString;
use sqlx::PgPool;

const USERNAME: &str = "ops.admin";
const EMAIL: &str = "ops.admin@example.com";
const INITIAL_PASSWORD: &str = "initial bootstrap password 2026";
const CHOSEN_PASSWORD: &str = "the operator's chosen password 2026";
const RECOVERY_PASSWORD: &str = "recovery temporary password 2026";

fn cli_services() -> CliServices {
    CliServices::with_password_service(
        &security_config(),
        password_service().clone(),
        auth_settings().temp_password_ttl,
    )
}

#[sqlx::test(migrations = "../../migrations")]
async fn break_glass_recovery_invalidates_old_sessions_and_forces_a_password_change(pool: PgPool) {
    let services = cli_services();
    let initial_password = SecretString::from(INITIAL_PASSWORD.to_owned());
    bootstrap_super_admin(
        &pool,
        &services,
        BootstrapInput {
            email: EMAIL,
            username: USERNAME,
            password: &initial_password,
        },
    )
    .await
    .expect("bootstrap");
    let app = app(&pool).await;

    // The bootstrap credential is a restricted session.
    let (status, headers, body) = login(&app, USERNAME, INITIAL_PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["must_change_password"], true);
    let restricted_token = body["access_token"].as_str().expect("access token");
    let restricted_cookie = refresh_cookie(&headers);

    // Change the password so the account has a normal, fully usable session.
    let (status, _, body) = send(
        app.clone(),
        change_password_request(
            restricted_token,
            Some(&restricted_cookie),
            INITIAL_PASSWORD,
            CHOSEN_PASSWORD,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", json(&body));

    let (status, headers, body) = login(&app, USERNAME, CHOSEN_PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["must_change_password"], false);
    let old_token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let old_cookie = refresh_cookie(&headers);

    let recovery_password = SecretString::from(RECOVERY_PASSWORD.to_owned());
    let outcome = recover_super_admin(
        &pool,
        &services,
        RecoverInput {
            email: EMAIL,
            password: &recovery_password,
        },
    )
    .await
    .expect("recover");
    assert_eq!(
        outcome.sessions_revoked, 2,
        "the changed-password session and the later login are both revoked"
    );

    // Every access token issued before recovery is dead.
    let (status, _, body) = send(
        app.clone(),
        auth_support::authorized_request("GET", "/api/v1/auth/me", &old_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{}", json(&body));
    assert_eq!(problem_code(&json(&body)), "session_invalid");

    // The refresh cookie of that session is dead too.
    let (status, _, body) = send(app.clone(), refresh_request(Some(&old_cookie))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{}", json(&body));
    assert_eq!(problem_code(&json(&body)), "session_invalid");

    // The pre-recovery password no longer authenticates.
    let (status, _, body) = login(&app, USERNAME, CHOSEN_PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(problem_code(&body), "invalid_credentials");

    // The recovery password works and establishes a restricted session.
    let (status, _, body) = login(&app, USERNAME, RECOVERY_PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["must_change_password"], true);
    assert_eq!(body["user"]["is_super_admin"], true);
    assert_eq!(body["user"]["username"], USERNAME);
    let recovery_token = body["access_token"].as_str().expect("access token");

    // Restricted sessions may read their own profile ...
    let (status, _, body) = send(
        app.clone(),
        auth_support::authorized_request("GET", "/api/v1/auth/me", recovery_token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", json(&body));
    assert_eq!(json(&body)["must_change_password"], true);

    // ... but every other authenticated endpoint demands the password change.
    let (status, _, body) = send(
        app.clone(),
        auth_support::authorized_request(
            "POST",
            "/api/v1/auth/logout-all",
            recovery_token,
            Some(&serde_json::json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{}", json(&body));
    assert_eq!(problem_code(&json(&body)), "password_change_required");
}
