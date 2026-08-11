use axum::{
    Router,
    body::Body,
    http::{Method, Request, Response, StatusCode, header},
};
use chrono::Utc;
use http_body_util::BodyExt;
use pinglake_protocol::{
    AlertKind, AlertRecord, AlertSettings, EnrollRequest, HistoryPoint, MetricReport, NodeSnapshot,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

use super::{
    Config, build_app, hash_admin_password, is_public_webhook_ip, validate_settings,
    verify_admin_password, webhook_client,
};

#[test]
fn administrator_passwords_use_argon2_and_verify_exactly() {
    let encoded = hash_admin_password("correct horse battery staple").expect("hash password");
    assert!(encoded.starts_with("$argon2"));
    assert!(verify_admin_password(
        "correct horse battery staple",
        &encoded
    ));
    assert!(!verify_admin_password("wrong password", &encoded));
}

#[tokio::test]
async fn webhook_targets_reject_private_reserved_and_local_addresses() {
    for address in [
        "127.0.0.1",
        "10.0.0.1",
        "169.254.169.254",
        "192.168.1.1",
        "100.64.0.1",
        "::1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
    ] {
        assert!(!is_public_webhook_ip(address.parse().expect("IP address")));
    }
    for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
        assert!(is_public_webhook_ip(address.parse().expect("IP address")));
    }

    let localhost = reqwest::Url::parse("http://localhost/hook").expect("URL");
    assert!(webhook_client(&localhost).await.is_err());

    let mut settings = AlertSettings {
        webhook_enabled: true,
        webhook_url: "http://127.0.0.1/hook".to_owned(),
        ..AlertSettings::default()
    };
    assert!(validate_settings(&settings).is_err());
    settings.webhook_url = "https://user:secret@example.com/hook".to_owned();
    assert!(validate_settings(&settings).is_err());
}

#[test]
fn alert_settings_use_the_public_ui_contract() {
    let mut settings = AlertSettings::default();
    assert!(validate_settings(&settings).is_ok());

    settings.cpu_percent = 0.0;
    assert!(validate_settings(&settings).is_err());
    settings.cpu_percent = 1.0;
    settings.temperature_celsius = -100.0;
    settings.offline_after_seconds = 5;
    settings.sustained_for_seconds = 86_400;
    assert!(validate_settings(&settings).is_ok());

    settings.temperature_celsius = 251.0;
    assert!(validate_settings(&settings).is_err());
    settings.temperature_celsius = 85.0;
    settings.offline_after_seconds = 86_401;
    assert!(validate_settings(&settings).is_err());
}

struct TestContext {
    _directory: TempDir,
    database_path: std::path::PathBuf,
    app: Router,
}

