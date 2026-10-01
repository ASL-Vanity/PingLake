mod config;
mod db;
mod error;
mod monitoring;
mod static_files;

use std::{
    collections::HashMap,
    convert::Infallible,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post, put},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use db::{Database, EnrollResult};
use error::AppError;
use futures_util::stream;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox,
    transport::smtp::authentication::Credentials,
};
use pinglake_protocol::{
    AlertRecord, AlertSettings, DEFAULT_REPORT_INTERVAL_SECS, DashboardSummary, EnrollRequest,
    EnrollResponse, HistoryPoint, LiveEvent, MetricReport, MonitoringHistoryPoint,
    NodeMonitoringConfig, NodeSnapshot, ProbeStatistics,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::{RwLock, Semaphore, broadcast};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

pub use config::Config;

const SESSION_COOKIE: &str = "pinglake_session";
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_HISTORY_MINUTES: u64 = 7 * 24 * 60;
const AGENT_BODY_LIMIT_BYTES: usize = 256 * 1024;
const LOGIN_BODY_LIMIT_BYTES: usize = 4 * 1024;
const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_CONCURRENT_LOGINS: usize = 4;
const FAILED_LOGIN_DELAY: Duration = Duration::from_millis(400);

#[derive(Clone)]
pub struct AppState {
    inner: Arc<InnerState>,
}

struct InnerState {
    database: Database,
    enrollment_token_hash: [u8; 32],
    admin_password_hash: String,
    cookie_secure: bool,
    smtp: Option<config::SmtpConfig>,
    sessions: RwLock<HashMap<String, Instant>>,
    events: broadcast::Sender<LiveEvent>,
    login_slots: Arc<Semaphore>,
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

#[derive(Serialize)]
struct AuthStatus {
    authenticated: bool,
}

#[derive(Serialize)]
struct HealthStatus {
    status: &'static str,
}

#[derive(Deserialize)]
struct HistoryQuery {
    #[serde(default = "default_history_minutes")]
    minutes: u64,
}

#[derive(Deserialize)]
struct MonitoringHistoryQuery {
    #[serde(default = "default_history_minutes")]
    minutes: u64,
    #[serde(default = "default_monitoring_section")]
    section: String,
    device: Option<String>,
}

fn default_monitoring_section() -> String {
    "all".to_owned()
}

#[derive(Deserialize)]
struct GroupRequest {
    name: String,
}

#[derive(Deserialize)]
struct NodeGroupRequest {
    group_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct NodeNameRequest {
    display_name: String,
}

pub fn build_app(config: Config) -> anyhow::Result<(Router, AppState)> {
    config.validate()?;
    let database = Database::open(&config.database_path)?;
    let (event_sender, _) = broadcast::channel(256);
    let admin_password_hash = hash_admin_password(&config.admin_password)?;
    let state = AppState {
        inner: Arc::new(InnerState {
            database,
            enrollment_token_hash: hash_bytes(config.enrollment_token.as_bytes()),
            admin_password_hash,
            cookie_secure: config.cookie_secure,
            smtp: config.smtp,
            sessions: RwLock::new(HashMap::new()),
            events: event_sender,
            login_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_LOGINS)),
        }),
    };

    let agent = Router::new()
        .route("/api/v1/agent/enroll", post(enroll))
        .route("/api/v1/agent/metrics", post(metrics))
        .route("/api/v1/agent/config", get(agent_monitoring_config))
        .layer(DefaultBodyLimit::max(AGENT_BODY_LIMIT_BYTES));

    let protected = Router::new()
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(me))
        .route("/api/v1/summary", get(summary))
        .route("/api/v1/nodes", get(nodes))
        .route("/api/v1/nodes/{id}", delete(delete_node))
        .route("/api/v1/nodes/{id}/group", put(assign_node_group))
        .route("/api/v1/nodes/{id}/name", put(rename_node))
        .route("/api/v1/nodes/{id}/history", get(history))
        .route(
            "/api/v1/nodes/{id}/monitoring",
            get(get_monitoring_config)
                .put(put_monitoring_config)
                .layer(DefaultBodyLimit::max(32 * 1024)),
        )
        .route(
            "/api/v1/nodes/{id}/monitoring/history",
            get(monitoring_history),
        )
        .route(
            "/api/v1/nodes/{id}/probes/statistics",
            get(probe_statistics),
        )
        .route(
            "/api/v1/nodes/{id}/checks/statistics",
            get(check_statistics),
        )
        .route("/api/v1/groups", get(groups).post(create_group))
        .route("/api/v1/groups/{id}", delete(delete_group))
        .route("/api/v1/alerts", get(alerts))
        .route("/api/v1/settings", get(get_settings).put(put_settings))
        .route("/api/v1/events", get(events))
        .layer(DefaultBodyLimit::max(LOGIN_BODY_LIMIT_BYTES))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_admin));

    let auth = Router::new()
        .route("/api/v1/auth/login", post(login))
        .layer(DefaultBodyLimit::max(LOGIN_BODY_LIMIT_BYTES));

    let app = Router::new()
        .route("/api/healthz", get(healthz))
        .merge(auth)
        .merge(agent)
        .merge(protected)
        .fallback(static_files::serve)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    Ok((app, state))
}

