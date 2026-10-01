use chrono::Utc;
use pinglake_protocol::{MetricStatus, NodeMonitoringConfig, ServiceCheck, ServiceResult};

pub async fn check_services(config: &NodeMonitoringConfig) -> Vec<ServiceResult> {
    let mut results = Vec::new();
    let checks = config
        .services
        .iter()
        .filter(|check| check.enabled)
        .take(32)
        .collect::<Vec<_>>();
    // Limit native queries and systemctl processes while checking each configured service.
    for batch in checks.chunks(4) {
        let mut jobs = tokio::task::JoinSet::new();
        for check in batch {
            let check = (*check).clone();
            let revision = config.revision;
            jobs.spawn(async move { query(check, revision).await });
        }
        while let Some(result) = jobs.join_next().await {
            match result {
                Ok(value) => results.push(value),
                Err(error) => tracing::warn!(reason = %error, "service query task failed"),
            }
        }
    }
    results.sort_by_key(|value| value.id);
    results
}

async fn query(check: ServiceCheck, revision: u64) -> ServiceResult {
    let mut result = ServiceResult {
        id: check.id,
        name: check.name.clone(),
        checked_at: Utc::now(),
        status: MetricStatus::Unavailable,
        state: "unknown".into(),
        healthy: None,
        error: None,
        config_revision: revision,
    };
    if !valid_name(&check.name) || !matches!(check.expected_state.as_str(), "running" | "stopped") {
        result.error = Some("invalid service configuration".into());
        return result;
    }
    let native = native_query(&check.name).await;
    match native {
        Ok(state) => {
            result.status = MetricStatus::Ok;
            result.healthy = Some(state == check.expected_state);
            result.state = state;
        }
        Err((status, error)) => {
            result.status = status;
            result.error = Some(error);
        }
    }
    result.checked_at = Utc::now();
    result
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && !name.starts_with('-')
        && name
            .chars()
            .all(|value| value.is_alphanumeric() || "._@- ".contains(value))
}

#[cfg(target_os = "linux")]
async fn native_query(name: &str) -> Result<String, (MetricStatus, String)> {
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio::process::Command;

    if !std::path::Path::new("/run/systemd/system").is_dir() {
        return Err((MetricStatus::Unsupported, "systemd is not available".into()));
    }
    let executable = ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .ok_or_else(|| {
            (
                MetricStatus::Unsupported,
                "systemctl is not installed".into(),
            )
        })?;
    let mut command = Command::new(executable);
    command
        .args([
            "show",
            "--no-pager",
            "--no-ask-password",
            "--property=LoadState,ActiveState,SubState",
            "--",
            name,
        ])
        .env_clear()
        .env("LC_ALL", "C")
        .env("SYSTEMD_COLORS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        (
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                MetricStatus::PermissionDenied
            } else {
                MetricStatus::Unavailable
            },
            "failed to start systemctl".into(),
        )
    })?;
    let mut stdout = child.stdout.take().expect("piped stdout").take(4097);
    let mut stderr = child.stderr.take().expect("piped stderr").take(4097);
    let operation = async {
        let mut bytes = Vec::new();
        let mut errors = Vec::new();
        let (output, error_output) = tokio::join!(
            stdout.read_to_end(&mut bytes),
            stderr.read_to_end(&mut errors)
        );
        output.and(error_output).map_err(|_| {
            (
                MetricStatus::Unavailable,
                "failed to read service state".into(),
            )
        })?;
        if bytes.len() > 4096 || errors.len() > 4096 {
            return Err((
                MetricStatus::Unavailable,
                "service query output exceeded limit".into(),
            ));
        }
        let exit = child
            .wait()
            .await
            .map_err(|_| (MetricStatus::Unavailable, "service query failed".into()))?;
        if !exit.success() {
            let message = String::from_utf8_lossy(&errors);
            if message.contains("Access denied") || message.contains("Permission denied") {
                return Err((
                    MetricStatus::PermissionDenied,
                    "systemd denied the service query".into(),
                ));
            }
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            (
                MetricStatus::Unavailable,
                "invalid service query response".into(),
            )
        })?;
        // systemctl may return a failure code for a missing unit while still producing its properties.
        let state = parse_systemd(text)?;
        if !exit.success() && state != "not_found" {
            return Err((MetricStatus::Unavailable, "systemctl query failed".into()));
        }
        Ok(state)
    };
    match tokio::time::timeout(Duration::from_secs(3), operation).await {
        Ok(value) => value,
        Err(_) => Err((MetricStatus::Unavailable, "service query timed out".into())),
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_systemd(text: &str) -> Result<String, (MetricStatus, String)> {
    let mut load = None;
    let mut active = None;
    for line in text.lines() {
        match line.split_once('=') {
            Some(("LoadState", value)) => load = Some(value),
            Some(("ActiveState", value)) => active = Some(value),
            _ => {}
        }
    }
    if load == Some("not-found") {
        return Ok("not_found".into());
    }
    if load.is_none() {
        return Err((
            MetricStatus::Unavailable,
            "missing service load state".into(),
        ));
    }
    match active {
        Some("active") => Ok("running".into()),
        Some("inactive") => Ok("stopped".into()),
        Some("failed") => Ok("failed".into()),
        Some("activating") => Ok("starting".into()),
        Some("deactivating") => Ok("stopping".into()),
        Some("reloading") => Ok("reloading".into()),
        _ => Err((
            MetricStatus::Unavailable,
            "unknown service active state".into(),
        )),
    }
}

#[cfg(target_os = "windows")]
async fn native_query(name: &str) -> Result<String, (MetricStatus, String)> {
    let name = name.to_owned();
    tokio::task::spawn_blocking(move || windows_query(&name))
        .await
        .map_err(|_| {
            (
                MetricStatus::Unavailable,
                "native service query failed".into(),
            )
        })?
}

#[cfg(target_os = "windows")]
fn windows_query(name: &str) -> Result<String, (MetricStatus, String)> {
    use std::ptr;
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_SERVICE_DOES_NOT_EXIST, GetLastError,
    };
    use windows_sys::Win32::System::Services::*;

    struct ServiceHandle(SC_HANDLE);
    impl Drop for ServiceHandle {
        fn drop(&mut self) {
            unsafe { CloseServiceHandle(self.0) };
        }
    }
    fn error(code: u32) -> (MetricStatus, String) {
        (
            if code == ERROR_ACCESS_DENIED {
                MetricStatus::PermissionDenied
            } else {
                MetricStatus::Unavailable
            },
            format!("SCM error {code}"),
        )
    }
    let manager = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT) };
    if manager.is_null() {
        return Err(error(unsafe { GetLastError() }));
    }
    let manager = ServiceHandle(manager);
    let wide = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let service = unsafe { OpenServiceW(manager.0, wide.as_ptr(), SERVICE_QUERY_STATUS) };
    if service.is_null() {
        let code = unsafe { GetLastError() };
        if code == ERROR_SERVICE_DOES_NOT_EXIST {
            return Ok("not_found".into());
        }
        return Err(error(code));
    }
    let service = ServiceHandle(service);
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0;
    if unsafe {
        QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
            std::mem::size_of_val(&status) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(error(unsafe { GetLastError() }));
    }
    let state = match status.dwCurrentState {
        SERVICE_RUNNING => "running",
        SERVICE_STOPPED => "stopped",
        SERVICE_START_PENDING => "starting",
        SERVICE_STOP_PENDING => "stopping",
        SERVICE_PAUSED => "paused",
        SERVICE_PAUSE_PENDING => "pausing",
        SERVICE_CONTINUE_PENDING => "resuming",
        _ => return Err((MetricStatus::Unavailable, "unknown SCM state".into())),
    };
    Ok(state.into())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
