use anyhow::{Context, Result};
pub use powd::agent::AgentConnection as RpcClient;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

use super::process::{resolve_powd_bin, temp_test_path};

const RPC_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AgentProcess {
    child: Child,
    socket_path: PathBuf,
}

impl AgentProcess {
    pub async fn spawn(pool: &str, strategy: &str, extra: &[&str]) -> Result<Self> {
        let bin = resolve_powd_bin()?;
        let socket_path = temp_test_path("agent", "sock");
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        let mut cmd = Command::new(bin);
        cmd.env("POWD_MAIN_POOL", pool)
            .env("POWD_MAIN_STRATEGY", strategy)
            .arg("__daemon")
            .arg("--socket")
            .arg(&socket_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for arg in extra {
            cmd.arg(arg);
        }

        let mut child = cmd.spawn().context("spawn powd failed")?;
        wait_for_socket(&mut child, &socket_path, Duration::from_secs(6)).await?;
        let mut rpc = RpcClient::connect(&socket_path, RPC_TIMEOUT).await?;
        let _: serde_json::Value = rpc
            .call(
                "daemon.configure",
                Some(json!({
                    "wallet_address": "0xd820b199fbaf1bc5e68eb1c511c2c3d1",
                    "worker_name": "agent",
                    "requested_mode": "auto",
                    "network": "main",
                })),
                RPC_TIMEOUT,
            )
            .await?;
        Ok(Self { child, socket_path })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

async fn wait_for_socket(child: &mut Child, path: &Path, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        if path.exists() && UnixStream::connect(path).await.is_ok() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(anyhow::anyhow!(
                "powd exited before socket ready, status: {}",
                status
            ));
        }
        if start.elapsed() > timeout {
            return Err(anyhow::anyhow!(
                "wait powd socket timeout: {}",
                path.display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
