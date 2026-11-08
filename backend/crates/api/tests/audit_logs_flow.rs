//! Read-only audit log query API behaviour over HTTP with a real PostgreSQL
//! schema (docs/specs/login-system.md §5.4, §7.9, §8.5).

mod auth_support;

use auth_support::{NewUser, PASSWORD, app, authorized_request, insert_user, login, problem_code};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::{DateTime, Duration, DurationRound, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn send_json(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let (status, headers, body) = auth_support::send(app, request).await;
    let value = if body.is_empty() {
        Value::Null
    } else {
        auth_support::json(&body)
    };
    (status, headers, value)
}

async fn login_token(app: &Router, username: &str) -> String {
    let (status, _headers, body) = login(app, username, PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "login failed: {body}");
    body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

async fn get(app: Router, uri: &str, token: &str) -> (StatusCode, HeaderMap, Value) {
    send_json(app, authorized_request("GET", uri, token, None)).await
}

/// An audit row inserted directly so tests control every query dimension.
/// This bypasses the writer (and thus the chain hash), which the read API
/// never consults.
struct AuditRowFixture {
    occurred_at: DateTime<Utc>,
    actor_user_id: Option<Uuid>,
    actor_label: String,
    action: String,
    target_type: String,
    target_id: Option<Uuid>,
    target_label: Option<String>,
    result: String,
    severity: String,
    ip: Option<String>,
    user_agent: Option<String>,
    request_id: Uuid,
    details: Value,
}

impl AuditRowFixture {
    fn new(action: impl Into<String>) -> Self {
        Self {
            occurred_at: Utc::now() - Duration::minutes(5),
            actor_user_id: Some(Uuid::now_v7()),
            actor_label: "alice".to_owned(),
            action: action.into(),
            target_type: "user".to_owned(),
            target_id: Some(Uuid::now_v7()),
            target_label: Some("bob".to_owned()),
            result: "success".to_owned(),
            severity: "info".to_owned(),
            ip: Some("192.0.2.10".to_owned()),
            user_agent: Some("test-agent/1.0".to_owned()),
            request_id: Uuid::now_v7(),
            details: json!({ "reason_code": "bad_credentials" }),
        }
    }

    fn at(mut self, occurred_at: DateTime<Utc>) -> Self {
        self.occurred_at = occurred_at;
        self
    }

    fn actor(mut self, id: Option<Uuid>, label: &str) -> Self {
        self.actor_user_id = id;
        self.actor_label = label.to_owned();
        self
    }

    fn target(mut self, target_type: &str, id: Option<Uuid>) -> Self {
        self.target_type = target_type.to_owned();
        self.target_id = id;
        self
    }

    fn result(mut self, result: &str) -> Self {
        self.result = result.to_owned();
        self
    }

    fn severity(mut self, severity: &str) -> Self {
        self.severity = severity.to_owned();
        self
    }

    fn request_id(mut self, request_id: Uuid) -> Self {
        self.request_id = request_id;
        self
    }

    fn ip(mut self, ip: &str) -> Self {
        self.ip = Some(ip.to_owned());
        self
    }

    fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}

async fn insert_audit_row(pool: &PgPool, fixture: AuditRowFixture) -> Uuid {
    let id = Uuid::now_v7();
    let chain_seq: i64 = sqlx::query_scalar(
        "UPDATE audit_chain_head
         SET last_seq = last_seq + 1,
             last_hash = decode(repeat('00', 32), 'hex'),
             updated_at = now()
         RETURNING last_seq",
    )
    .fetch_one(pool)
    .await
    .expect("advance chain head");

    sqlx::query(
        "INSERT INTO audit_logs (
            id, occurred_at, actor_user_id, actor_label, action, target_type,
            target_id, target_label, result, severity, ip, user_agent,
            request_id, details, chain_seq, prev_hash, row_hash, hmac_key_id
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::inet, $12,
            $13, $14, $15, decode(repeat('00', 32), 'hex'),
            decode(repeat('00', 32), 'hex'), 'test'
         )",
    )
    .bind(id)
    .bind(fixture.occurred_at)
    .bind(fixture.actor_user_id)
    .bind(&fixture.actor_label)
    .bind(&fixture.action)
    .bind(&fixture.target_type)
    .bind(fixture.target_id)
    .bind(fixture.target_label.as_deref())
    .bind(&fixture.result)
    .bind(&fixture.severity)
    .bind(fixture.ip.as_deref())
    .bind(fixture.user_agent.as_deref())
    .bind(fixture.request_id)
    .bind(fixture.details)
    .bind(chain_seq)
    .execute(pool)
    .await
    .expect("insert audit row");

    id
}

/// Remove login/audit noise so list assertions see only the fixture rows, and
/// reset the chain head so later writes start clean.
async fn reset_audit_log(pool: &PgPool) {
    sqlx::query("DELETE FROM audit_logs")
        .execute(pool)
        .await
        .expect("clear audit log");
    sqlx::query(
        "UPDATE audit_chain_head
         SET last_seq = 0, last_hash = decode(repeat('00', 32), 'hex'), updated_at = now()",
    )
    .execute(pool)
    .await
    .expect("reset chain head");
}

/// RFC3339 with a `Z` suffix; an unencoded `+00:00` would be decoded as a
/// space in a query string.
fn rfc3339(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn item_ids(body: &Value) -> Vec<&str> {
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().expect("id"))
        .collect()
}

// ---------------------------------------------------------------------------
// Permission gate
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn list_requires_audit_read_and_audits_the_refusal(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;

    let token = login_token(&app, "plain").await;
    let (status, _headers, body) = get(app.clone(), "/api/v1/audit-logs", &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(problem_code(&body), "forbidden");

    let denied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied' AND details->>'operation' = 'list_audit_logs'",
    )
    .fetch_one(&pool)
    .await
    .expect("count denials");
    assert_eq!(denied, 1, "missing audit.read refusal must be audited");

    // A plain admin holds audit.read.
    let token = login_token(&app, "admin1").await;
    let (status, _headers, body) = get(app, "/api/v1/audit-logs", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["items"].is_array(), "{body}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn unauthenticated_list_is_rejected(pool: PgPool) {
    let app = app(&pool).await;
    let (status, _headers, body) = get_without_token(app).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(problem_code(&body), "access_token_invalid");
}

async fn get_without_token(app: Router) -> (StatusCode, HeaderMap, Value) {
    let request = Request::builder()
        .uri("/api/v1/audit-logs")
        .body(Body::empty())
        .expect("request");
    send_json(app, request).await
}

// ---------------------------------------------------------------------------
// Default window, ordering and response shape
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn list_defaults_to_the_last_thirty_days_newest_first(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    let now = Utc::now();
    let recent = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").at(now - Duration::hours(1)),
    )
    .await;
    let older = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_updated").at(now - Duration::days(2)),
    )
    .await;
    insert_audit_row(
        &pool,
        AuditRowFixture::new("user_updated").at(now - Duration::days(40)),
    )
    .await;

    let (status, _headers, body) = get(app, "/api/v1/audit-logs", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["page"], 1);
    assert_eq!(body["page_size"], 20);
    assert_eq!(body["total"], 2);
    assert_eq!(
        item_ids(&body),
        [recent.to_string(), older.to_string()],
        "default sort is -occurred_at and the default window is 30 days"
    );

    let item = &body["items"][0];
    assert_eq!(item["action"], "user_created");
    assert_eq!(item["result"], "success");
    assert_eq!(item["severity"], "info");
    assert_eq!(item["target"]["type"], "user");
    assert!(item["target"].get("label").is_some());
    assert!(item["actor"].get("id").is_some());
    assert!(item["actor"].get("label").is_some());
    assert!(item["request_id"].is_string());
    assert!(item["ip"].is_string());
    assert!(
        item["occurred_at"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z')),
        "wire timestamps are RFC3339 UTC: {item}"
    );
    assert!(
        item.get("details").is_none(),
        "summary must not carry details"
    );
    assert!(
        item.get("user_agent").is_none(),
        "summary must not carry user_agent"
    );
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn list_filters_by_every_documented_dimension(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    let now = Utc::now() - Duration::hours(1);
    let alice = Uuid::now_v7();
    let target_user = Uuid::now_v7();
    let target_role = Uuid::now_v7();
    let shared_request = Uuid::now_v7();

    let created = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created")
            .at(now)
            .actor(Some(alice), "alice")
            .target("user", Some(target_user))
            .request_id(shared_request)
            .ip("192.0.2.10")
            .severity("info")
            .result("success")
            .details(json!({ "username": "bob" })),
    )
    .await;
    let failed_login = insert_audit_row(
        &pool,
        AuditRowFixture::new("authn_login_fail")
            .at(now + Duration::seconds(1))
            .actor(None, "anonymous")
            .target("system", None)
            .request_id(Uuid::now_v7())
            .ip("2001:db8::1")
            .severity("warning")
            .result("failure"),
    )
    .await;
    let role_update = insert_audit_row(
        &pool,
        AuditRowFixture::new("role_updated")
            .at(now + Duration::seconds(2))
            .actor(Some(alice), "alice")
            .target("role", Some(target_role))
            .request_id(shared_request)
            .ip("198.51.100.7")
            .severity("warning")
            .result("success"),
    )
    .await;

    let cases = [
        (format!("actor_id={alice}"), vec![role_update, created]),
        ("action=user_created".to_owned(), vec![created]),
        ("target_type=role".to_owned(), vec![role_update]),
        (format!("target_id={target_user}"), vec![created]),
        ("result=failure".to_owned(), vec![failed_login]),
        (
            "severity=warning".to_owned(),
            vec![role_update, failed_login],
        ),
        (
            format!("request_id={shared_request}"),
            vec![role_update, created],
        ),
        ("ip=192.0.2.10".to_owned(), vec![created]),
        // IPv4-mapped IPv6 input is normalized before matching.
        ("ip=::ffff:192.0.2.10".to_owned(), vec![created]),
        // IPv6 input is compared after canonical compression.
        ("ip=2001:0DB8:0:0:0:0:0:1".to_owned(), vec![failed_login]),
        // Unknown catalog values are tolerated and simply match nothing.
        ("action=future_event".to_owned(), vec![]),
        ("target_type=future_target".to_owned(), vec![]),
    ];

    for (query, expected) in cases {
        let (status, _headers, body) =
            get(app.clone(), &format!("/api/v1/audit-logs?{query}"), &token).await;
        assert_eq!(status, StatusCode::OK, "{query}: {body}");
        let expected: Vec<String> = expected.iter().map(Uuid::to_string).collect();
        assert_eq!(item_ids(&body), expected, "filter {query}: {body}");
        assert_eq!(body["total"], expected.len() as i64, "filter {query}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_rejects_malformed_filter_values(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;

    for (query, pointer) in [
        ("actor_id=not-a-uuid", "/actor_id"),
        ("target_id=not-a-uuid", "/target_id"),
        ("request_id=not-a-uuid", "/request_id"),
        ("result=maybe", "/result"),
        ("severity=loud", "/severity"),
        ("ip=not-an-ip", "/ip"),
    ] {
        let (status, _headers, body) =
            get(app.clone(), &format!("/api/v1/audit-logs?{query}"), &token).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {body}");
        assert_eq!(problem_code(&body), "validation_failed", "{query}");
        assert_eq!(body["errors"][0]["pointer"], pointer, "{query}: {body}");
    }
}

// ---------------------------------------------------------------------------
// Sorting and pagination
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn list_sorts_only_by_occurred_at(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    let base = Utc::now() - Duration::hours(3);
    let oldest = insert_audit_row(&pool, AuditRowFixture::new("user_created").at(base)).await;
    let middle = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").at(base + Duration::hours(1)),
    )
    .await;
    let newest = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").at(base + Duration::hours(2)),
    )
    .await;

    let (status, _headers, body) =
        get(app.clone(), "/api/v1/audit-logs?sort=occurred_at", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        item_ids(&body),
        [oldest.to_string(), middle.to_string(), newest.to_string()]
    );

    let (status, _headers, body) =
        get(app.clone(), "/api/v1/audit-logs?sort=-occurred_at", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        item_ids(&body),
        [newest.to_string(), middle.to_string(), oldest.to_string()]
    );

    for sort in ["action", "-action", "severity", "occurred_at,id"] {
        let (status, _headers, body) = get(
            app.clone(),
            &format!("/api/v1/audit-logs?sort={sort}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{sort}: {body}");
        assert_eq!(problem_code(&body), "invalid_sort", "{sort}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn pagination_uses_a_total_order_when_timestamps_tie(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    // Every row shares occurred_at; the unique chain sequence is the only
    // tie-break that can keep offset pages stable.
    let occurred_at = Utc::now() - Duration::hours(1);
    let mut inserted = Vec::new();
    for _ in 0..5 {
        inserted.push(
            insert_audit_row(&pool, AuditRowFixture::new("user_created").at(occurred_at)).await,
        );
    }

    let mut seen = Vec::new();
    for page in 1..=3 {
        let (status, _headers, body) = get(
            app.clone(),
            &format!("/api/v1/audit-logs?page={page}&page_size=2"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["page"], page);
        assert_eq!(body["page_size"], 2);
        assert_eq!(body["total"], 5, "total describes the filtered set");
        seen.extend(item_ids(&body).into_iter().map(str::to_owned));
    }

    let expected: Vec<String> = inserted.iter().rev().map(Uuid::to_string).collect();
    assert_eq!(seen, expected, "newest-first means last chain row first");
    assert_eq!(
        seen.iter().collect::<std::collections::HashSet<_>>().len(),
        5,
        "pages must not repeat a row"
    );

    for query in ["page=0", "page_size=0", "page_size=201"] {
        let (status, _headers, body) =
            get(app.clone(), &format!("/api/v1/audit-logs?{query}"), &token).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {body}");
    }
}

// ---------------------------------------------------------------------------
// Absolute time window
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn time_window_is_from_inclusive_to_exclusive(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    let from: DateTime<Utc> = (Utc::now() - Duration::days(10))
        .duration_trunc(chrono::Duration::seconds(1))
        .expect("truncate");
    let to = from + Duration::hours(1);

    insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").at(from - Duration::milliseconds(1)),
    )
    .await;
    let at_from = insert_audit_row(&pool, AuditRowFixture::new("user_created").at(from)).await;
    let inside = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").at(from + Duration::minutes(30)),
    )
    .await;
    insert_audit_row(&pool, AuditRowFixture::new("user_created").at(to)).await;
    insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").at(to + Duration::milliseconds(1)),
    )
    .await;

    let query = format!(
        "/api/v1/audit-logs?from={}&to={}&page_size=200",
        rfc3339(from),
        rfc3339(to)
    );
    let (status, _headers, body) = get(app, &query, &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        item_ids(&body),
        [inside.to_string(), at_from.to_string()],
        "from is inclusive, to is exclusive"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn invalid_or_oversized_time_ranges_are_rejected(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;

    let now = Utc::now();
    let cases = [
        ("from=yesterday".to_owned(), "invalid_time_range"),
        (
            format!("from={}&to={}", rfc3339(now), rfc3339(now)),
            "invalid_time_range",
        ),
        (
            format!(
                "from={}&to={}",
                rfc3339(now),
                rfc3339(now - Duration::seconds(1))
            ),
            "invalid_time_range",
        ),
        (
            format!(
                "from={}&to={}",
                rfc3339(now),
                rfc3339(now + Duration::days(91))
            ),
            "time_range_too_large",
        ),
    ];
    for (query, code) in cases {
        let (status, _headers, body) =
            get(app.clone(), &format!("/api/v1/audit-logs?{query}"), &token).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {body}");
        assert_eq!(problem_code(&body), code, "{query}: {body}");
    }

    // Exactly 90 days is allowed.
    let query = format!(
        "from={}&to={}",
        rfc3339(now),
        rfc3339(now + Duration::days(90))
    );
    let (status, _headers, body) = get(app, &format!("/api/v1/audit-logs?{query}"), &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn to_only_defaults_the_window_to_thirty_days(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    let to = Utc::now() - Duration::days(1);
    let inside = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").at(to - Duration::days(29)),
    )
    .await;
    insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").at(to - Duration::days(31)),
    )
    .await;

    let (status, _headers, body) = get(
        app,
        &format!("/api/v1/audit-logs?to={}", rfc3339(to)),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(item_ids(&body), [inside.to_string()], "{body}");
}

// ---------------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct AccessEvent {
    actor_user_id: Option<Uuid>,
    actor_label: String,
    target_type: String,
    target_id: Option<Uuid>,
    result: String,
    severity: String,
    request_id: Uuid,
    ip: Option<String>,
    details: Value,
}

async fn access_events(pool: &PgPool) -> Vec<AccessEvent> {
    sqlx::query_as(
        "SELECT actor_user_id, actor_label, target_type, target_id, result, severity,
                request_id, host(ip) AS ip, details
         FROM audit_logs
         WHERE action = 'audit_log_accessed'
         ORDER BY chain_seq",
    )
    .fetch_all(pool)
    .await
    .expect("access events")
}

#[sqlx::test(migrations = "../../migrations")]
async fn detail_returns_the_full_record_and_writes_audit_log_accessed(pool: PgPool) {
    let app = app(&pool).await;
    let reader = insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    let viewed = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created")
            .details(json!({ "username": "bob", "role_ids": ["role-1"] })),
    )
    .await;

    let (status, headers, body) =
        get(app.clone(), &format!("/api/v1/audit-logs/{viewed}"), &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], viewed.to_string());
    assert_eq!(body["action"], "user_created");
    assert_eq!(body["actor"]["label"], "alice");
    assert_eq!(body["target"]["type"], "user");
    assert_eq!(body["user_agent"], "test-agent/1.0");
    assert_eq!(body["details"]["username"], "bob");
    assert_eq!(body["details"]["role_ids"][0], "role-1");

    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("request id header");

    let events = access_events(&pool).await;
    assert_eq!(events.len(), 1, "one detail read writes exactly one event");
    let event = &events[0];
    assert_eq!(event.actor_user_id, Some(reader));
    assert_eq!(event.actor_label, "admin1");
    assert_eq!(event.target_type, "audit_log");
    assert_eq!(event.target_id, Some(viewed));
    assert_eq!(event.result, "success");
    assert_eq!(event.severity, "info");
    assert_eq!(event.request_id.to_string(), request_id);
    assert_eq!(event.ip.as_deref(), Some("127.0.0.1"));
    assert_eq!(event.details["view"], "detail");
    assert!(
        event.details.get("audit_log_id").is_none(),
        "the viewed id lives on target_id, not duplicated in details"
    );

    // Every read writes, including a repeat of the same detail.
    let (status, _headers, _body) =
        get(app.clone(), &format!("/api/v1/audit-logs/{viewed}"), &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(access_events(&pool).await.len(), 2);

    // A list query never writes an access event.
    let (status, _headers, _body) = get(app.clone(), "/api/v1/audit-logs", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(access_events(&pool).await.len(), 2);

    // Missing and malformed ids are 404 and do not write.
    for id in [Uuid::now_v7().to_string(), "not-a-uuid".to_owned()] {
        let (status, _headers, body) =
            get(app.clone(), &format!("/api/v1/audit-logs/{id}"), &token).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}: {body}");
        assert_eq!(problem_code(&body), "not_found");
    }
    assert_eq!(access_events(&pool).await.len(), 2);
}

#[sqlx::test(migrations = "../../migrations")]
async fn detail_requires_audit_read(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(&pool, NewUser::active("plain", "plain@example.com")).await;
    let token = login_token(&app, "plain").await;
    let id = Uuid::now_v7();

    let (status, _headers, body) = get(app, &format!("/api/v1/audit-logs/{id}"), &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(problem_code(&body), "forbidden");

    let denied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
         WHERE action = 'authz_denied' AND details->>'operation' = 'get_audit_log'",
    )
    .fetch_one(&pool)
    .await
    .expect("count denials");
    assert_eq!(denied, 1);
    assert!(access_events(&pool).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn audit_read_is_not_filtered_by_management_scope(pool: PgPool) {
    let app = app(&pool).await;
    let root = insert_user(
        &pool,
        NewUser {
            role: Some("super_admin"),
            ..NewUser::active("root1", "root1@example.com")
        },
    )
    .await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    // The super administrator is outside the admin's management scope, yet
    // both endpoints must expose the event.
    let event = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_roles_changed")
            .actor(Some(root), "root1")
            .target("user", Some(root)),
    )
    .await;

    let (status, _headers, body) = get(app.clone(), "/api/v1/audit-logs", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(item_ids(&body), [event.to_string()]);

    let (status, _headers, body) = get(app, &format!("/api/v1/audit-logs/{event}"), &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["actor"]["id"], root.to_string());
}

#[sqlx::test(migrations = "../../migrations")]
async fn detail_masks_forbidden_detail_keys(pool: PgPool) {
    let app = app(&pool).await;
    insert_user(
        &pool,
        NewUser {
            role: Some("admin"),
            ..NewUser::active("admin1", "admin1@example.com")
        },
    )
    .await;
    let token = login_token(&app, "admin1").await;
    reset_audit_log(&pool).await;

    let id = insert_audit_row(
        &pool,
        AuditRowFixture::new("user_created").details(json!({
            "password": "hunter2-unique",
            "nested": { "access_token": "token-unique-abc" },
            "permission_keys": ["users.read"],
            "key_type": "jwt",
        })),
    )
    .await;

    let (status, _headers, body) = get(app, &format!("/api/v1/audit-logs/{id}"), &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["details"]["password"], "***");
    assert_eq!(body["details"]["nested"]["access_token"], "***");
    assert_eq!(body["details"]["permission_keys"][0], "users.read");
    assert_eq!(body["details"]["key_type"], "jwt");

    let raw = body.to_string();
    assert!(!raw.contains("hunter2-unique"), "secret leaked: {raw}");
    assert!(!raw.contains("token-unique-abc"), "secret leaked: {raw}");
}