impl AppState {
    pub fn start_background_tasks(&self) {
        let offline_state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                match offline_state.inner.database.check_offline_nodes() {
                    Ok(alerts) => offline_state.publish_alerts(alerts),
                    Err(error) => tracing::error!(error = %error, "offline check failed"),
                }
            }
        });

        let cleanup_state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                match cleanup_state.inner.database.cleanup_old_metrics() {
                    Ok(deleted) if deleted != 0 => {
                        tracing::info!(deleted, "expired metrics removed")
                    }
                    Ok(_) => {}
                    Err(error) => tracing::error!(error = %error, "metric cleanup failed"),
                }
            }
        });
    }

    fn publish(&self, event: LiveEvent) {
        let _ = self.inner.events.send(event);
    }

    fn publish_alerts(&self, alerts: Vec<AlertRecord>) {
        for alert in alerts {
            self.publish(LiveEvent::Alert(alert.clone()));
            self.dispatch_notifications(alert);
        }
    }

    fn dispatch_notifications(&self, alert: AlertRecord) {
        let settings = match self.inner.database.settings() {
            Ok(settings) => settings,
            Err(error) => {
                tracing::error!(error = %error, "could not load webhook settings");
                return;
            }
        };
        if !notification_enabled_for_kind(&settings, &alert.kind) {
            return;
        }
        if settings.webhook_enabled && !settings.webhook_url.trim().is_empty() {
            let url = settings.webhook_url.clone();
            let webhook_alert = alert.clone();
            tokio::spawn(async move {
                if let Err(error) = send_webhook(url, webhook_alert).await {
                    tracing::warn!(error = %error, "webhook delivery failed");
                }
            });
        }
        if settings.email_enabled && !settings.email_recipients.is_empty() {
            let Some(smtp) = self.inner.smtp.clone() else {
                tracing::warn!("email notifications are enabled but SMTP is not configured");
                return;
            };
            let recipients = settings.email_recipients;
            tokio::spawn(async move {
                if let Err(error) = send_email(smtp, recipients, alert).await {
                    tracing::warn!(error = %error, "email delivery failed");
                }
            });
        }
    }
}

async fn healthz(State(state): State<AppState>) -> Result<Json<HealthStatus>, AppError> {
    state.inner.database.readiness_check()?;
    Ok(Json(HealthStatus { status: "ok" }))
}

async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, AppError> {
    let _permit = state
        .inner
        .login_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::too_many_requests("too many concurrent login attempts"))?;
    let password_valid = request.password.len() <= MAX_PASSWORD_BYTES
        && verify_admin_password(&request.password, &state.inner.admin_password_hash);
    if !password_valid {
        tokio::time::sleep(FAILED_LOGIN_DELAY).await;
        return Err(AppError::unauthorized());
    }

    let session = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let expires_at = Instant::now() + SESSION_TTL;
    let mut sessions = state.inner.sessions.write().await;
    sessions.retain(|_, expiry| *expiry > Instant::now());
    sessions.insert(session.clone(), expires_at);
    drop(sessions);

    let cookie = session_cookie(&session, state.inner.cookie_secure, false);
    let mut response = Json(AuthStatus {
        authenticated: true,
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|error| AppError::Internal(error.into()))?,
    );
    Ok(response)
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    if let Some(session) = session_token(&headers) {
        state.inner.sessions.write().await.remove(&session);
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&session_cookie("", state.inner.cookie_secure, true))
            .map_err(|error| AppError::Internal(error.into()))?,
    );
    Ok(response)
}

