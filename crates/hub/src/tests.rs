use axum::{
    Router,
    body::Body,
    http::{Method, Request, Response, StatusCode, header},
};
use chrono::Utc;
use http_body_util::BodyExt;
use pinglake_protocol::{
    AlertKind, AlertRecord, AlertSettings, CpuCore, EnrollRequest, HistoryPoint, HostGroup,
    MetricReport, MetricStatus, MonitoringData, MonitoringHistoryPoint, NodeMonitoringConfig,
    NodeSnapshot, ProbeKind, ProbeResult, ProbeStatistics, ProbeStatus, ProbeTarget, ServiceCheck,
    ServiceResult,
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
    state: super::AppState,
}

impl TestContext {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("tempdir");
        let database_path = directory.path().join("pinglake.db");
        let (app, state) = build_app(Config::for_test(database_path.clone())).expect("build app");
        Self {
            _directory: directory,
            database_path,
            app,
            state,
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
    assert!(history[0].collected_at < server_time_before_report - chrono::Duration::days(29));
    assert_eq!(
        history[0].collected_at.timestamp_millis(),
        report.collected_at.timestamp_millis()
    );
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

#[tokio::test]
async fn monitoring_config_is_scoped_versioned_and_allows_only_precise_https_origins() {
    let context = TestContext::new();
    let id = Uuid::new_v4();
    enroll_monitoring_node(&context, id).await;
    let cookie = context.login().await;
    let path = format!("/api/v1/nodes/{id}/monitoring");
    assert_eq!(
        send_empty(&context.app, Method::GET, &path, &[])
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let config = NodeMonitoringConfig {
        browser_latency_url: Some("https://node.example.com:8443/ping".into()),
        ..Default::default()
    };
    let saved = send_serialized(
        &context.app,
        Method::PUT,
        &path,
        &[("cookie", &cookie)],
        &config,
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK);
    let csp = saved.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("connect-src 'self' https://node.example.com:8443;"));
    assert!(!csp.contains("connect-src *"));
    let first: NodeMonitoringConfig = response_json(saved).await;
    assert_eq!(first.revision, 1);
    let second: NodeMonitoringConfig = response_json(
        send_serialized(
            &context.app,
            Method::PUT,
            &path,
            &[("cookie", &cookie)],
            &first,
        )
        .await,
    )
    .await;
    assert_eq!(second.revision, 2);
    let mut stale = first.clone();
    stale.browser_latency_url = Some("https://stale.example.com/ping".into());
    assert_eq!(
        send_serialized(
            &context.app,
            Method::PUT,
            &path,
            &[("cookie", &cookie)],
            &stale
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let conflict = context
        .state
        .inner
        .database
        .save_monitoring_config(id, stale)
        .unwrap_err();
    assert!(conflict.is::<super::monitoring::MonitoringRevisionConflict>());
    let latest: NodeMonitoringConfig =
        response_json(send_empty(&context.app, Method::GET, &path, &[("cookie", &cookie)]).await)
            .await;
    assert_eq!(latest.revision, second.revision);
    assert_eq!(latest.browser_latency_url, second.browser_latency_url);
    let connection = rusqlite::Connection::open(&context.database_path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM monitoring_configs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let id_text = id.to_string();
    let own: NodeMonitoringConfig = response_json(
        send_empty(
            &context.app,
            Method::GET,
            "/api/v1/agent/config",
            &[
                ("x-agent-id", &id_text),
                ("authorization", "Bearer correct-secret"),
            ],
        )
        .await,
    )
    .await;
    assert_eq!(own.revision, 2);
    assert_eq!(
        send_empty(
            &context.app,
            Method::GET,
            "/api/v1/agent/config",
            &[
                ("x-agent-id", &id_text),
                ("authorization", "Bearer wrong-secret")
            ]
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let missing = format!("/api/v1/nodes/{}/monitoring", Uuid::new_v4());
    assert_eq!(
        send_empty(&context.app, Method::GET, &missing, &[("cookie", &cookie)])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let mut invalid = first.clone();
    invalid.browser_latency_url = Some("https://user:secret@example.com/ping".into());
    assert_eq!(
        send_serialized(
            &context.app,
            Method::PUT,
            &path,
            &[("cookie", &cookie)],
            &invalid
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let nodes: Vec<NodeSnapshot> = response_json(
        send_empty(
            &context.app,
            Method::GET,
            "/api/v1/nodes",
            &[("cookie", &cookie)],
        )
        .await,
    )
    .await;
    assert_eq!(nodes[0].browser_latency_url, first.browser_latency_url);
}

#[tokio::test]
async fn concurrent_monitoring_config_writes_commit_only_one_revision() {
    let context = TestContext::new();
    let id = Uuid::new_v4();
    enroll_monitoring_node(&context, id).await;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let first_state = context.state.clone();
    let first_barrier = barrier.clone();
    let second_state = context.state.clone();
    let first = tokio::task::spawn_blocking(move || {
        first_barrier.wait();
        first_state.inner.database.save_monitoring_config(
            id,
            NodeMonitoringConfig {
                browser_latency_url: Some("https://first.example.com/ping".into()),
                ..Default::default()
            },
        )
    });
    let second = tokio::task::spawn_blocking(move || {
        barrier.wait();
        second_state.inner.database.save_monitoring_config(
            id,
            NodeMonitoringConfig {
                browser_latency_url: Some("https://second.example.com/ping".into()),
                ..Default::default()
            },
        )
    });
    let outcomes = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome
                .as_ref()
                .is_err_and(|error| error.is::<super::monitoring::MonitoringRevisionConflict>()))
            .count(),
        1
    );
    let latest = context
        .state
        .inner
        .database
        .monitoring_config(id)
        .unwrap()
        .unwrap();
    assert_eq!(latest.revision, 1);
    assert!(
        latest.browser_latency_url == Some("https://first.example.com/ping".into())
            || latest.browser_latency_url == Some("https://second.example.com/ping".into())
    );
}

#[tokio::test]
async fn extended_samples_and_probe_ids_are_deduplicated_with_scoped_history() {
    let context = TestContext::new();
    let id = Uuid::new_v4();
    enroll_monitoring_node(&context, id).await;
    let cookie = context.login().await;
    let target_id = Uuid::new_v4();
    let config = NodeMonitoringConfig {
        probes: vec![ProbeTarget {
            id: target_id,
            name: "HTTPS".into(),
            kind: ProbeKind::Http,
            target: "https://example.com/".into(),
            port: None,
            enabled: true,
            interval_secs: 30,
            timeout_ms: 5000,
            expected_status: Some(200),
            response_contains: None,
        }],
        ..Default::default()
    };
    let saved: NodeMonitoringConfig = response_json(
        send_serialized(
            &context.app,
            Method::PUT,
            &format!("/api/v1/nodes/{id}/monitoring"),
            &[("cookie", &cookie)],
            &config,
        )
        .await,
    )
    .await;
    let mut report = metric_report();
    report.collected_at = Utc::now() - chrono::Duration::seconds(30);
    let at = Utc::now();
    report.monitoring = Some(MonitoringData {
        schema_version: 1,
        session_id: Uuid::new_v4(),
        sample_sequence: 1,
        report_interval_secs: 5,
        cpu_cores: vec![CpuCore {
            id: "0".into(),
            usage_percent: 42.0,
            frequency_mhz: Some(3000),
        }],
        probes: vec![ProbeResult {
            sample_id: Uuid::new_v4(),
            target_id,
            config_revision: saved.revision,
            kind: ProbeKind::Http,
            scheduled_at: at,
            completed_at: at,
            status: ProbeStatus::Success,
            latency_ms: Some(42.0),
            http_status: Some(200),
            error: None,
        }],
        ..Default::default()
    });
    let id_text = id.to_string();
    let headers = [
        ("x-agent-id", id_text.as_str()),
        ("authorization", "Bearer correct-secret"),
    ];
    for _ in 0..2 {
        assert_eq!(
            send_serialized(
                &context.app,
                Method::POST,
                "/api/v1/agent/metrics",
                &headers,
                &report
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
    }
    report.monitoring.as_mut().unwrap().sample_sequence = 2;
    assert_eq!(
        send_serialized(
            &context.app,
            Method::POST,
            "/api/v1/agent/metrics",
            &headers,
            &report
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    let connection = rusqlite::Connection::open(&context.database_path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM metrics", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM probe_samples", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let statistics: Vec<ProbeStatistics> = response_json(
        send_empty(
            &context.app,
            Method::GET,
            &format!("/api/v1/nodes/{id}/probes/statistics?minutes=60"),
            &[("cookie", &cookie)],
        )
        .await,
    )
    .await;
    assert_eq!(statistics.len(), 1);
    assert_eq!(statistics[0].successful, 1);
    assert_eq!(statistics[0].p95_ms, Some(42.0));
    let points: Vec<MonitoringHistoryPoint> = response_json(
        send_empty(
            &context.app,
            Method::GET,
            &format!("/api/v1/nodes/{id}/monitoring/history?section=cpu&device=0"),
            &[("cookie", &cookie)],
        )
        .await,
    )
    .await;
    assert!(!points.is_empty());
    assert!(points.len() <= 240);
    assert_eq!(points[0].monitoring.cpu_cores[0].usage_percent, 42.0);
    assert!(points[0].monitoring.probes.is_empty());
    assert!(points[0].received_at > points[0].collected_at + chrono::Duration::seconds(29));
    let probe_points: Vec<MonitoringHistoryPoint> = response_json(
        send_empty(
            &context.app,
            Method::GET,
            &format!("/api/v1/nodes/{id}/monitoring/history?section=probes&device={target_id}"),
            &[("cookie", &cookie)],
        )
        .await,
    )
    .await;
    assert_eq!(probe_points.len(), 1);
    report.monitoring.as_mut().unwrap().probes[0].target_id = Uuid::new_v4();
    report.monitoring.as_mut().unwrap().sample_sequence = 3;
    assert_eq!(
        send_serialized(
            &context.app,
            Method::POST,
            "/api/v1/agent/metrics",
            &headers,
            &report
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send_empty(
            &context.app,
            Method::GET,
            &format!("/api/v1/nodes/{id}/monitoring/history?section=invalid"),
            &[("cookie", &cookie)]
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn deleting_groups_preserves_nodes_and_metric_history() {
    let context = TestContext::new();
    let id = Uuid::new_v4();
    enroll_monitoring_node(&context, id).await;
    let cookie = context.login().await;
    let mut events = context.state.inner.events.subscribe();
    let group: HostGroup = response_json(
        send_json(
            &context.app,
            Method::POST,
            "/api/v1/groups",
            &[("cookie", &cookie)],
            json!({"name":"Group"}),
        )
        .await,
    )
    .await;
    assert_eq!(
        send_json(
            &context.app,
            Method::PUT,
            &format!("/api/v1/nodes/{id}/group"),
            &[("cookie", &cookie)],
            json!({"group_id":group.id})
        )
        .await
        .status(),
        StatusCode::OK
    );
    let id_text = id.to_string();
    assert_eq!(
        send_serialized(
            &context.app,
            Method::POST,
            "/api/v1/agent/metrics",
            &[
                ("x-agent-id", &id_text),
                ("authorization", "Bearer correct-secret")
            ],
            &metric_report()
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    let path = format!("/api/v1/groups/{}", group.id);
    assert_eq!(
        send_empty(&context.app, Method::DELETE, &path, &[])
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send_empty(&context.app, Method::DELETE, &path, &[("cookie", &cookie)])
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send_empty(&context.app, Method::DELETE, &path, &[("cookie", &cookie)])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let nodes: Vec<NodeSnapshot> = response_json(
        send_empty(
            &context.app,
            Method::GET,
            "/api/v1/nodes",
            &[("cookie", &cookie)],
        )
        .await,
    )
    .await;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].group_id, None);
    assert!(nodes[0].latest.is_some());
    let history: Vec<HistoryPoint> = response_json(
        send_empty(
            &context.app,
            Method::GET,
            &format!("/api/v1/nodes/{id}/history"),
            &[("cookie", &cookie)],
        )
        .await,
    )
    .await;
    assert_eq!(history.len(), 1);
    let mut group_removed = false;
    let mut node_ungrouped = false;
    while let Ok(event) = events.try_recv() {
        match event {
            pinglake_protocol::LiveEvent::GroupsChanged(groups) if groups.is_empty() => {
                group_removed = true
            }
            pinglake_protocol::LiveEvent::Snapshot(node)
                if node.id == id && node.group_id.is_none() =>
            {
                node_ungrouped = true
            }
            _ => {}
        }
    }
    assert!(group_removed && node_ungrouped);
}

#[tokio::test]
async fn monitoring_retention_preserves_the_configuration_baseline() {
    let context = TestContext::new();
    let id = Uuid::new_v4();
    enroll_monitoring_node(&context, id).await;
    let cookie = context.login().await;
    let path = format!("/api/v1/nodes/{id}/monitoring");
    for _ in 0..3 {
        let current: NodeMonitoringConfig = response_json(
            send_empty(&context.app, Method::GET, &path, &[("cookie", &cookie)]).await,
        )
        .await;
        assert_eq!(
            send_serialized(
                &context.app,
                Method::PUT,
                &path,
                &[("cookie", &cookie)],
                &current
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
    let id_text = id.to_string();
    assert_eq!(
        send_serialized(
            &context.app,
            Method::POST,
            "/api/v1/agent/metrics",
            &[
                ("x-agent-id", &id_text),
                ("authorization", "Bearer correct-secret")
            ],
            &metric_report()
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    let connection = rusqlite::Connection::open(&context.database_path).unwrap();
    let old = (Utc::now() - chrono::Duration::days(8)).to_rfc3339();
    connection
        .execute("UPDATE metrics SET received_at=?1", [&old])
        .unwrap();
    connection
        .execute(
            "UPDATE monitoring_configs SET effective_at=?1 WHERE revision IN (1,2)",
            [&old],
        )
        .unwrap();
    connection.execute("INSERT INTO probe_samples(node_id,sample_id,target_id,config_revision,scheduled_at,received_at,result_json) VALUES(?1,'old','target',1,?2,?2,'{}')", rusqlite::params![id_text, old]).unwrap();
    connection.execute("INSERT INTO service_samples(node_id,subject_id,config_revision,checked_at,received_at,result_json) VALUES(?1,'old',1,?2,?2,'{}')", rusqlite::params![id_text, old]).unwrap();
    assert_eq!(
        context.state.inner.database.cleanup_old_metrics().unwrap(),
        1
    );
    for table in ["metrics", "probe_samples", "service_samples"] {
        assert_eq!(
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let mut statement = connection
        .prepare("SELECT revision FROM monitoring_configs ORDER BY revision")
        .unwrap();
    let revisions = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(revisions, vec![2, 3]);
}

#[tokio::test]
async fn service_alert_subjects_recover_independently_and_unknown_is_not_recovery() {
    let context = TestContext::new();
    let id = Uuid::new_v4();
    enroll_monitoring_node(&context, id).await;
    let cookie = context.login().await;
    let settings = AlertSettings {
        sustained_for_seconds: 0,
        ..Default::default()
    };
    assert_eq!(
        send_serialized(
            &context.app,
            Method::PUT,
            "/api/v1/settings",
            &[("cookie", &cookie)],
            &settings
        )
        .await
        .status(),
        StatusCode::OK
    );
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let config = NodeMonitoringConfig {
        services: vec![
            ServiceCheck {
                id: first,
                name: "first.service".into(),
                enabled: true,
                expected_state: "running".into(),
            },
            ServiceCheck {
                id: second,
                name: "second.service".into(),
                enabled: true,
                expected_state: "running".into(),
            },
        ],
        ..Default::default()
    };
    let saved: NodeMonitoringConfig = response_json(
        send_serialized(
            &context.app,
            Method::PUT,
            &format!("/api/v1/nodes/{id}/monitoring"),
            &[("cookie", &cookie)],
            &config,
        )
        .await,
    )
    .await;
    let mut report = metric_report();
    let at = Utc::now() - chrono::Duration::seconds(10);
    report.monitoring = Some(MonitoringData {
        session_id: Uuid::new_v4(),
        sample_sequence: 1,
        report_interval_secs: 5,
        services: saved
            .services
            .iter()
            .map(|check| ServiceResult {
                id: check.id,
                name: check.name.clone(),
                checked_at: at,
                status: MetricStatus::Ok,
                state: "stopped".into(),
                healthy: Some(false),
                error: None,
                config_revision: saved.revision,
            })
            .collect(),
        ..Default::default()
    });
    let id_text = id.to_string();
    let headers = [
        ("x-agent-id", id_text.as_str()),
        ("authorization", "Bearer correct-secret"),
    ];
    assert_eq!(
        send_serialized(
            &context.app,
            Method::POST,
            "/api/v1/agent/metrics",
            &headers,
            &report
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    let alerts: Vec<AlertRecord> = response_json(
        send_empty(
            &context.app,
            Method::GET,
            "/api/v1/alerts",
            &[("cookie", &cookie)],
        )
        .await,
    )
    .await;
    assert_eq!(alerts.len(), 2);
    assert!(
        alerts
            .iter()
            .all(|alert| alert.active && matches!(alert.kind, AlertKind::Service))
    );
    let data = report.monitoring.as_mut().unwrap();
    data.sample_sequence = 2;
    data.services[0].checked_at = at + chrono::Duration::seconds(1);
    data.services[0].status = MetricStatus::PermissionDenied;
    data.services[0].healthy = None;
    data.services[1].checked_at = at + chrono::Duration::seconds(1);
    data.services[1].state = "running".into();
    data.services[1].healthy = Some(true);
    assert_eq!(
        send_serialized(
            &context.app,
            Method::POST,
            "/api/v1/agent/metrics",
            &headers,
            &report
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    let alerts: Vec<AlertRecord> = response_json(
        send_empty(
            &context.app,
            Method::GET,
            "/api/v1/alerts",
            &[("cookie", &cookie)],
        )
        .await,
    )
    .await;
    assert!(
        alerts
            .iter()
            .find(|alert| alert.subject_id.as_deref() == Some(&first.to_string()))
            .unwrap()
            .active
    );
    assert!(
        !alerts
            .iter()
            .find(|alert| alert.subject_id.as_deref() == Some(&second.to_string()))
            .unwrap()
            .active
    );
}

async fn enroll_monitoring_node(context: &TestContext, id: Uuid) {
    let response = send_serialized(
        &context.app,
        Method::POST,
        "/api/v1/agent/enroll",
        &[("x-enrollment-token", "test-enrollment-token")],
        &enroll_request(id, "correct-secret"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
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
        monitoring: None,
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
