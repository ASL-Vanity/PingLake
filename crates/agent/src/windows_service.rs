use std::ffi::OsString;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tokio_util::sync::CancellationToken;
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_dispatcher;

use super::{PreparedAgent, Settings, StateStore, init_service_logging};

const SERVICE_NAME: &str = "PingLakeAgent";
const START_WAIT_HINT: Duration = Duration::from_secs(30);
const STOP_WAIT_HINT: Duration = Duration::from_secs(30);
const SERVICE_FAILURE_CODE: u32 = 1;

define_windows_service!(ffi_service_main, service_main);

pub fn is_service_mode() -> bool {
    is_service_mode_from(std::env::args_os())
}

pub fn dispatch() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("failed to connect PingLake Agent to the Windows Service Control Manager")
}

fn service_main(_arguments: Vec<OsString>) {
    let _ = run_service();
}

fn run_service() -> Result<()> {
    let shutdown = CancellationToken::new();
    let status_slot = Arc::new(OnceLock::<ServiceStatusHandle>::new());
    let handler_shutdown = shutdown.clone();
    let handler_status_slot = Arc::clone(&status_slot);

    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if !handler_shutdown.is_cancelled() {
                    if let Some(status_handle) = handler_status_slot.get() {
                        let _ = status_handle.set_service_status(stop_pending_status());
                    }
                    handler_shutdown.cancel();
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })
        .context("failed to register the PingLake Agent service control handler")?;
    status_slot
        .set(status_handle)
        .map_err(|_| anyhow!("service status handle was initialized more than once"))?;
    let status_handle = status_slot
        .get()
        .expect("service status handle must exist after initialization");

    status_handle
        .set_service_status(start_pending_status())
        .context("failed to report StartPending service status")?;

    let outcome = run_service_body(status_handle, shutdown.clone());

    let _ = status_handle.set_service_status(stop_pending_status());
    if let Err(error) = &outcome {
        tracing::error!(reason = %error, "PingLake Agent service stopped due to an error");
    }
    let exit_code = if outcome.is_ok() {
        ServiceExitCode::Win32(0)
    } else {
        ServiceExitCode::ServiceSpecific(SERVICE_FAILURE_CODE)
    };
    status_handle
        .set_service_status(stopped_status(exit_code))
        .context("failed to report Stopped service status")?;

    outcome
}

fn run_service_body(
    status_handle: &ServiceStatusHandle,
    shutdown: CancellationToken,
) -> Result<()> {
    let settings = Settings::load()?;
    let state_store = StateStore::open(settings.state_dir.as_deref())?;
    init_service_logging(state_store.directory())?;
    let agent = PreparedAgent::new(settings, state_store)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the agent service runtime")?;

    status_handle
        .set_service_status(running_status())
        .context("failed to report Running service status")?;
    tracing::info!("PingLake Agent service is running");

    runtime.block_on(agent.run(shutdown))
}

fn start_pending_status() -> ServiceStatus {
    status(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(0),
        1,
        START_WAIT_HINT,
    )
}

fn running_status() -> ServiceStatus {
    status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ServiceExitCode::Win32(0),
        0,
        Duration::ZERO,
    )
}

fn stop_pending_status() -> ServiceStatus {
    status(
        ServiceState::StopPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(0),
        1,
        STOP_WAIT_HINT,
    )
}

fn stopped_status(exit_code: ServiceExitCode) -> ServiceStatus {
    status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit_code,
        0,
        Duration::ZERO,
    )
}

fn status(
    current_state: ServiceState,
    controls_accepted: ServiceControlAccept,
    exit_code: ServiceExitCode,
    checkpoint: u32,
    wait_hint: Duration,
) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state,
        controls_accepted,
        exit_code,
        checkpoint,
        wait_hint,
        process_id: None,
    }
}

fn is_service_mode_from(arguments: impl IntoIterator<Item = OsString>) -> bool {
    arguments
        .into_iter()
        .skip(1)
        .any(|argument| argument == "--service")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_mode_requires_the_explicit_flag() {
        assert!(is_service_mode_from([
            OsString::from("pinglake-agent.exe"),
            OsString::from("--service"),
            OsString::from("--config"),
            OsString::from(r"C:\ProgramData\PingLake\agent.json"),
        ]));
        assert!(!is_service_mode_from([
            OsString::from("pinglake-agent.exe"),
            OsString::from("--config"),
            OsString::from(r"C:\ProgramData\PingLake\agent.json"),
        ]));
    }

    #[test]
    fn running_state_accepts_stop_and_shutdown() {
        let status = running_status();
        assert!(
            status
                .controls_accepted
                .contains(ServiceControlAccept::STOP)
        );
        assert!(
            status
                .controls_accepted
                .contains(ServiceControlAccept::SHUTDOWN)
        );
    }
}