impl TestContext {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("tempdir");
        let database_path = directory.path().join("pinglake.db");
        let (app, _) = build_app(Config::for_test(database_path.clone())).expect("build app");
        Self {
            _directory: directory,
            database_path,
            app,
        }
    }

    async fn login(&self) -> String {
        let response = send_json(
            &self.app,
            Method::POST,
            "/api/v1/auth/login",
            &[],
            json!({ "password": "test-admin-password" }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("session cookie")
            .to_str()
            .expect("ASCII cookie");
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Strict"));
        assert!(!set_cookie.contains("; Secure"));
        set_cookie
            .split(';')
            .next()
            .expect("cookie pair")
            .to_owned()
    }
}

#[tokio::test]
async fn management_routes_require_a_valid_random_session() {
    let context = TestContext::new();
    let health = send_empty(&context.app, Method::GET, "/api/healthz", &[]).await;
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(health.headers()["x-content-type-options"], "nosniff");
    assert_eq!(health.headers()["x-frame-options"], "DENY");
    assert_eq!(health.headers()["referrer-policy"], "same-origin");

    let unauthorized = send_empty(&context.app, Method::GET, "/api/v1/summary", &[]).await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let wrong_password = send_json(
        &context.app,
        Method::POST,
        "/api/v1/auth/login",
        &[],
        json!({ "password": "wrong" }),
    )
    .await;
    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);

    let oversized_login = send_json(
        &context.app,
        Method::POST,
        "/api/v1/auth/login",
        &[],
        json!({ "password": "x".repeat(5_000) }),
    )
    .await;
    assert_eq!(oversized_login.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let cookie = context.login().await;
    let authenticated = send_empty(
        &context.app,
        Method::GET,
        "/api/v1/auth/me",
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    assert_eq!(authenticated.status(), StatusCode::OK);

    let logout = send_empty(
        &context.app,
        Method::POST,
        "/api/v1/auth/logout",
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert!(
        logout
            .headers()
            .get(header::SET_COOKIE)
            .expect("clearing cookie")
            .to_str()
            .expect("ASCII cookie")
            .contains("Max-Age=0")
    );

    let expired = send_empty(
        &context.app,
        Method::GET,
        "/api/v1/auth/me",
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn enrollment_token_is_only_required_for_new_agent_ids() {
    let context = TestContext::new();
    let node_id = Uuid::new_v4();
    let mut request = enroll_request(node_id, "agent-secret");

    let missing_token = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/enroll",
        &[],
        &request,
    )
    .await;
    assert_eq!(missing_token.status(), StatusCode::UNAUTHORIZED);

    let enrolled = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/enroll",
        &[("x-enrollment-token", "test-enrollment-token")],
        &request,
    )
    .await;
    assert_eq!(enrolled.status(), StatusCode::OK);

    request.display_name = "Updated node name".to_owned();
    let reregistered_without_token = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/enroll",
        &[],
        &request,
    )
    .await;
    assert_eq!(reregistered_without_token.status(), StatusCode::OK);

    request.agent_secret = "attacker-secret".to_owned();
    let takeover = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/enroll",
        &[("x-enrollment-token", "test-enrollment-token")],
        &request,
    )
    .await;
    assert_eq!(takeover.status(), StatusCode::CONFLICT);

    let connection = rusqlite::Connection::open(&context.database_path).expect("open database");
    let stored_hash: String = connection
        .query_row(
            "SELECT secret_hash FROM nodes WHERE id = ?1",
            [node_id.to_string()],
            |row| row.get(0),
        )
        .expect("stored hash");
    assert_ne!(stored_hash, "agent-secret");
    assert_ne!(stored_hash, "attacker-secret");
    assert_eq!(stored_hash.len(), 64);

    let cookie = context.login().await;
    let nodes_response = send_empty(
        &context.app,
        Method::GET,
        "/api/v1/nodes",
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    let nodes: Vec<NodeSnapshot> = response_json(nodes_response).await;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].display_name, "Updated node name");
}