async fn native_query(_: &str) -> Result<String, (MetricStatus, String)> {
    Err((
        MetricStatus::Unsupported,
        "service queries are unsupported on this platform".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_option_and_shell_injection_names() {
        for name in [
            "--all",
            "nginx;id",
            "$(id)",
            "../service",
            "service\nname",
            "",
        ] {
            assert!(!valid_name(name));
        }
        for name in [
            "nginx.service",
            "postgresql@15-main.service",
            "Windows Search",
        ] {
            assert!(valid_name(name));
        }
    }
    #[test]
    fn systemd_distinguishes_stopped_failed_and_not_found() {
        assert_eq!(
            parse_systemd("LoadState=loaded\nActiveState=active\nSubState=running").unwrap(),
            "running"
        );
        assert_eq!(
            parse_systemd("LoadState=loaded\nActiveState=inactive").unwrap(),
            "stopped"
        );
        assert_eq!(
            parse_systemd("LoadState=loaded\nActiveState=failed").unwrap(),
            "failed"
        );
        assert_eq!(
            parse_systemd("LoadState=not-found\nActiveState=inactive").unwrap(),
            "not_found"
        );
        assert!(parse_systemd("ActiveState=active").is_err());
    }
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn native_scm_observes_missing_service_without_false_healthy() {
        let config = NodeMonitoringConfig {
            revision: 7,
            services: vec![ServiceCheck {
                id: uuid::Uuid::new_v4(),
                name: "PingLakeMissingServiceTest70C2A9".into(),
                enabled: true,
                expected_state: "stopped".into(),
            }],
            ..Default::default()
        };
        let results = check_services(&config).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, MetricStatus::Ok);
        assert_eq!(results[0].state, "not_found");
        assert_eq!(results[0].healthy, Some(false));
        assert_eq!(results[0].config_revision, 7);
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn missing_systemd_has_explicit_unsupported_status() {
        if std::path::Path::new("/run/systemd/system").is_dir() {
            return;
        }
        let result = native_query("nginx.service").await.unwrap_err();
        assert_eq!(result.0, MetricStatus::Unsupported);
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn native_systemd_distinguishes_a_live_unit_from_missing_unit() {
        if !std::path::Path::new("/run/systemd/system").is_dir() {
            return;
        }
        assert_eq!(native_query("dbus.service").await.unwrap(), "running");
        assert_eq!(
            native_query("pinglake-missing-test-70c2a9.service")
                .await
                .unwrap(),
            "not_found"
        );
    }
}
