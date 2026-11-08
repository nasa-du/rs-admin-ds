//! The committed OpenAPI document is the source of truth for the generated
//! TypeScript client. These assertions pin the contract surface introduced by
//! the walking skeleton so drift is caught by `cargo test` as well as by the
//! CI `git diff --exit-code` check.

use rs_admin_api::openapi_document;
use rs_admin_domain::ErrorCode;
use serde_json::Value;

fn document() -> Value {
    serde_json::to_value(openapi_document()).expect("serializable OpenAPI document")
}

#[test]
fn document_targets_openapi_31_and_names_the_service() {
    let doc = document();

    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(doc["info"]["title"], "rs-admin API");
    assert!(
        doc["info"]["version"]
            .as_str()
            .is_some_and(|version| !version.is_empty()),
        "info.version must be set"
    );
    // Health probes live at the root; business endpoints will document their
    // full `/api/v1/...` paths under the same server entry.
    assert_eq!(doc["servers"][0]["url"], "/");

    // Presence checks rather than an exact path set: later tickets add
    // business endpoints without having to rewrite this test. The committed
    // document and the CI diff remain the review point for new paths.
    for path in ["/health/live", "/health/ready"] {
        assert!(
            doc["paths"][path].is_object(),
            "missing documented path {path}"
        );
    }
}

#[test]
fn health_operations_have_stable_operation_ids_and_response_schemas() {
    let doc = document();
    let live = &doc["paths"]["/health/live"]["get"];
    let ready = &doc["paths"]["/health/ready"]["get"];

    // Generated client method names come from operationId; changing one is a
    // breaking change for consumers.
    assert_eq!(live["operationId"], "healthLive");
    assert_eq!(ready["operationId"], "healthReady");
    assert_eq!(live["tags"][0], "Health");
    assert_eq!(ready["tags"][0], "Health");

    for (operation, status) in [(live, "200"), (ready, "200"), (ready, "503")] {
        assert_eq!(
            operation["responses"][status]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/HealthStatus",
            "response {status} must use HealthStatus"
        );
        assert_eq!(
            operation["responses"][status]["headers"]["X-Request-Id"]["schema"]["type"], "string",
            "response {status} must document X-Request-Id"
        );
    }

    let schemas = &doc["components"]["schemas"];
    assert_eq!(
        schemas["HealthStatus"]["properties"]["status"]["$ref"],
        "#/components/schemas/HealthState"
    );
    let states: Vec<&str> = schemas["HealthState"]["enum"]
        .as_array()
        .expect("HealthState enum")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(states, ["live", "ready", "unready"]);
}

