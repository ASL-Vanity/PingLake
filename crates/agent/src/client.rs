use std::fmt;
use std::time::Duration;

use pinglake_protocol::{EnrollRequest, EnrollResponse, MetricReport, NodeMonitoringConfig};
use reqwest::{Client, StatusCode};
use url::Url;
use uuid::Uuid;

const ENROLL_PATH: &str = "/api/v1/agent/enroll";
const METRICS_PATH: &str = "/api/v1/agent/metrics";

#[derive(Debug)]
pub enum SendError {
    Unauthorized,
    Permanent(StatusCode),
    Transient(String),
}

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => formatter.write_str("authentication was rejected"),
            Self::Permanent(status) => write!(formatter, "HTTP {status}"),
            Self::Transient(message) => formatter.write_str(message),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiClient {
    client: Client,
    enroll_url: Url,
    metrics_url: Url,
    config_url: Url,
}

impl ApiClient {
    pub fn new(mut hub_url: Url, insecure_skip_verify: bool) -> anyhow::Result<Self> {
        hub_url.set_path(ENROLL_PATH);
        let enroll_url = hub_url.clone();
        hub_url.set_path(METRICS_PATH);
        let metrics_url = hub_url.clone();
        hub_url.set_path("/api/v1/agent/config");
        let config_url = hub_url;

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(insecure_skip_verify)
            .user_agent(concat!("pinglake-agent/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            client,
            enroll_url,
            metrics_url,
            config_url,
        })
    }

    pub async fn enroll(
        &self,
        enrollment_token: Option<&str>,
        request: &EnrollRequest,
    ) -> Result<EnrollResponse, SendError> {
        let mut request_builder = self.client.post(self.enroll_url.clone()).json(request);
        if let Some(token) = enrollment_token {
            request_builder = request_builder.header("X-Enrollment-Token", token);
        }
        let response = request_builder.send().await.map_err(transport_error)?;

        classify_status(response.status())?;
        response.json().await.map_err(transport_error)
    }

    pub async fn send_metrics(
        &self,
        agent_id: Uuid,
        agent_secret: &str,
        report: &MetricReport,
    ) -> Result<(), SendError> {
        let response = self
            .client
            .post(self.metrics_url.clone())
            .header("X-Agent-ID", agent_id.to_string())
            .bearer_auth(agent_secret)
            .json(report)
            .send()
            .await
            .map_err(transport_error)?;

        classify_status(response.status())
    }

    pub async fn monitoring_config(
        &self,
        agent_id: Uuid,
        agent_secret: &str,
    ) -> Result<NodeMonitoringConfig, SendError> {
        let mut response = self
            .client
            .get(self.config_url.clone())
            .header("X-Agent-ID", agent_id.to_string())
            .bearer_auth(agent_secret)
            .header("X-Monitoring-Schema-Max", "2")
            .send()
            .await
            .map_err(transport_error)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(NodeMonitoringConfig::default());
        }
        classify_status(response.status())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > 64 * 1024 {
                return Err(SendError::Transient(
                    "configuration exceeds size limit".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| SendError::Transient("invalid monitoring configuration".into()))
    }
}

fn classify_status(status: StatusCode) -> Result<(), SendError> {
    if status.is_success() {
        return Ok(());
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Err(SendError::Unauthorized);
    }
    if status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
    {
        return Err(SendError::Transient(format!("hub returned HTTP {status}")));
    }
    Err(SendError::Permanent(status))
}

fn transport_error(error: reqwest::Error) -> SendError {
    SendError::Transient(error.without_url().to_string())
}

#[cfg(test)]
mod tests {
    use pinglake_protocol::EnrollRequest;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn classifies_auth_errors_as_terminal() {
        assert!(matches!(
            classify_status(StatusCode::UNAUTHORIZED),
            Err(SendError::Unauthorized)
        ));
        assert!(matches!(
            classify_status(StatusCode::FORBIDDEN),
            Err(SendError::Unauthorized)
        ));
    }

    #[test]
    fn retries_server_errors_and_rate_limits() {
        assert!(matches!(
            classify_status(StatusCode::SERVICE_UNAVAILABLE),
            Err(SendError::Transient(_))
        ));
        assert!(matches!(
            classify_status(StatusCode::TOO_MANY_REQUESTS),
            Err(SendError::Transient(_))
        ));
    }

    #[tokio::test]
    async fn enrollment_redirects_are_not_followed() {
        let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("redirect listener");
        let sink_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("sink listener");
        let redirect_address = redirect_listener.local_addr().expect("redirect address");
        let sink_address = sink_listener.local_addr().expect("sink address");

        let redirect_task = tokio::spawn(async move {
            let (mut stream, _) = redirect_listener.accept().await.expect("redirect request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).await.expect("read request");
            let response = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{sink_address}/capture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write redirect");
        });

        let client = ApiClient::new(
            Url::parse(&format!("http://{redirect_address}")).expect("hub URL"),
            false,
        )
        .expect("client");
        let request = EnrollRequest {
            agent_id: Uuid::new_v4(),
            agent_secret: "agent-secret-sentinel".to_owned(),
            hostname: "host".to_owned(),
            display_name: "host".to_owned(),
            os: "test".to_owned(),
            os_version: "1".to_owned(),
            kernel_version: "1".to_owned(),
            architecture: "x86_64".to_owned(),
            agent_version: "0.1.0".to_owned(),
        };
        let result = client
            .enroll(Some("enrollment-token-sentinel"), &request)
            .await;
        assert!(matches!(
            result,
            Err(SendError::Permanent(StatusCode::TEMPORARY_REDIRECT))
        ));
        redirect_task.await.expect("redirect task");

        assert!(
            tokio::time::timeout(Duration::from_millis(250), sink_listener.accept())
                .await
                .is_err(),
            "redirect target unexpectedly received the enrollment request"
        );
    }
}