#[tokio::test]
async fn authenticated_metrics_are_persisted_and_returned_as_history() {
    let context = TestContext::new();
    let node_id = Uuid::new_v4();
    let request = enroll_request(node_id, "correct-secret");
    let enrolled = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/enroll",
        &[("x-enrollment-token", "test-enrollment-token")],
        &request,
    )
    .await;
    assert_eq!(enrolled.status(), StatusCode::OK);

    let server_time_before_report = Utc::now();
    let mut report = metric_report();
    report.collected_at = server_time_before_report - chrono::Duration::days(30);
    let node_id_text = node_id.to_string();
    let wrong_secret = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/metrics",
        &[
            ("x-agent-id", node_id_text.as_str()),
            (header::AUTHORIZATION.as_str(), "Bearer wrong-secret"),
        ],
        &report,
    )
    .await;
    assert_eq!(wrong_secret.status(), StatusCode::UNAUTHORIZED);

    let accepted = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/metrics",
        &[
            ("x-agent-id", node_id_text.as_str()),
            (header::AUTHORIZATION.as_str(), "Bearer correct-secret"),
        ],
        &report,
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    let cookie = context.login().await;
    let history_response = send_empty(
        &context.app,
        Method::GET,
        &format!("/api/v1/nodes/{node_id}/history?minutes=60"),
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    assert_eq!(history_response.status(), StatusCode::OK);
    let history: Vec<HistoryPoint> = response_json(history_response).await;
    assert_eq!(history.len(), 1);
    assert!(history[0].collected_at >= server_time_before_report - chrono::Duration::seconds(1));
    assert!(history[0].collected_at > report.collected_at + chrono::Duration::days(29));
    assert_eq!(history[0].cpu_percent, report.cpu_percent);
    assert_eq!(history[0].memory_used_bytes, report.memory_used_bytes);

    let nodes_response = send_empty(
        &context.app,
        Method::GET,
        "/api/v1/nodes",
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    let nodes: Vec<NodeSnapshot> = response_json(nodes_response).await;
    assert_eq!(nodes.len(), 1);
    assert!(nodes[0].online);
    assert!(nodes[0].last_seen_at.is_some());
    assert_eq!(
        nodes[0].latest.as_ref().expect("latest metric").cpu_percent,
        report.cpu_percent
    );
}

#[tokio::test]
async fn malformed_and_oversized_metric_reports_are_rejected() {
    let context = TestContext::new();
    let node_id = Uuid::new_v4();
    let request = enroll_request(node_id, "correct-secret");
    let enrolled = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/enroll",
        &[("x-enrollment-token", "test-enrollment-token")],
        &request,
    )
    .await;
    assert_eq!(enrolled.status(), StatusCode::OK);

    let node_id_text = node_id.to_string();
    let headers = [
        ("x-agent-id", node_id_text.as_str()),
        (header::AUTHORIZATION.as_str(), "Bearer correct-secret"),
    ];
    let mut invalid_usage = metric_report();
    invalid_usage.memory_used_bytes = invalid_usage.memory_total_bytes + 1;
    let invalid_usage_response = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/metrics",
        &headers,
        &invalid_usage,
    )
    .await;
    assert_eq!(invalid_usage_response.status(), StatusCode::BAD_REQUEST);

    let mut invalid_cpu = metric_report();
    invalid_cpu.cpu_percent = 101.0;
    let invalid_cpu_response = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/metrics",
        &headers,
        &invalid_cpu,
    )
    .await;
    assert_eq!(invalid_cpu_response.status(), StatusCode::BAD_REQUEST);

    let oversized = send(
        &context.app,
        Method::POST,
        "/api/v1/agent/metrics",
        &headers,
        Body::from(vec![b' '; 256 * 1024 + 1]),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let cookie = context.login().await;
    let history_response = send_empty(
        &context.app,
        Method::GET,
        &format!("/api/v1/nodes/{node_id}/history?minutes=60"),
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    let history: Vec<HistoryPoint> = response_json(history_response).await;
    assert!(history.is_empty());
}

#[tokio::test]
async fn threshold_alerts_are_deduplicated_and_recover() {
    let context = TestContext::new();
    let node_id = Uuid::new_v4();
    let request = enroll_request(node_id, "correct-secret");
    let enrolled = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/enroll",
        &[("x-enrollment-token", "test-enrollment-token")],
        &request,
    )
    .await;
    assert_eq!(enrolled.status(), StatusCode::OK);

    let cookie = context.login().await;
    let invalid_settings = AlertSettings {
        webhook_url: "ftp://invalid.example".to_owned(),
        ..AlertSettings::default()
    };
    let invalid_url = send_serialized(
        &context.app,
        Method::PUT,
        "/api/v1/settings",
        &[(header::COOKIE.as_str(), cookie.as_str())],
        &invalid_settings,
    )
    .await;
    assert_eq!(invalid_url.status(), StatusCode::BAD_REQUEST);

    let missing_webhook_url = AlertSettings {
        webhook_enabled: true,
        ..AlertSettings::default()
    };
    let missing_url = send_serialized(
        &context.app,
        Method::PUT,
        "/api/v1/settings",
        &[(header::COOKIE.as_str(), cookie.as_str())],
        &missing_webhook_url,
    )
    .await;
    assert_eq!(missing_url.status(), StatusCode::BAD_REQUEST);

    let settings = AlertSettings {
        cpu_percent: 10.0,
        sustained_for_seconds: 0,
        ..AlertSettings::default()
    };
    let updated = send_serialized(
        &context.app,
        Method::PUT,
        "/api/v1/settings",
        &[(header::COOKIE.as_str(), cookie.as_str())],
        &settings,
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);

    let node_id_text = node_id.to_string();
    let agent_headers = [
        ("x-agent-id", node_id_text.as_str()),
        (header::AUTHORIZATION.as_str(), "Bearer correct-secret"),
    ];
    for _ in 0..2 {
        let accepted = send_serialized(
            &context.app,
            Method::POST,
            "/api/v1/agent/metrics",
            &agent_headers,
            &metric_report(),
        )
        .await;
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    }

    let alerts_response = send_empty(
        &context.app,
        Method::GET,
        "/api/v1/alerts",
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    let alerts: Vec<AlertRecord> = response_json(alerts_response).await;
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].active);
    assert!(matches!(alerts[0].kind, AlertKind::Cpu));

    let mut recovered = metric_report();
    recovered.cpu_percent = 1.0;
    let accepted = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/metrics",
        &agent_headers,
        &recovered,
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    let alerts_response = send_empty(
        &context.app,
        Method::GET,
        "/api/v1/alerts",
        &[(header::COOKIE.as_str(), cookie.as_str())],
    )
    .await;
    let alerts: Vec<AlertRecord> = response_json(alerts_response).await;
    assert_eq!(alerts.len(), 1);
    assert!(!alerts[0].active);
    assert!(alerts[0].resolved_at.is_some());
}