#[test]
fn auth_operations_are_documented_with_stable_ids_and_bearer_security() {
    let doc = document();

    let login = &doc["paths"]["/api/v1/auth/login"]["post"];
    assert_eq!(login["operationId"], "login");
    assert_eq!(login["tags"][0], "Auth");
    assert_eq!(
        login["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/LoginRequest"
    );
    assert_eq!(
        login["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/LoginResponse"
    );
    assert!(
        login["responses"]["200"]["headers"]["Set-Cookie"].is_object(),
        "login must document the refresh cookie header"
    );
    assert!(login.get("security").is_none(), "login is unauthenticated");
    assert_eq!(
        login["responses"]["401"]["content"]["application/problem+json"]["schema"]["$ref"],
        "#/components/schemas/ProblemDetails"
    );
    assert!(
        login["responses"]["401"]["headers"]["WWW-Authenticate"].is_object(),
        "401 must document the Bearer challenge"
    );
    assert!(
        login["responses"]["429"]["headers"]["Retry-After"].is_object(),
        "429 must document Retry-After"
    );

    for (path, method, operation_id, response_schema) in [
        (
            "/api/v1/auth/me",
            "get",
            "getMe",
            "#/components/schemas/UserSelfResponse",
        ),
        (
            "/api/v1/auth/reauthenticate",
            "post",
            "reauthenticate",
            "#/components/schemas/ReauthenticateResponse",
        ),
    ] {
        let operation = &doc["paths"][path][method];
        assert_eq!(operation["operationId"], operation_id, "{path}");
        assert_eq!(
            operation["security"][0]["bearerAuth"],
            serde_json::json!([]),
            "{path} must require bearerAuth"
        );
        assert_eq!(
            operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            response_schema
        );
    }

    let scheme = &doc["components"]["securitySchemes"]["bearerAuth"];
    assert_eq!(scheme["type"], "http");
    assert_eq!(scheme["scheme"], "bearer");
    assert_eq!(scheme["bearerFormat"], "JWT");
}

#[test]
fn refresh_is_documented_with_a_cookie_scheme_and_stable_operation_id() {
    let doc = document();
    let refresh = &doc["paths"]["/api/v1/auth/refresh"]["post"];

    assert_eq!(refresh["operationId"], "refreshToken");
    assert_eq!(refresh["tags"][0], "Auth");
    assert_eq!(
        refresh["security"][0]["refreshCookie"],
        serde_json::json!([]),
        "refresh authenticates by cookie"
    );
    assert_eq!(
        refresh["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/RefreshRequest"
    );
    assert_eq!(
        refresh["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/RefreshResponse"
    );
    assert!(
        refresh["responses"]["200"]["headers"]["Set-Cookie"].is_object(),
        "refresh must document the rotated cookie"
    );
    for status in ["401", "403"] {
        assert_eq!(
            refresh["responses"][status]["content"]["application/problem+json"]["schema"]["$ref"],
            "#/components/schemas/ProblemDetails",
            "response {status} must use the problem contract"
        );
    }
    assert!(
        refresh["responses"]["401"]["headers"]["WWW-Authenticate"].is_object(),
        "401 must document the Bearer challenge"
    );

    let scheme = &doc["components"]["securitySchemes"]["refreshCookie"];
    assert_eq!(scheme["type"], "apiKey");
    assert_eq!(scheme["in"], "cookie");
    assert_eq!(scheme["name"], "__Host-refresh");
}

#[test]
fn change_password_is_documented_with_a_stable_id_and_bearer_security() {
    let doc = document();
    let change = &doc["paths"]["/api/v1/auth/change-password"]["post"];

    assert_eq!(change["operationId"], "changePassword");
    assert_eq!(change["tags"][0], "Auth");
    assert_eq!(
        change["security"][0]["bearerAuth"],
        serde_json::json!([]),
        "change-password authenticates by access token"
    );
    assert_eq!(
        change["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/ChangePasswordRequest"
    );
    assert_eq!(
        change["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/ChangePasswordResponse"
    );
    assert!(
        change["responses"]["200"]["headers"]["Set-Cookie"].is_object(),
        "change-password documents the rotated cookie"
    );
    for status in ["401", "403", "422", "429", "503"] {
        assert_eq!(
            change["responses"][status]["content"]["application/problem+json"]["schema"]["$ref"],
            "#/components/schemas/ProblemDetails",
            "response {status} must use the problem contract"
        );
    }
    assert!(
        change["responses"]["401"]["headers"]["WWW-Authenticate"].is_object(),
        "401 must document the Bearer challenge"
    );
    assert!(
        change["responses"]["429"]["headers"]["Retry-After"].is_object(),
        "429 must document Retry-After"
    );
}

#[test]
fn logout_operations_are_documented_with_stable_ids_and_their_auth_scheme() {
    let doc = document();

    let logout = &doc["paths"]["/api/v1/auth/logout"]["post"];
    assert_eq!(logout["operationId"], "logout");
    assert_eq!(logout["tags"][0], "Auth");
    assert_eq!(
        logout["security"][0]["refreshCookie"],
        serde_json::json!([]),
        "logout authenticates by cookie"
    );
    assert_eq!(
        logout["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/LogoutRequest"
    );
    assert!(
        logout["responses"]["204"].is_object(),
        "logout documents the 204 outcome"
    );
    assert!(
        logout["responses"]["204"]["headers"]["Set-Cookie"].is_object(),
        "logout documents the cleared cookie"
    );

    let logout_all = &doc["paths"]["/api/v1/auth/logout-all"]["post"];
    assert_eq!(logout_all["operationId"], "logoutAll");
    assert_eq!(logout_all["tags"][0], "Auth");
    assert_eq!(
        logout_all["security"][0]["bearerAuth"],
        serde_json::json!([]),
        "logout-all authenticates by access token"
    );
    assert!(
        logout_all["responses"]["204"]["headers"]["Set-Cookie"].is_object(),
        "logout-all documents the cleared cookie"
    );
    for status in ["401", "403"] {
        assert_eq!(
            logout_all["responses"][status]["content"]["application/problem+json"]["schema"]["$ref"],
            "#/components/schemas/ProblemDetails",
            "response {status} must use the problem contract"
        );
    }
    assert!(
        logout_all["responses"]["401"]["headers"]["WWW-Authenticate"].is_object(),
        "401 must document the Bearer challenge"
    );
}

#[test]
fn problem_details_and_the_closed_error_catalog_are_in_the_schema() {
    let doc = document();
    let schemas = &doc["components"]["schemas"];

    let problem = &schemas["ProblemDetails"];
    assert_eq!(problem["properties"]["type"]["type"], "string");
    assert_eq!(problem["properties"]["status"]["type"], "integer");
    assert_eq!(
        problem["properties"]["errors"]["items"]["$ref"],
        "#/components/schemas/FieldError"
    );
    assert_eq!(
        problem["properties"]["code"]["$ref"],
        "#/components/schemas/ErrorCode"
    );

    let codes = schemas["ErrorCode"]["enum"]
        .as_array()
        .expect("ErrorCode must expose its closed catalog as an enum");
    let values: Vec<&str> = codes.iter().filter_map(Value::as_str).collect();

    assert_eq!(values.len(), ErrorCode::ALL.len());
    for code in ErrorCode::ALL {
        assert!(
            values.contains(&code.as_str()),
            "OpenAPI is missing error code {}",
            code.as_str()
        );
    }
    // Spot-check wire values independently of the Rust variant names.
    assert!(values.contains(&"validation_failed"));
    assert!(values.contains(&"service_unavailable"));
}