async fn me() -> Json<AuthStatus> {
    Json(AuthStatus {
        authenticated: true,
    })
}

async fn enroll(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<EnrollRequest>,
) -> Result<Json<EnrollResponse>, AppError> {
    validate_enroll_request(&request)?;
    let enrollment_token_valid = headers
        .get("x-enrollment-token")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            constant_time_equal(
                &hash_bytes(value.as_bytes()),
                &state.inner.enrollment_token_hash,
            )
        });
    let secret_hash = hash_hex(request.agent_secret.as_bytes());
    match state
        .inner
        .database
        .enroll(&request, &secret_hash, enrollment_token_valid)?
    {
        EnrollResult::Created | EnrollResult::Updated => Ok(Json(EnrollResponse {
            accepted: true,
            report_interval_secs: DEFAULT_REPORT_INTERVAL_SECS,
            monitoring_schema_max: 2,
        })),
        EnrollResult::SecretMismatch => Err(AppError::conflict("agent ID is already registered")),
        EnrollResult::EnrollmentTokenRequired => Err(AppError::unauthorized()),
    }
}

async fn metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(report): Json<MetricReport>,
) -> Result<StatusCode, AppError> {
    validate_metric_report(&report)?;
    let node_id = headers
        .get("x-agent-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(AppError::unauthorized)?;
    let secret = bearer_secret(&headers).ok_or_else(AppError::unauthorized)?;
    let secret_hash = hash_hex(secret.as_bytes());
    if state
        .inner
        .database
        .agent_monitoring_config(node_id, &secret_hash)?
        .is_none()
    {
        return Err(AppError::unauthorized());
    }
    if let Some(data) = &report.monitoring
        && !state
            .inner
            .database
            .validate_monitoring_identity(node_id, data)?
    {
        return Err(AppError::bad_request(
            "service or probe result does not match this node's monitoring configuration",
        ));
    }
    let result = state
        .inner
        .database
        .record_metric(node_id, &secret_hash, &report)?
        .ok_or_else(AppError::unauthorized)?;

    state.publish(LiveEvent::Snapshot(Box::new(result.snapshot)));
    state.publish_alerts(result.alerts);
    Ok(StatusCode::ACCEPTED)
}

async fn summary(State(state): State<AppState>) -> Result<Json<DashboardSummary>, AppError> {
    let nodes = state.inner.database.nodes()?;
    let online_nodes = nodes.iter().filter(|node| node.online).count();
    let cpu_values = nodes
        .iter()
        .filter(|node| node.online)
        .filter_map(|node| {
            node.latest
                .as_ref()
                .map(|metric| f64::from(metric.cpu_percent))
        })
        .collect::<Vec<_>>();
    let memory_values = nodes
        .iter()
        .filter(|node| node.online)
        .filter_map(|node| {
            let latest = node.latest.as_ref()?;
            (latest.memory_total_bytes != 0)
                .then(|| latest.memory_used_bytes as f64 * 100.0 / latest.memory_total_bytes as f64)
        })
        .collect::<Vec<_>>();
    let total_nodes = nodes.len();
    Ok(Json(DashboardSummary {
        total_nodes,
        online_nodes,
        offline_nodes: total_nodes.saturating_sub(online_nodes),
        active_alerts: state.inner.database.active_alert_count()?,
        average_cpu_percent: average(&cpu_values),
        average_memory_percent: average(&memory_values),
    }))
}

async fn nodes(State(state): State<AppState>) -> Result<Json<Vec<NodeSnapshot>>, AppError> {
    Ok(Json(state.inner.database.nodes()?))
}

async fn groups(
    State(state): State<AppState>,
) -> Result<Json<Vec<pinglake_protocol::HostGroup>>, AppError> {
    Ok(Json(state.inner.database.groups()?))
}

async fn create_group(
    State(state): State<AppState>,
    Json(request): Json<GroupRequest>,
) -> Result<(StatusCode, Json<pinglake_protocol::HostGroup>), AppError> {
    let name = request.name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::bad_request("group name must be 1-64 bytes"));
    }
    if state
        .inner
        .database
        .groups()?
        .iter()
        .any(|group| group.name.eq_ignore_ascii_case(name))
    {
        return Err(AppError::conflict("group name already exists"));
    }
    let group = state.inner.database.create_group(name)?;
    state.publish(LiveEvent::GroupsChanged(state.inner.database.groups()?));
    Ok((StatusCode::CREATED, Json(group)))
}

