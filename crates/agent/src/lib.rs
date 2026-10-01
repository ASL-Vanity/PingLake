mod client;
mod config;
mod latency_endpoint;
mod metrics;
mod monitoring_runtime;
mod probes;
mod services;
mod state;

#[cfg(target_os = "windows")]
pub mod windows_service;

#[cfg(test)]
use std::time::Instant;
use std::{path::PathBuf, time::Duration};

#[cfg(target_os = "windows")]
use std::fs::{self, File, OpenOptions};
#[cfg(target_os = "windows")]
use std::io::{self, Write};
#[cfg(target_os = "windows")]
use std::path::Path;
#[cfg(target_os = "windows")]
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use config::Settings;
use metrics::MetricCollector;
use pinglake_protocol::{DEFAULT_REPORT_INTERVAL_SECS, EnrollRequest, EnrollResponse};
use state::{AgentState, StateStore};
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use tracing::error;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::client::{ApiClient, SendError};

const MIN_REPORT_INTERVAL_SECS: u64 = 1;
const MAX_REPORT_INTERVAL_SECS: u64 = 86_400;
#[cfg(target_os = "windows")]
const SERVICE_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;

pub async fn run() -> Result<()> {
    init_console_logging()?;
    let settings = Settings::load()?;
    let state_store = StateStore::open(settings.state_dir.as_deref())?;
    let agent = PreparedAgent::new(settings, state_store)?;
    let shutdown = CancellationToken::new();
    let agent_run = agent.run(shutdown.clone());
    tokio::pin!(agent_run);

    tokio::select! {
        result = &mut agent_run => result,
        signal = tokio::signal::ctrl_c() => {
            signal.context("failed to listen for the shutdown signal")?;
            shutdown.cancel();
            agent_run.await
        }
    }
}

struct PreparedAgent {
    client: ApiClient,
    enrollment_token: Option<String>,
    state: AgentState,
    collector: MetricCollector,
    enroll_request: EnrollRequest,
    configured_interval_secs: Option<u64>,
    queue_path: PathBuf,
    probe_policy: probes::ProbePolicy,
    latency_endpoint: Option<(std::net::SocketAddr, String)>,
}

impl PreparedAgent {
    fn new(settings: Settings, state_store: StateStore) -> Result<Self> {
        let queue_path = state_store.directory().join("pending-reports.json");
        let probe_policy = probes::ProbePolicy {
            allow_private: settings.allow_private_probe_targets,
            allow_loopback: settings.allow_loopback_probe_targets,
        };
        let latency_endpoint = settings.latency_bind.zip(settings.dashboard_origin.clone());
        if settings.insecure_skip_verify {
            warn!(
                "SECURITY WARNING: TLS certificate verification is disabled; use only for temporary diagnostics"
            );
        }
        if settings.allow_insecure_http {
            warn!(
                "SECURITY WARNING: non-loopback HTTP is allowed; enrollment and agent credentials may be exposed in transit"
            );
        }

        let loaded_state = state_store.load_or_create()?;
        if loaded_state.newly_created && settings.enrollment_token.is_none() {
            bail!(
                "this is a new agent identity; an enrollment token is required in the config file or PINGLAKE_ENROLLMENT_TOKEN environment variable"
            )
        }
        let state = loaded_state.state;
        let client = ApiClient::new(settings.hub_url, settings.insecure_skip_verify)?;

        let collector = MetricCollector::new();
        let host = collector.host_identity(settings.name.as_deref());
        let enroll_request = EnrollRequest {
            agent_id: state.agent_id,
            agent_secret: state.agent_secret.clone(),
            hostname: host.hostname,
            display_name: host.display_name,
            os: host.os,
            os_version: host.os_version,
            kernel_version: host.kernel_version,
            architecture: host.architecture,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        };

        Ok(Self {
            client,
            enrollment_token: settings.enrollment_token,
            state,
            collector,
            enroll_request,
            configured_interval_secs: settings.interval_secs,
            queue_path,
            probe_policy,
            latency_endpoint,
        })
    }

    async fn run(mut self, shutdown: CancellationToken) -> Result<()> {
        if shutdown.is_cancelled() {
            info!("shutdown requested; stopping agent");
            return Ok(());
        }

        info!(
            agent_id = %self.state.agent_id,
            "registering agent with PingLake Hub"
        );
        let enroll_response = match enroll_with_backoff(
            &self.client,
            self.enrollment_token.as_deref(),
            &self.enroll_request,
            &shutdown,
        )
        .await?
        {
            Some(response) => response,
            None => {
                info!("shutdown requested; stopping agent");
                return Ok(());
            }
        };
        self.enrollment_token = None;
        if !enroll_response.accepted {
            bail!("hub did not accept this agent")
        }

        let interval_secs = clamp_interval(self.configured_interval_secs.unwrap_or({
            if enroll_response.report_interval_secs == 0 {
                DEFAULT_REPORT_INTERVAL_SECS
            } else {
                enroll_response.report_interval_secs
            }
        }));
        info!(
            agent_id = %self.state.agent_id,
            interval_secs,
            "agent enrolled; metric reporting started"
        );

        monitoring_runtime::run(
            self.client,
            self.state,
            self.collector,
            interval_secs,
            enroll_response.monitoring_schema_max,
            self.queue_path,
            self.probe_policy,
            self.latency_endpoint,
            shutdown,
        )
        .await
    }
}

