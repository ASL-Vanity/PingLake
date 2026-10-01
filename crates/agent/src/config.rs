use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use serde::Deserialize;
use url::{Host, Url};

#[derive(Debug, Parser)]
#[command(
    name = "pinglake-agent",
    version,
    about = "PingLake host metrics agent"
)]
struct Cli {
    #[cfg(target_os = "windows")]
    #[arg(long, hide = true)]
    service: bool,

    #[arg(long, env = "PINGLAKE_CONFIG", value_name = "PATH")]
    config: Option<PathBuf>,

    #[arg(long, env = "PINGLAKE_HUB_URL", value_name = "URL")]
    hub_url: Option<String>,

    #[arg(long, env = "PINGLAKE_NAME")]
    name: Option<String>,

    #[arg(long, env = "PINGLAKE_INTERVAL", value_name = "SECONDS")]
    interval: Option<u64>,

    #[arg(long, env = "PINGLAKE_STATE_DIR", value_name = "PATH")]
    state_dir: Option<PathBuf>,

    #[arg(
        long,
        env = "PINGLAKE_INSECURE_SKIP_VERIFY",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    insecure_skip_verify: Option<bool>,

    #[arg(
        long,
        env = "PINGLAKE_ALLOW_INSECURE_HTTP",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    allow_insecure_http: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    hub_url: Option<String>,
    enrollment_token: Option<String>,
    name: Option<String>,
    interval_secs: Option<u64>,
    state_dir: Option<PathBuf>,
    #[serde(default)]
    insecure_skip_verify: bool,
    #[serde(default)]
    allow_insecure_http: bool,
    #[serde(default)]
    allow_private_probe_targets: bool,
    #[serde(default)]
    allow_loopback_probe_targets: bool,
    latency_bind: Option<std::net::SocketAddr>,
    dashboard_origin: Option<String>,
}

#[derive(Debug)]
pub struct Settings {
    pub hub_url: Url,
    pub enrollment_token: Option<String>,
    pub name: Option<String>,
    pub interval_secs: Option<u64>,
    pub state_dir: Option<PathBuf>,
    pub insecure_skip_verify: bool,
    pub allow_insecure_http: bool,
    pub allow_private_probe_targets: bool,
    pub allow_loopback_probe_targets: bool,
    pub latency_bind: Option<std::net::SocketAddr>,
    pub dashboard_origin: Option<String>,
}

impl Settings {
    pub fn load() -> Result<Self> {
        let cli = Cli::parse();
        #[cfg(target_os = "windows")]
        let _ = cli.service;
        let file = match cli.config.as_deref() {
            Some(path) => load_config_file(path)?,
            None => FileConfig::default(),
        };

        let allow_insecure_http = cli.allow_insecure_http.unwrap_or(file.allow_insecure_http);
        let hub_url = cli
            .hub_url
            .or(file.hub_url)
            .context("hub URL is required in the config file, environment, or CLI")?;
        let hub_url = parse_hub_url(&hub_url, allow_insecure_http)?;

        let enrollment_token = enrollment_token_from_env()?.or(file.enrollment_token);
        if enrollment_token.as_deref().is_some_and(str::is_empty) {
            bail!("enrollment token must not be empty")
        }

        let name = cli.name.or(file.name).and_then(non_empty_string);
        if name.as_ref().is_some_and(|value| value.len() > 512) {
            bail!("agent display name must not exceed 512 UTF-8 bytes")
        }
        let interval_secs = cli.interval.or(file.interval_secs);
        if interval_secs == Some(0) {
            bail!("report interval must be at least one second")
        }
        if let Some(bind) = file.latency_bind {
            if !bind.ip().is_loopback() {
                bail!("latency_bind must be a loopback address behind an HTTPS reverse proxy")
            }
            let origin = file
                .dashboard_origin
                .as_deref()
                .context("dashboard_origin is required with latency_bind")?;
            let parsed = Url::parse(origin).context("invalid dashboard_origin")?;
            if parsed.scheme() != "https" || parsed.origin().ascii_serialization() != origin {
                bail!("dashboard_origin must be an exact HTTPS origin without path or credentials")
            }
        }

        Ok(Self {
            hub_url,
            enrollment_token,
            name,
            interval_secs,
            state_dir: cli.state_dir.or(file.state_dir),
            insecure_skip_verify: cli
                .insecure_skip_verify
                .unwrap_or(file.insecure_skip_verify),
            allow_insecure_http,
            allow_private_probe_targets: file.allow_private_probe_targets,
            allow_loopback_probe_targets: file.allow_loopback_probe_targets,
            latency_bind: file.latency_bind,
            dashboard_origin: file.dashboard_origin,
        })
    }
}