async fn delete_group(
    State(state): State<AppState>,
    Path(group_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if !state.inner.database.delete_group(group_id)? {
        return Err(AppError::not_found("group not found"));
    }
    state.publish(LiveEvent::GroupsChanged(state.inner.database.groups()?));
    for snapshot in state.inner.database.nodes()? {
        state.publish(LiveEvent::Snapshot(Box::new(snapshot)));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn assign_node_group(
    State(state): State<AppState>,
    Path(node_id): Path<Uuid>,
    Json(request): Json<NodeGroupRequest>,
) -> Result<Json<NodeSnapshot>, AppError> {
    let snapshot = state
        .inner
        .database
        .assign_group(node_id, request.group_id)?
        .ok_or_else(|| AppError::not_found("node or group not found"))?;
    state.publish(LiveEvent::Snapshot(Box::new(snapshot.clone())));
    Ok(Json(snapshot))
}

async fn rename_node(
    State(state): State<AppState>,
    Path(node_id): Path<Uuid>,
    Json(request): Json<NodeNameRequest>,
) -> Result<Json<NodeSnapshot>, AppError> {
    let name = request.display_name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::bad_request("display_name must be 1-64 bytes"));
    }
    let snapshot = state
        .inner
        .database
        .rename_node(node_id, name)?
        .ok_or_else(|| AppError::not_found("node not found"))?;
    state.publish(LiveEvent::Snapshot(Box::new(snapshot.clone())));
    Ok(Json(snapshot))
}

async fn history(
    State(state): State<AppState>,
    Path(node_id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<HistoryPoint>>, AppError> {
    if query.minutes == 0 || query.minutes > MAX_HISTORY_MINUTES {
        return Err(AppError::bad_request(format!(
            "minutes must be between 1 and {MAX_HISTORY_MINUTES}"
        )));
    }
    state
        .inner
        .database
        .history(node_id, query.minutes)?
        .map(Json)
        .ok_or_else(|| AppError::not_found("node not found"))
}

async fn get_monitoring_config(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeMonitoringConfig>, AppError> {
    state
        .inner
        .database
        .monitoring_config(id)?
        .map(Json)
        .ok_or_else(|| AppError::not_found("node not found"))
}

async fn put_monitoring_config(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(config): Json<NodeMonitoringConfig>,
) -> Result<Json<NodeMonitoringConfig>, AppError> {
    monitoring::validate_config(&config)?;
    let config = state
        .inner
        .database
        .save_monitoring_config(id, config)
        .map_err(|error| {
            if error.is::<monitoring::MonitoringRevisionConflict>() {
                AppError::conflict(error.to_string())
            } else {
                AppError::Internal(error)
            }
        })?
        .ok_or_else(|| AppError::not_found("node not found"))?;
    if let Some(snapshot) = state
        .inner
        .database
        .nodes()?
        .into_iter()
        .find(|node| node.id == id)
    {
        state.publish(LiveEvent::Snapshot(Box::new(snapshot)));
    }
    Ok(Json(config))
}

async fn agent_monitoring_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<NodeMonitoringConfig>, AppError> {
    let id = headers
        .get("x-agent-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(AppError::unauthorized)?;
    let secret = bearer_secret(&headers).ok_or_else(AppError::unauthorized)?;
    let schema_max = headers
        .get("x-monitoring-schema-max")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1);
    let config = state
        .inner
        .database
        .agent_monitoring_config(id, &hash_hex(secret.as_bytes()))?
        .ok_or_else(AppError::unauthorized)?;
    if schema_max < 2 {
        let mut legacy = config;
        legacy.dns_checks.clear();
        legacy.process_checks.clear();
        legacy.local_port_checks.clear();
        legacy.probes.retain(|probe| {
            matches!(
                probe.kind,
                pinglake_protocol::ProbeKind::Icmp
                    | pinglake_protocol::ProbeKind::Tcp
                    | pinglake_protocol::ProbeKind::Http
            )
        });
        return Ok(Json(legacy));
    }
    Ok(Json(config))
}

fn validate_history_minutes(minutes: u64) -> Result<(), AppError> {
    if minutes == 0 || minutes > MAX_HISTORY_MINUTES {
        Err(AppError::bad_request(format!(
            "minutes must be between 1 and {MAX_HISTORY_MINUTES}"
        )))
    } else {
        Ok(())
    }
}

async fn monitoring_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<MonitoringHistoryQuery>,
) -> Result<Json<Vec<MonitoringHistoryPoint>>, AppError> {
    validate_history_minutes(query.minutes)?;
    if ![
        "cpu",
        "memory",
        "disk",
        "network",
        "tcp",
        "agent",
        "services",
        "probes",
        "dns",
        "processes",
        "ports",
        "all",
    ]
    .contains(&query.section.as_str())
        || query
            .device
            .as_ref()
            .is_some_and(|device| device.len() > 512)
    {
        return Err(AppError::bad_request(
            "invalid monitoring history section or device",
        ));
    }
    let points = tokio::task::spawn_blocking(move || {
        state.inner.database.monitoring_history(
            id,
            query.minutes,
            &query.section,
            query.device.as_deref(),
        )
    })
    .await
    .map_err(|error| AppError::Internal(error.into()))??
    .ok_or_else(|| AppError::not_found("node not found"))?;
    if serde_json::to_vec(&points)
        .map_err(|error| AppError::Internal(error.into()))?
        .len()
        > monitoring::MAX_HISTORY_BYTES
    {
        return Err(AppError::bad_request(
            "history exceeds 4 MiB; select a section or device, or a shorter window",
        ));
    }
    Ok(Json(points))
}

async fn probe_statistics(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<ProbeStatistics>>, AppError> {
    validate_history_minutes(query.minutes)?;
    tokio::task::spawn_blocking(move || state.inner.database.probe_statistics(id, query.minutes))
        .await
        .map_err(|error| AppError::Internal(error.into()))??
        .map(Json)
        .ok_or_else(|| AppError::not_found("node not found"))
}

#[derive(Debug, Deserialize)]
struct CheckStatisticsQuery {
    #[serde(default = "default_history_minutes")]
    minutes: u64,
    kind: Option<String>,
}

async fn check_statistics(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<CheckStatisticsQuery>,
) -> Result<Json<Vec<monitoring::CheckStatistics>>, AppError> {
    validate_history_minutes(query.minutes)?;
    if query
        .kind
        .as_deref()
        .is_some_and(|kind| !matches!(kind, "dns" | "process" | "port"))
    {
        return Err(AppError::bad_request("invalid check statistics kind"));
    }
    tokio::task::spawn_blocking(move || {
        state
            .inner
            .database
            .check_statistics(id, query.minutes, query.kind.as_deref())
    })
    .await
    .map_err(|error| AppError::Internal(error.into()))??
    .map(Json)
    .ok_or_else(|| AppError::not_found("node not found"))
}

async fn delete_node(
    State(state): State<AppState>,
    Path(node_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if !state.inner.database.delete_node(node_id)? {
        return Err(AppError::not_found("node not found"));
    }
    state.publish(LiveEvent::NodeRemoved { id: node_id });
    Ok(StatusCode::NO_CONTENT)
}

async fn alerts(State(state): State<AppState>) -> Result<Json<Vec<AlertRecord>>, AppError> {
    Ok(Json(state.inner.database.alerts()?))
}

async fn get_settings(State(state): State<AppState>) -> Result<Json<AlertSettings>, AppError> {
    Ok(Json(state.inner.database.settings()?))
}

async fn put_settings(
    State(state): State<AppState>,
    Json(settings): Json<AlertSettings>,
) -> Result<Json<AlertSettings>, AppError> {
    validate_settings(&settings)?;
    state.inner.database.update_settings(&settings)?;
    state.publish(LiveEvent::SettingsChanged(settings.clone()));
    Ok(Json(settings))
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.inner.events.subscribe();
    let stream = stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event)
                        .expect("LiveEvent serialization is infallible for valid values");
                    return Some((Ok(Event::default().event("pinglake").data(data)), receiver));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

async fn require_admin(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let Some(session) = session_token(request.headers()) else {
        return Err(AppError::unauthorized());
    };
    let now = Instant::now();
    let mut sessions = state.inner.sessions.write().await;
    sessions.retain(|_, expiry| *expiry > now);
    if !sessions.contains_key(&session) {
        return Err(AppError::unauthorized());
    }
    drop(sessions);
    Ok(next.run(request).await)
}

async fn security_headers(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let origins = state
        .inner
        .database
        .browser_latency_origins()
        .unwrap_or_default();
    let connect_sources = origins.join(" ");
    let csp = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' {connect_sources}; font-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'"
    );
    let headers = response.headers_mut();
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "same-origin"),
        ("cross-origin-opener-policy", "same-origin"),
        ("content-security-policy", csp.as_str()),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
        ),
    ] {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(HeaderName::from_static(name), value);
        }
    }
    response
}