async fn enroll_with_backoff(
    client: &ApiClient,
    enrollment_token: Option<&str>,
    request: &EnrollRequest,
    shutdown: &CancellationToken,
) -> Result<Option<EnrollResponse>> {
    let mut backoff = Duration::from_secs(1);
    let maximum_backoff = Duration::from_secs(60);
    let mut attempts = 0_u64;

    loop {
        let enrollment = tokio::select! {
            _ = shutdown.cancelled() => return Ok(None),
            result = client.enroll(enrollment_token, request) => result,
        };
        match enrollment {
            Ok(response) => return Ok(Some(response)),
            Err(SendError::Unauthorized) => {
                bail!("hub rejected the enrollment identity or token (HTTP 401/403)")
            }
            Err(SendError::Permanent(status)) => {
                bail!("hub rejected enrollment with HTTP {status}")
            }
            Err(SendError::Transient(error)) => {
                attempts = attempts.saturating_add(1);
                warn!(
                    attempt = attempts,
                    retry_in_secs = backoff.as_secs(),
                    reason = %error,
                    "hub enrollment failed; retrying"
                );
            }
        }

        tokio::select! {
            _ = shutdown.cancelled() => return Ok(None),
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = backoff.saturating_mul(2).min(maximum_backoff);
    }
}

#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
enum SendResult {
    Sent(Duration),
    Shutdown,
}

#[cfg(test)]
async fn send_with_backoff(
    client: &ApiClient,
    state: &AgentState,
    report: &pinglake_protocol::MetricReport,
    shutdown: &CancellationToken,
) -> Result<SendResult> {
    let mut backoff = Duration::from_secs(1);
    let maximum_backoff = Duration::from_secs(60);
    let mut attempts = 0_u64;

    loop {
        let started_at = Instant::now();
        let delivery = tokio::select! {
            _ = shutdown.cancelled() => return Ok(SendResult::Shutdown),
            result = client.send_metrics(state.agent_id, &state.agent_secret, report) => result,
        };
        match delivery {
            Ok(()) => return Ok(SendResult::Sent(started_at.elapsed())),
            Err(SendError::Unauthorized) => {
                error!(
                    "hub rejected the agent identity (HTTP 401/403); stopping instead of retrying"
                );
                bail!("agent authentication was rejected by the hub")
            }
            Err(SendError::Permanent(status)) => {
                error!(
                    %status,
                    "hub rejected the metric report permanently; stopping instead of retrying"
                );
                bail!("metric report was rejected with HTTP {status}")
            }
            Err(SendError::Transient(error)) => {
                attempts = attempts.saturating_add(1);
                warn!(
                    attempt = attempts,
                    retry_in_secs = backoff.as_secs(),
                    reason = %error,
                    "metric delivery failed; retrying"
                );
            }
        }

        tokio::select! {
            _ = shutdown.cancelled() => return Ok(SendResult::Shutdown),
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = backoff.saturating_mul(2).min(maximum_backoff);
    }
}

fn clamp_interval(value: u64) -> u64 {
    value.clamp(MIN_REPORT_INTERVAL_SECS, MAX_REPORT_INTERVAL_SECS)
}

fn init_console_logging() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize console logging: {error}"))
}

#[cfg(target_os = "windows")]
fn init_service_logging(state_directory: &Path) -> Result<()> {
    let log_writer = RotatingLogWriter::open(state_directory, SERVICE_LOG_MAX_BYTES)?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false)
        .compact()
        .with_writer(Mutex::new(log_writer))
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize service file logging: {error}"))
}

#[cfg(target_os = "windows")]
struct RotatingLogWriter {
    path: PathBuf,
    rotated_path: PathBuf,
    file: Option<File>,
    size: u64,
    max_size: u64,
}

