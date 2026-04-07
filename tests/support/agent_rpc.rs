#![allow(dead_code)]

use anyhow::{Context, Result};
#[allow(unused_imports)]
pub use starcoin_cpu_miner::agent::AgentConnection as RpcClient;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

use super::process::{resolve_stc_mint_agent_bin, temp_test_path};

pub struct AgentProcess {
    child: Child,
    socket_path: PathBuf,
}

impl AgentProcess {
    pub async fn spawn(pool: &str, strategy: &str, extra: &[&str]) -> Result<Self> {
        let bin = resolve_stc_mint_agent_bin()?;
        let socket_path = temp_test_path("agent", "sock");
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        let mut cmd = Command::new(bin);
        cmd.arg("--pool")
            .arg(pool)
            .arg("--login")
            .arg("0xd820b199fbaf1bc5e68eb1c511c2c3d1.agent")
            .arg("--pass")
            .arg("x")
            .arg("--consensus-strategy")
            .arg(strategy)
            .arg("--socket")
            .arg(&socket_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for arg in extra {
            cmd.arg(arg);
        }

        let mut child = cmd.spawn().context("spawn stc-mint-agent failed")?;
        wait_for_socket(&mut child, &socket_path, Duration::from_secs(6)).await?;
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
                "stc-mint-agent exited before socket ready, status: {}",
                status
            ));
        }
        if start.elapsed() > timeout {
            return Err(anyhow::anyhow!(
                "wait stc-mint-agent socket timeout: {}",
                path.display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