fn validate_enroll_request(request: &EnrollRequest) -> Result<(), AppError> {
    if request.agent_secret.is_empty() || request.agent_secret.len() > 4096 {
        return Err(AppError::bad_request("agent_secret must be 1-4096 bytes"));
    }
    for (name, value) in [
        ("hostname", request.hostname.as_str()),
        ("display_name", request.display_name.as_str()),
        ("os", request.os.as_str()),
        ("os_version", request.os_version.as_str()),
        ("kernel_version", request.kernel_version.as_str()),
        ("architecture", request.architecture.as_str()),
        ("agent_version", request.agent_version.as_str()),
    ] {
        if value.trim().is_empty() || value.len() > 512 {
            return Err(AppError::bad_request(format!("{name} must be 1-512 bytes")));
        }
    }
    Ok(())
}

fn validate_metric_report(report: &MetricReport) -> Result<(), AppError> {
    if !report.cpu_percent.is_finite() || !(0.0..=100.0).contains(&report.cpu_percent) {
        return Err(AppError::bad_request(
            "cpu_percent must be between 0 and 100",
        ));
    }
    for (name, value) in [
        ("load_one", report.load_one),
        ("load_five", report.load_five),
        ("load_fifteen", report.load_fifteen),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(AppError::bad_request(format!(
                "{name} must be finite and non-negative"
            )));
        }
    }
    if report
        .temperature_celsius
        .is_some_and(|value| !value.is_finite() || !(-273.15..=1000.0).contains(&value))
    {
        return Err(AppError::bad_request(
            "temperature_celsius must be finite and between -273.15 and 1000",
        ));
    }
    if report
        .hub_latency_ms
        .is_some_and(|value| !value.is_finite() || !(0.0..=60_000.0).contains(&value))
    {
        return Err(AppError::bad_request(
            "hub_latency_ms must be finite and between 0 and 60000",
        ));
    }
    if report.disks.len() > 128 || report.interfaces.len() > 128 || report.processes.len() > 25 {
        return Err(AppError::bad_request(
            "metric report contains too many disks, interfaces, or processes",
        ));
    }
    for (name, used, total) in [
        (
            "memory",
            report.memory_used_bytes,
            report.memory_total_bytes,
        ),
        ("swap", report.swap_used_bytes, report.swap_total_bytes),
        ("disk", report.disk_used_bytes, report.disk_total_bytes),
    ] {
        validate_usage(name, used, total)?;
    }
    for (name, value) in [
        (
            "network_received_bytes_per_sec",
            report.network_received_bytes_per_sec,
        ),
        (
            "network_transmitted_bytes_per_sec",
            report.network_transmitted_bytes_per_sec,
        ),
        ("uptime_seconds", report.uptime_seconds),
    ] {
        validate_sqlite_integer(name, value)?;
    }
    if i64::try_from(report.process_count).is_err() {
        return Err(AppError::bad_request(
            "process_count exceeds the supported range",
        ));
    }
    for disk in &report.disks {
        for (name, value) in [
            ("disk.name", disk.name.as_str()),
            ("disk.mount_point", disk.mount_point.as_str()),
            ("disk.file_system", disk.file_system.as_str()),
        ] {
            validate_metric_string(name, value)?;
        }
        validate_usage("disk entry", disk.used_bytes, disk.total_bytes)?;
    }
    for interface in &report.interfaces {
        validate_metric_string("interface.name", &interface.name)?;
        validate_sqlite_integer(
            "interface.received_bytes_per_sec",
            interface.received_bytes_per_sec,
        )?;
        validate_sqlite_integer(
            "interface.transmitted_bytes_per_sec",
            interface.transmitted_bytes_per_sec,
        )?;
    }
    for process in &report.processes {
        validate_metric_string("process.name", &process.name)?;
        if !process.cpu_percent.is_finite() || !(0.0..=10_000.0).contains(&process.cpu_percent) {
            return Err(AppError::bad_request(
                "process.cpu_percent must be between 0 and 10000",
            ));
        }
        validate_sqlite_integer("process.memory_bytes", process.memory_bytes)?;
    }
    if let Some(data) = &report.monitoring {
        monitoring::validate_data(data)?;
    }
    Ok(())
}

