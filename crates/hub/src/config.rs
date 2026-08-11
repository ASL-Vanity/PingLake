use std::{env, net::SocketAddr, path::PathBuf};

#[cfg(test)]
use std::net::{IpAddr, Ipv4Addr};

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_path: PathBuf,
    pub enrollment_token: String,
    pub admin_password: String,
    pub allow_weak_admin_password: bool,
    pub cookie_secure: bool,
    pub smtp: Option<SmtpConfig>,
}

#[derive(Clone, Debug)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind = env::var("PINGLAKE_BIND")
            .unwrap_or_else(|_| "0.0.0.0:8090".to_owned())
            .parse()
            .context("PINGLAKE_BIND must be a valid socket address")?;
        let database_path = env::var_os("PINGLAKE_DATABASE")
            .or_else(|| env::var_os("PINGLAKE_DB_PATH"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/pinglake.db"));
        let enrollment_token = required_secret("PINGLAKE_ENROLLMENT_TOKEN")?;
        let admin_password = required_value("PINGLAKE_ADMIN_PASSWORD")?;
        let allow_weak_admin_password = match env::var("PINGLAKE_ALLOW_WEAK_ADMIN_PASSWORD") {
            Ok(value) => {
                parse_bool(&value).context("invalid PINGLAKE_ALLOW_WEAK_ADMIN_PASSWORD")?
            }
            Err(env::VarError::NotPresent) => false,
            Err(error) => return Err(error).context("invalid PINGLAKE_ALLOW_WEAK_ADMIN_PASSWORD"),
        };
        let cookie_secure = match env::var("PINGLAKE_COOKIE_SECURE") {
            Ok(value) => parse_bool(&value).context("invalid PINGLAKE_COOKIE_SECURE")?,
            Err(env::VarError::NotPresent) => false,
            Err(error) => return Err(error).context("invalid PINGLAKE_COOKIE_SECURE"),
        };
        let smtp = SmtpConfig::from_env()?;

        let config = Self {
            bind,
            database_path,
            enrollment_token,
            admin_password,
            allow_weak_admin_password,
            cookie_secure,
            smtp,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        validate_secret("PINGLAKE_ENROLLMENT_TOKEN", &self.enrollment_token)?;
        validate_admin_password(&self.admin_password, self.allow_weak_admin_password)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn for_test(database_path: PathBuf) -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            database_path,
            enrollment_token: "test-enrollment-token".to_owned(),
            admin_password: "test-admin-password".to_owned(),
            allow_weak_admin_password: false,
            cookie_secure: false,
            smtp: None,
        }
    }
}

impl SmtpConfig {
    fn from_env() -> Result<Option<Self>> {
        let host = match env::var("PINGLAKE_SMTP_HOST") {
            Ok(value) if !value.trim().is_empty() => value,
            Ok(_) => return Ok(None),
            Err(env::VarError::NotPresent) => return Ok(None),
            Err(error) => return Err(error).context("invalid PINGLAKE_SMTP_HOST"),
        };
        let port = required_value("PINGLAKE_SMTP_PORT")?
            .parse::<u16>()
            .context("PINGLAKE_SMTP_PORT must be a valid port")?;
        if port == 0 {
            bail!("PINGLAKE_SMTP_PORT must be between 1 and 65535");
        }
        let from = required_value("PINGLAKE_SMTP_FROM")?;
        if from.trim().is_empty() || from.len() > 320 {
            bail!("PINGLAKE_SMTP_FROM must be 1-320 bytes");
        }
        let username = env::var("PINGLAKE_SMTP_USERNAME")
            .ok()
            .filter(|value| !value.is_empty());
        let password = env::var("PINGLAKE_SMTP_PASSWORD")
            .ok()
            .filter(|value| !value.is_empty());
        if username.is_some() != password.is_some() {
            bail!("PINGLAKE_SMTP_USERNAME and PINGLAKE_SMTP_PASSWORD must be set together");
        }
        Ok(Some(Self {
            host,
            port,
            username,
            password,
            from,
        }))
    }
}

fn required_secret(name: &str) -> Result<String> {
    let value = required_value(name)?;
    validate_secret(name, &value)?;
    Ok(value)
}

fn required_value(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("{name} must be set"))
}

fn validate_secret(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    if value.len() < 16 {
        bail!("{name} must be at least 16 bytes");
    }
    Ok(())
}

fn validate_admin_password(value: &str, allow_weak_password: bool) -> Result<()> {
    if value.trim().is_empty() {
        bail!("PINGLAKE_ADMIN_PASSWORD must not be empty");
    }
    if !allow_weak_password && value.len() < 16 {
        bail!("PINGLAKE_ADMIN_PASSWORD must be at least 16 bytes");
    }
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("expected true/false, yes/no, on/off, or 1/0"),
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_admin_password, validate_secret};

    #[test]
    fn root_secrets_must_be_non_blank_and_at_least_sixteen_bytes() {
        assert!(validate_secret("TEST_SECRET", "                ").is_err());
        assert!(validate_secret("TEST_SECRET", "too-short").is_err());
        assert!(validate_secret("TEST_SECRET", "sixteen-bytes-ok").is_ok());
    }

    #[test]
    fn weak_admin_passwords_require_an_explicit_test_flag() {
        assert!(validate_admin_password("123456", false).is_err());
        assert!(validate_admin_password("123456", true).is_ok());
    }
}