fn enrollment_token_from_env() -> Result<Option<String>> {
    let Some(value) = std::env::var_os("PINGLAKE_ENROLLMENT_TOKEN") else {
        return Ok(None);
    };
    value
        .into_string()
        .map(Some)
        .map_err(|_| anyhow::anyhow!("PINGLAKE_ENROLLMENT_TOKEN must contain valid UTF-8"))
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn parse_hub_url(value: &str, allow_insecure_http: bool) -> Result<Url> {
    let mut url = Url::parse(value).context("hub URL is invalid")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("hub URL must use http or https")
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("hub URL must not contain embedded credentials")
    }
    if url.scheme() == "http" && !is_loopback_url(&url) && !allow_insecure_http {
        bail!(
            "non-loopback hub URLs must use HTTPS; set allow_insecure_http only for explicitly accepted temporary deployments"
        )
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address == std::net::Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(address)) => address == std::net::Ipv6Addr::LOCALHOST,
        None => false,
    }
}

fn load_config_file(path: &Path) -> Result<FileConfig> {
    enforce_config_permissions(path)?;
    let bytes =
        fs::read(path).with_context(|| format!("failed to read config file {}", path.display()))?;
    let text = std::str::from_utf8(&bytes).context("config file must be UTF-8")?;

    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => serde_json::from_str(text).context("invalid JSON config file"),
        Some("toml") | None => parse_toml_config(text),
        Some(other) => bail!("unsupported config file extension: {other}"),
    }
}

fn parse_toml_config(text: &str) -> Result<FileConfig> {
    toml::from_str(text).map_err(|_| anyhow!("invalid TOML config file"))
}

#[cfg(unix)]
fn enforce_config_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect config file {}", path.display()))?;
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!(
            "config file {} is accessible by group or other users; run chmod 600 on it",
            path.display()
        )
    }
    Ok(())
}

#[cfg(not(unix))]
fn enforce_config_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_urls_without_credentials() {
        assert!(parse_hub_url("https://monitor.example.test", false).is_ok());
        assert!(parse_hub_url("http://localhost:8090", false).is_ok());
        assert!(parse_hub_url("http://127.0.0.1:8090", false).is_ok());
        assert!(parse_hub_url("http://[::1]:8090", false).is_ok());
        assert!(parse_hub_url("ftp://example.test", false).is_err());
        assert!(parse_hub_url("https://user:secret@example.test", false).is_err());
    }

    #[test]
    fn remote_http_requires_an_explicit_override() {
        assert!(parse_hub_url("http://monitor.example.test", false).is_err());
        assert!(parse_hub_url("http://192.0.2.10:8090", false).is_err());
        assert!(parse_hub_url("http://monitor.example.test", true).is_ok());
    }

    #[test]
    fn parses_toml_config_shape() {
        let config: FileConfig = toml::from_str(
            r#"
                hub_url = "https://monitor.example.test"
                enrollment_token = "token"
                name = "db-01"
                interval_secs = 10
                insecure_skip_verify = false
                allow_insecure_http = false
            "#,
        )
        .unwrap();

        assert_eq!(config.interval_secs, Some(10));
        assert_eq!(config.name.as_deref(), Some("db-01"));
    }

    #[test]
    fn toml_parse_errors_do_not_echo_secret_input() {
        let sentinel = "PINGLAKE_SECRET_SENTINEL_7xY9";
        let error = parse_toml_config(&format!(
            "hub_url = \"https://monitor.example.test\"\nenrollment_token = \"{sentinel}\n"
        ))
        .expect_err("invalid TOML must fail");
        let rendered = format!("{error:#}");
        assert!(!rendered.contains(sentinel));
        assert_eq!(rendered, "invalid TOML config file");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_service_flag_is_accepted_with_a_config_path() {
        let cli = Cli::try_parse_from([
            "pinglake-agent.exe",
            "--service",
            "--config",
            r"C:\ProgramData\PingLake\agent.json",
        ])
        .expect("Windows service command line must parse");

        assert!(cli.service);
        assert_eq!(
            cli.config.as_deref(),
            Some(Path::new(r"C:\ProgramData\PingLake\agent.json"))
        );
    }

    #[test]
    fn insecure_http_override_is_available_on_the_cli() {
        let cli = Cli::try_parse_from([
            "pinglake-agent",
            "--hub-url",
            "http://monitor.example.test",
            "--allow-insecure-http=true",
        ])
        .expect("insecure HTTP override must parse");

        assert_eq!(cli.allow_insecure_http, Some(true));
    }
}