fn validate_usage(name: &str, used: u64, total: u64) -> Result<(), AppError> {
    validate_sqlite_integer(&format!("{name}_used_bytes"), used)?;
    validate_sqlite_integer(&format!("{name}_total_bytes"), total)?;
    if used > total {
        return Err(AppError::bad_request(format!(
            "{name} used bytes must not exceed total bytes"
        )));
    }
    Ok(())
}

fn validate_sqlite_integer(name: &str, value: u64) -> Result<(), AppError> {
    if value > i64::MAX as u64 {
        return Err(AppError::bad_request(format!(
            "{name} exceeds the supported range"
        )));
    }
    Ok(())
}

fn validate_metric_string(name: &str, value: &str) -> Result<(), AppError> {
    if value.len() > 512 {
        return Err(AppError::bad_request(format!(
            "{name} must not exceed 512 bytes"
        )));
    }
    Ok(())
}

fn validate_settings(settings: &AlertSettings) -> Result<(), AppError> {
    for (name, value) in [
        ("cpu_percent", settings.cpu_percent),
        ("memory_percent", settings.memory_percent),
        ("disk_percent", settings.disk_percent),
    ] {
        if !value.is_finite() || !(1.0..=100.0).contains(&value) {
            return Err(AppError::bad_request(format!(
                "{name} must be between 1 and 100"
            )));
        }
    }
    if !settings.temperature_celsius.is_finite()
        || !(-100.0..=250.0).contains(&settings.temperature_celsius)
    {
        return Err(AppError::bad_request(
            "temperature_celsius must be between -100 and 250",
        ));
    }
    if settings.offline_after_seconds < 5 || settings.offline_after_seconds > 24 * 60 * 60 {
        return Err(AppError::bad_request(
            "offline_after_seconds must be between 5 and 86400",
        ));
    }
    if settings.sustained_for_seconds > 24 * 60 * 60 {
        return Err(AppError::bad_request(
            "sustained_for_seconds must not exceed 86400",
        ));
    }
    if settings.email_recipients.len() > 32 {
        return Err(AppError::bad_request(
            "email_recipients may contain at most 32 addresses",
        ));
    }
    for recipient in &settings.email_recipients {
        recipient
            .parse::<Mailbox>()
            .map_err(|_| AppError::bad_request("email_recipients contains an invalid address"))?;
    }
    if settings.email_enabled && settings.email_recipients.is_empty() {
        return Err(AppError::bad_request(
            "email_recipients is required when email notifications are enabled",
        ));
    }
    if settings.webhook_url.trim().is_empty() {
        if settings.webhook_enabled {
            return Err(AppError::bad_request(
                "webhook_url is required when webhooks are enabled",
            ));
        }
    } else {
        let url = reqwest::Url::parse(&settings.webhook_url)
            .map_err(|_| AppError::bad_request("webhook_url must be a valid HTTP(S) URL"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AppError::bad_request(
                "webhook_url must be a valid HTTP(S) URL",
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(AppError::bad_request(
                "webhook_url must not contain embedded credentials",
            ));
        }
        if let Some(host) = url.host_str()
            && let Ok(address) = host.parse::<IpAddr>()
            && !is_public_webhook_ip(address)
        {
            return Err(AppError::bad_request(
                "webhook_url must not target a private or reserved address",
            ));
        }
    }
    Ok(())
}

fn notification_enabled_for_kind(
    settings: &AlertSettings,
    kind: &pinglake_protocol::AlertKind,
) -> bool {
    match kind {
        pinglake_protocol::AlertKind::Offline => settings.offline_enabled,
        pinglake_protocol::AlertKind::Cpu => settings.cpu_enabled,
        pinglake_protocol::AlertKind::Memory => settings.memory_enabled,
        pinglake_protocol::AlertKind::Disk => settings.disk_enabled,
        pinglake_protocol::AlertKind::Temperature => settings.temperature_enabled,
        pinglake_protocol::AlertKind::Service | pinglake_protocol::AlertKind::Probe => true,
    }
}

async fn send_email(
    smtp: config::SmtpConfig,
    recipients: Vec<String>,
    alert: AlertRecord,
) -> anyhow::Result<()> {
    let mut transport = AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)?.port(smtp.port);
    if let (Some(username), Some(password)) = (smtp.username, smtp.password) {
        transport = transport.credentials(Credentials::new(username, password));
    }
    let transport = transport.build();
    let from = smtp.from.parse::<Mailbox>()?;
    let state = if alert.active { "active" } else { "resolved" };
    let subject = format!("[PingLake] {} {} alert", alert.node_name, state);
    let body = format!(
        "Node: {}\nState: {}\nKind: {:?}\nMessage: {}\nTime: {}\n",
        alert.node_name,
        state,
        alert.kind,
        alert.message,
        alert.opened_at.to_rfc3339(),
    );
    for recipient in recipients {
        let message = Message::builder()
            .from(from.clone())
            .to(recipient.parse::<Mailbox>()?)
            .subject(&subject)
            .body(body.clone())?;
        transport.send(message).await?;
    }
    Ok(())
}

async fn send_webhook(url: String, alert: AlertRecord) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(&url).map_err(|_| anyhow::anyhow!("invalid webhook URL"))?;
    let client = webhook_client(&url).await?;
    let event = if alert.active {
        "alert_opened"
    } else {
        "alert_resolved"
    };
    let response = client
        .post(url)
        .json(&json!({
            "event": event,
            "source": "pinglake",
            "alert": alert,
        }))
        .send()
        .await
        .map_err(|error| anyhow::anyhow!(error.without_url()))?;
    if !response.status().is_success() {
        anyhow::bail!("webhook returned HTTP {}", response.status());
    }
    Ok(())
}