fn enroll_request(agent_id: Uuid, agent_secret: &str) -> EnrollRequest {
    EnrollRequest {
        agent_id,
        agent_secret: agent_secret.to_owned(),
        hostname: "test-host".to_owned(),
        display_name: "Test node".to_owned(),
        os: "linux".to_owned(),
        os_version: "1".to_owned(),
        kernel_version: "6.0".to_owned(),
        architecture: "x86_64".to_owned(),
        agent_version: "0.1.0".to_owned(),
    }
}

fn metric_report() -> MetricReport {
    MetricReport {
        collected_at: Utc::now(),
        cpu_percent: 42.5,
        memory_used_bytes: 4 * 1024 * 1024,
        memory_total_bytes: 8 * 1024 * 1024,
        swap_used_bytes: 0,
        swap_total_bytes: 0,
        disk_used_bytes: 20 * 1024 * 1024,
        disk_total_bytes: 100 * 1024 * 1024,
        network_received_bytes_per_sec: 1000,
        network_transmitted_bytes_per_sec: 500,
        hub_latency_ms: Some(42.0),
        load_one: Some(0.5),
        load_five: Some(0.4),
        load_fifteen: Some(0.3),
        temperature_celsius: Some(55.0),
        uptime_seconds: 3600,
        process_count: 100,
        processes: Vec::new(),
        disks: Vec::new(),
        interfaces: Vec::new(),
    }
}

async fn send_empty(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
) -> Response<Body> {
    send(app, method, uri, headers, Body::empty()).await
}

async fn send_json(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Value,
) -> Response<Body> {
    send(
        app,
        method,
        uri,
        headers,
        Body::from(serde_json::to_vec(&body).expect("serialize JSON")),
    )
    .await
}

async fn send_serialized<T: serde::Serialize>(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: &T,
) -> Response<Body> {
    send(
        app,
        method,
        uri,
        headers,
        Body::from(serde_json::to_vec(body).expect("serialize body")),
    )
    .await
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    app.clone()
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("router response")
}

async fn response_json<T: DeserializeOwned>(response: Response<Body>) -> T {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("deserialize response")
}