#[cfg(target_os = "windows")]
impl RotatingLogWriter {
    fn open(state_directory: &Path, max_size: u64) -> Result<Self> {
        let path = state_directory.join("pinglake-agent.log");
        let rotated_path = state_directory.join("pinglake-agent.log.1");
        let file = open_log_file(&path)?;
        let size = file
            .metadata()
            .with_context(|| format!("failed to inspect service log file {}", path.display()))?
            .len();
        let mut writer = Self {
            path,
            rotated_path,
            file: Some(file),
            size,
            max_size,
        };
        if writer.size >= writer.max_size {
            writer.rotate()?;
        }
        Ok(writer)
    }

    fn rotate_if_needed(&mut self, incoming_bytes: usize) -> io::Result<()> {
        if self.size > 0
            && self
                .size
                .saturating_add(incoming_bytes as u64)
                .gt(&self.max_size)
        {
            self.rotate().map_err(io::Error::other)?;
        }
        Ok(())
    }

    fn rotate(&mut self) -> Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush().context("failed to flush service log file")?;
            drop(file);
        }

        match fs::remove_file(&self.rotated_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                self.reopen_after_rotation_failure();
                return Err(error).with_context(|| {
                    format!(
                        "failed to remove rotated service log {}",
                        self.rotated_path.display()
                    )
                });
            }
        }
        if let Err(error) = fs::rename(&self.path, &self.rotated_path) {
            self.reopen_after_rotation_failure();
            return Err(error).with_context(|| {
                format!(
                    "failed to rotate service log {} to {}",
                    self.path.display(),
                    self.rotated_path.display()
                )
            });
        }

        self.file = Some(open_log_file(&self.path)?);
        self.size = 0;
        Ok(())
    }

    fn reopen_after_rotation_failure(&mut self) {
        self.file = open_log_file(&self.path).ok();
        self.size = self
            .file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |metadata| metadata.len());
    }
}

#[cfg(target_os = "windows")]
impl Write for RotatingLogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.rotate_if_needed(buffer.len())?;
        let written = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("service log file is unavailable"))?
            .write(buffer)?;
        self.size = self.size.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("service log file is unavailable"))?
            .flush()
    }
}

#[cfg(target_os = "windows")]
fn open_log_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open service log file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use rand::RngCore;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn report_interval_is_bounded() {
        assert_eq!(clamp_interval(0), MIN_REPORT_INTERVAL_SECS);
        assert_eq!(clamp_interval(5), 5);
        assert_eq!(
            clamp_interval(MAX_REPORT_INTERVAL_SECS + 1),
            MAX_REPORT_INTERVAL_SECS
        );
    }

    #[tokio::test]
    async fn cancellation_stops_enrollment_without_waiting_for_backoff() {
        let client = ApiClient::new(
            url::Url::parse("http://127.0.0.1:9").expect("test URL"),
            false,
        )
        .expect("API client");
        let request = EnrollRequest {
            agent_id: Uuid::new_v4(),
            agent_secret: "agent-secret".to_owned(),
            hostname: "test-host".to_owned(),
            display_name: "test-host".to_owned(),
            os: "test".to_owned(),
            os_version: "1".to_owned(),
            kernel_version: "1".to_owned(),
            architecture: "test".to_owned(),
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        let result = tokio::time::timeout(
            Duration::from_millis(250),
            enroll_with_backoff(&client, Some("enrollment-token"), &request, &shutdown),
        )
        .await
        .expect("cancellation must not wait for network backoff")
        .expect("cancellation is a clean exit");

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cancellation_stops_metric_delivery_without_waiting_for_backoff() {
        let client = ApiClient::new(
            url::Url::parse("http://127.0.0.1:9").expect("test URL"),
            false,
        )
        .expect("API client");
        let mut secret = [0_u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let state = AgentState {
            version: 1,
            agent_id: Uuid::new_v4(),
            agent_secret: URL_SAFE_NO_PAD.encode(secret),
        };
        let report = MetricCollector::new().collect();
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        let result = tokio::time::timeout(
            Duration::from_millis(250),
            send_with_backoff(&client, &state, &report, &shutdown),
        )
        .await
        .expect("cancellation must not wait for network backoff")
        .expect("cancellation is a clean exit");

        assert_eq!(result, SendResult::Shutdown);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn service_log_rotates_before_crossing_the_size_limit() {
        let directory = tempfile::tempdir().expect("temporary log directory");
        let mut writer = RotatingLogWriter::open(directory.path(), 64).expect("log writer");
        writer
            .write_all(&[b'a'; 60])
            .expect("write initial log contents");
        writer
            .write_all(&[b'b'; 8])
            .expect("write contents after rotation");
        writer.flush().expect("flush active log");

        assert_eq!(
            fs::read(directory.path().join("pinglake-agent.log.1")).expect("read rotated log"),
            vec![b'a'; 60]
        );
        assert_eq!(
            fs::read(directory.path().join("pinglake-agent.log")).expect("read active log"),
            vec![b'b'; 8]
        );
    }
}