async fn webhook_client(url: &reqwest::Url) -> anyhow::Result<reqwest::Client> {
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("webhook URL contains embedded credentials");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("webhook URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("webhook URL has no usable port"))?;
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();

    if let Ok(address) = host.parse::<IpAddr>() {
        if !is_public_webhook_ip(address) {
            anyhow::bail!("webhook target is private or reserved");
        }
    } else {
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| anyhow::anyhow!("webhook host resolution failed"))?
            .collect::<Vec<SocketAddr>>();
        if addresses.is_empty() {
            anyhow::bail!("webhook host did not resolve to an address");
        }
        if addresses
            .iter()
            .any(|address| !is_public_webhook_ip(address.ip()))
        {
            anyhow::bail!("webhook host resolved to a private or reserved address");
        }
        builder = builder.resolve(host, addresses[0]);
    }

    builder
        .build()
        .map_err(|error| anyhow::anyhow!(error.without_url()))
}

fn is_public_webhook_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    if first == 0
        || address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || first >= 240
    {
        return false;
    }

    !matches!(
        (first, second, third),
        (100, 64..=127, _)
            | (192, 0, 0)
            | (192, 0, 2)
            | (192, 88, 99)
            | (198, 18..=19, _)
            | (198, 51, 100)
            | (203, 0, 113)
    )
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    if address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || address.is_unique_local()
        || address.is_unicast_link_local()
    {
        return false;
    }
    let octets = address.octets();
    let global_unicast = octets[0] & 0xe0 == 0x20;
    let documentation = octets[..4] == [0x20, 0x01, 0x0d, 0xb8];
    let nat64_well_known = octets[..12] == [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0];
    let nat64_local = octets[..6] == [0x00, 0x64, 0xff, 0x9b, 0x00, 0x01];
    global_unicast && !documentation && !nat64_well_known && !nat64_local
}

fn bearer_secret(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|secret| !secret.is_empty())
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for cookie in value.split(';') {
            let Some((name, value)) = cookie.trim().split_once('=') else {
                continue;
            };
            if name == SESSION_COOKIE && !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn session_cookie(value: &str, secure: bool, clear: bool) -> String {
    let max_age = if clear { 0 } else { SESSION_TTL.as_secs() };
    let secure_attribute = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure_attribute}"
    )
}

fn hash_admin_password(password: &str) -> anyhow::Result<String> {
    let salt_bytes = rand::random::<[u8; 16]>();
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|error| anyhow::anyhow!("could not encode password salt: {error}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow::anyhow!("could not hash administrator password: {error}"))
}

fn verify_admin_password(password: &str, encoded_hash: &str) -> bool {
    let Ok(hash) = PasswordHash::new(encoded_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .is_ok()
}

fn hash_bytes(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn hash_hex(value: &[u8]) -> String {
    let digest = hash_bytes(value);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn average(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn default_history_minutes() -> u64 {
    60
}

#[cfg(test)]
mod tests;
