#[cfg(target_os = "windows")]
use anyhow::Context;
use anyhow::Result;

#[cfg(not(target_os = "windows"))]
#[tokio::main]
async fn main() -> Result<()> {
    pinglake_agent::run().await
}

#[cfg(target_os = "windows")]
fn main() -> Result<()> {
    if pinglake_agent::windows_service::is_service_mode() {
        return pinglake_agent::windows_service::dispatch();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the agent runtime")?;
    runtime.block_on(pinglake_agent::run())
}
