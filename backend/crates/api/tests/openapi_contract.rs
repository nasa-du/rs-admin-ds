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
fn management_cli_commands_are_not_http_endpoints() {
    let doc = document();
    let paths = doc["paths"].as_object().expect("paths object");

    // `bootstrap-super-admin` and `recover-super-admin` are offline commands
    // (ADR-0003 §6); exposing either over HTTP is a security regression.
    for path in paths.keys() {
        assert!(
            !path.contains("bootstrap") && !path.contains("recover"),
            "management command leaked into the HTTP contract: {path}"
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
fn update_me_is_documented_with_a_stable_id_and_bearer_security() {
    let doc = document();
    let update = &doc["paths"]["/api/v1/auth/me"]["patch"];

    assert_eq!(update["operationId"], "updateMe");
    assert_eq!(update["tags"][0], "Auth");
    assert_eq!(
        update["security"][0]["bearerAuth"],
        serde_json::json!([]),
        "updateMe authenticates by access token"
    );
    assert_eq!(
        update["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/UpdateMeRequest"
    );
    assert_eq!(
        update["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/UserSelfResponse"
    );
    for status in ["401", "403", "422", "503"] {
        assert_eq!(
            update["responses"][status]["content"]["application/problem+json"]["schema"]["$ref"],
            "#/components/schemas/ProblemDetails",
            "response {status} must use the problem contract"
        );
        assert!(
            update["responses"][status]["headers"]["X-Request-Id"].is_object(),
            "response {status} must document X-Request-Id"
        );
    }
    assert!(
        update["responses"]["401"]["headers"]["WWW-Authenticate"].is_object(),
        "401 must document the Bearer challenge"
    );

    let schema = &doc["components"]["schemas"]["UpdateMeRequest"];
    assert_eq!(schema["properties"]["display_name"]["type"], "string");
    assert!(
        schema["properties"]["display_name"]
            .get("nullable")
            .is_none(),
        "display_name must not accept null"
    );
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
fn cookie_endpoints_document_the_required_csrf_header() {
    let doc = document();

    for path in [
        "/api/v1/auth/login",
        "/api/v1/auth/refresh",
        "/api/v1/auth/logout",
        "/api/v1/auth/logout-all",
    ] {
        let parameters = doc["paths"][path]["post"]["parameters"]
            .as_array()
            .unwrap_or_else(|| panic!("{path} must document its header parameters"));
        let header = parameters
            .iter()
            .find(|parameter| parameter["name"] == "X-Requested-With")
            .unwrap_or_else(|| panic!("{path} must document X-Requested-With"));
        assert_eq!(header["in"], "header", "{path}");
        assert_eq!(header["required"], true, "{path}");
        assert_eq!(header["schema"]["type"], "string", "{path}");
    }

    // These endpoints authenticate by Bearer only and require no CSRF header.
    for path in [
        "/api/v1/auth/reauthenticate",
        "/api/v1/auth/change-password",
    ] {
        assert!(
            doc["paths"][path]["post"]["parameters"].is_null(),
            "{path} must not require the CSRF header"
        );
    }
}

#[test]
fn logout_documents_the_csrf_refusal() {
    let doc = document();
    let logout = &doc["paths"]["/api/v1/auth/logout"]["post"];

    assert_eq!(
        logout["responses"]["403"]["content"]["application/problem+json"]["schema"]["$ref"],
        "#/components/schemas/ProblemDetails",
        "logout must document the 403 csrf_failed outcome"
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

#[test]
fn roles_and_permissions_operations_are_documented_with_stable_ids() {
    let doc = document();

    for (path, method, operation_id, tag, success, schema) in [
        (
            "/api/v1/roles",
            "get",
            "listRoles",
            "Roles",
            "200",
            "#/components/schemas/RoleListResponse",
        ),
        (
            "/api/v1/roles",
            "post",
            "createRole",
            "Roles",
            "201",
            "#/components/schemas/RoleDetail",
        ),
        (
            "/api/v1/roles/{id}",
            "get",
            "getRole",
            "Roles",
            "200",
            "#/components/schemas/RoleDetail",
        ),
        (
            "/api/v1/roles/{id}",
            "patch",
            "updateRole",
            "Roles",
            "200",
            "#/components/schemas/RoleDetail",
        ),
        (
            "/api/v1/roles/{id}/permissions",
            "put",
            "setRolePermissions",
            "Roles",
            "200",
            "#/components/schemas/RoleDetail",
        ),
        (
            "/api/v1/permissions",
            "get",
            "listPermissions",
            "Permissions",
            "200",
            "#/components/schemas/PermissionListResponse",
        ),
    ] {
        let operation = &doc["paths"][path][method];
        assert_eq!(operation["operationId"], operation_id, "{method} {path}");
        assert_eq!(operation["tags"][0], tag, "{method} {path}");
        assert_eq!(
            operation["security"][0]["bearerAuth"],
            serde_json::json!([]),
            "{method} {path} must require bearerAuth"
        );
        assert_eq!(
            operation["responses"][success]["content"]["application/json"]["schema"]["$ref"],
            schema,
            "{method} {path} response {success}"
        );
        assert!(
            operation["responses"][success]["headers"]["X-Request-Id"].is_object(),
            "{method} {path} must document X-Request-Id"
        );
    }

    let delete = &doc["paths"]["/api/v1/roles/{id}"]["delete"];
    assert_eq!(delete["operationId"], "deleteRole");
    assert_eq!(delete["tags"][0], "Roles");
    assert_eq!(
        delete["responses"]["204"]["headers"]["X-Request-Id"]["schema"]["type"],
        "string"
    );

    let create = &doc["paths"]["/api/v1/roles"]["post"];
    assert_eq!(
        create["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/CreateRoleRequest"
    );
    assert!(
        create["responses"]["201"]["headers"]["Location"].is_object(),
        "creation must document Location"
    );

    let list = &doc["paths"]["/api/v1/roles"]["get"];
    let params: Vec<&str> = list["parameters"]
        .as_array()
        .expect("list parameters")
        .iter()
        .filter_map(|parameter| parameter["name"].as_str())
        .collect();
    for expected in ["q", "is_builtin", "page", "page_size", "sort"] {
        assert!(
            params.contains(&expected),
            "missing query parameter {expected}"
        );
    }

    let update = &doc["paths"]["/api/v1/roles/{id}"]["patch"];
    assert_eq!(
        update["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/UpdateRoleRequest"
    );
    let set_permissions = &doc["paths"]["/api/v1/roles/{id}/permissions"]["put"];
    assert_eq!(
        set_permissions["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/SetRolePermissionsRequest"
    );

    // Every write documents the authorization, validation and reauthentication
    // codes, with the shared response headers.
    for (path, method) in [
        ("/api/v1/roles", "post"),
        ("/api/v1/roles/{id}", "patch"),
        ("/api/v1/roles/{id}", "delete"),
        ("/api/v1/roles/{id}/permissions", "put"),
    ] {
        let operation = &doc["paths"][path][method];
        for status in ["401", "403", "503"] {
            assert_eq!(
                operation["responses"][status]["content"]["application/problem+json"]["schema"]["$ref"],
                "#/components/schemas/ProblemDetails",
                "{method} {path} must document {status} as a problem"
            );
            assert!(
                operation["responses"][status]["headers"]["X-Request-Id"].is_object(),
                "{method} {path} {status} must document X-Request-Id"
            );
        }
        assert!(
            operation["responses"]["401"]["headers"]["WWW-Authenticate"].is_object(),
            "{method} {path} 401 must document the Bearer challenge"
        );
    }

    // Every error response on this surface carries the common headers:
    // X-Request-Id everywhere, plus the Bearer challenge on 401.
    for (path, operations) in doc["paths"].as_object().expect("paths") {
        if path != "/api/v1/permissions" && !path.starts_with("/api/v1/roles") {
            continue;
        }
        for (method, operation) in operations.as_object().expect("operations") {
            for (status, response) in operation["responses"].as_object().expect("responses") {
                if !(status.starts_with('4') || status.starts_with('5')) {
                    continue;
                }
                assert!(
                    response["headers"]["X-Request-Id"].is_object(),
                    "{method} {path} {status} must document X-Request-Id"
                );
                if status == "401" {
                    assert!(
                        response["headers"]["WWW-Authenticate"].is_object(),
                        "{method} {path} 401 must document the Bearer challenge"
                    );
                }
            }
        }
    }

    let schemas = &doc["components"]["schemas"];
    for name in [
        "RoleSummary",
        "RoleDetail",
        "RoleListResponse",
        "PermissionGroup",
        "PermissionItem",
        "PermissionListResponse",
    ] {
        assert!(
            schemas[name]["properties"].is_object() || schemas[name]["type"] == "object",
            "missing schema {name}"
        );
    }
    assert_eq!(
        schemas["RoleSummary"]["properties"]["permission_keys"]["items"]["type"],
        "string"
    );
    for flag in ["assignable", "manageable"] {
        assert_eq!(
            schemas["RoleSummary"]["properties"][flag]["type"], "boolean",
            "RoleSummary.{flag}"
        );
    }
}

#[test]
fn audit_log_operations_are_documented_with_stable_ids_and_catalog_schemas() {
    let doc = document();

    let list = &doc["paths"]["/api/v1/audit-logs"]["get"];
    assert_eq!(list["operationId"], "listAuditLogs");
    assert_eq!(list["tags"][0], "Audit Logs");
    assert_eq!(
        list["security"][0]["bearerAuth"],
        serde_json::json!([]),
        "listAuditLogs must require bearerAuth"
    );
    assert_eq!(
        list["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/AuditLogListResponse"
    );

    let parameters = list["parameters"].as_array().expect("query parameters");
    for name in [
        "actor_id",
        "action",
        "target_type",
        "target_id",
        "result",
        "severity",
        "request_id",
        "ip",
        "from",
        "to",
        "page",
        "page_size",
        "sort",
    ] {
        assert!(
            parameters
                .iter()
                .any(|parameter| parameter["name"] == name && parameter["in"] == "query"),
            "listAuditLogs must document the {name} query parameter"
        );
    }

    let detail = &doc["paths"]["/api/v1/audit-logs/{id}"]["get"];
    assert_eq!(detail["operationId"], "getAuditLog");
    assert_eq!(detail["tags"][0], "Audit Logs");
    assert_eq!(
        detail["security"][0]["bearerAuth"],
        serde_json::json!([]),
        "getAuditLog must require bearerAuth"
    );
    assert_eq!(
        detail["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/AuditLogDetail"
    );
    for status in ["401", "403", "404", "503"] {
        assert_eq!(
            detail["responses"][status]["content"]["application/problem+json"]["schema"]["$ref"],
            "#/components/schemas/ProblemDetails",
            "getAuditLog must document {status} as a problem"
        );
    }

    let schemas = &doc["components"]["schemas"];
    // `action` and `target_type` stay open strings with a documented catalog so
    // future values remain readable by older clients; `result` and `severity`
    // are closed enums (spec §7.9).
    assert_eq!(
        schemas["AuditLogSummary"]["properties"]["action"]["$ref"],
        "#/components/schemas/AuditAction"
    );
    assert_eq!(
        schemas["AuditTargetRef"]["properties"]["type"]["$ref"],
        "#/components/schemas/AuditTargetType"
    );
    assert_eq!(
        schemas["AuditAction"]["type"], "string",
        "action stays an open string"
    );
    assert!(
        schemas["AuditAction"].get("enum").is_none(),
        "action must not become a closed enum"
    );
    let action_description = schemas["AuditAction"]["description"]
        .as_str()
        .expect("AuditAction description");
    for action in [
        "authn_login_success",
        "user_roles_changed",
        "audit_log_accessed",
    ] {
        assert!(
            action_description.contains(action),
            "AuditAction must document the current catalog; missing {action}"
        );
    }
    assert_eq!(
        schemas["AuditTargetType"]["type"], "string",
        "target.type stays an open string"
    );
    assert!(
        schemas["AuditTargetType"].get("enum").is_none(),
        "target.type must not become a closed enum"
    );
    let target_description = schemas["AuditTargetType"]["description"]
        .as_str()
        .expect("AuditTargetType description");
    for target_type in ["user", "role", "session", "audit_log", "system"] {
        assert!(
            target_description.contains(target_type),
            "AuditTargetType must document the current catalog; missing {target_type}"
        );
    }

    let result_values: Vec<&str> = schemas["AuditResult"]["enum"]
        .as_array()
        .expect("AuditResult enum")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert_eq!(result_values, ["success", "failure", "denied"]);
    let severity_values: Vec<&str> = schemas["AuditSeverity"]["enum"]
        .as_array()
        .expect("AuditSeverity enum")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert_eq!(severity_values, ["info", "warning", "error", "critical"]);
    assert_eq!(
        schemas["AuditLogSummary"]["properties"]["result"]["$ref"],
        "#/components/schemas/AuditResult"
    );
    assert_eq!(
        schemas["AuditLogSummary"]["properties"]["severity"]["$ref"],
        "#/components/schemas/AuditSeverity"
    );

    // The same closed catalogs type the query parameters.
    for (name, schema) in [("result", "AuditResult"), ("severity", "AuditSeverity")] {
        let parameter = parameters
            .iter()
            .find(|parameter| parameter["name"] == name)
            .unwrap_or_else(|| panic!("missing {name} parameter"));
        assert_eq!(
            parameter["schema"]["$ref"],
            format!("#/components/schemas/{schema}"),
            "query parameter {name} must use the closed catalog"
        );
    }

    assert_eq!(
        schemas["AuditLogSummary"]["properties"]["target"]["$ref"],
        "#/components/schemas/AuditTargetRef"
    );
    assert_eq!(
        schemas["AuditLogSummary"]["properties"]["actor"]["$ref"],
        "#/components/schemas/AuditActorRef"
    );
    assert_eq!(
        schemas["AuditLogDetail"]["properties"]["details"]["type"], "object",
        "details is an open object"
    );
    assert!(
        schemas["AuditLogDetail"]["properties"]["details"]
            .get("additionalProperties")
            .is_none_or(|value| value != &serde_json::json!(false)),
        "details must allow arbitrary keys"
    );
}
