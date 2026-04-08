use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

pub static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub struct StratumdProcess {
    child: Child,
    ipc_path: PathBuf,
}

impl StratumdProcess {
    pub async fn spawn(listen: SocketAddr, node_rpc: &str) -> Result<Self> {
        let bin = resolve_stratumd_bin()?;
        let ipc_path = temp_test_path("ipc", "sock");
        let database_url = std::env::var("TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.trim().is_empty())
            .unwrap_or_else(|| {
                "postgresql://postgres@127.0.0.1:55432/starcoin_pool_test".to_string()
            });

        let mut cmd = Command::new(&bin);
        cmd.arg("--listen")
            .arg(listen.to_string())
            .arg("--node-rpc")
            .arg(node_rpc)
            .arg("--database-url")
            .arg(database_url)
            .arg("--ipc-path")
            .arg(ipc_path.as_os_str())
            .arg("--job-poll-ms")
            .arg("50")
            .arg("--disable-pplns")
            .arg("true")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = cmd.spawn().context("spawn stratumd failed")?;
        wait_for_server_ready(&mut child, listen, Duration::from_secs(6)).await?;
        Ok(Self { child, ipc_path })
    }
}

impl Drop for StratumdProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(self.ipc_path.as_path());
    }
}

pub fn resolve_cpu_miner_bin() -> Result<PathBuf> {
    resolve_binary("stc-mint-miner")
}

pub fn resolve_stc_mint_agent_bin() -> Result<PathBuf> {
    resolve_binary("stc-mint-agent")
}

pub fn resolve_stc_mint_agentctl_bin() -> Result<PathBuf> {
    resolve_binary("stc-mint-agentctl")
}

fn resolve_stratumd_bin() -> Result<PathBuf> {
    resolve_binary("starcoin_stratumd")
}

fn resolve_binary(name: &str) -> Result<PathBuf> {
    let env_var = format!("CARGO_BIN_EXE_{}", name);
    if let Ok(bin) = std::env::var(&env_var) {
        return Ok(PathBuf::from(bin));
    }
    let current = std::env::current_exe().context("resolve current test executable failed")?;
    let debug_dir = current
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| {
            anyhow::anyhow!("cannot locate target/debug directory from {:?}", current)
        })?;
    let candidate = debug_dir.join(if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    });
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(anyhow::anyhow!(
        "cannot find {} binary via env var or target/debug",
        name,
    ))
}

pub async fn wait_for_submit_count<F>(
    mut count: F,
    expected: usize,
    timeout: Duration,
) -> Result<()>
where
    F: FnMut() -> Result<usize>,
{
    let start = Instant::now();
    loop {
        if count()? >= expected {
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(anyhow::anyhow!(
                "wait submit count timeout: expected at least {}",
                expected,
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn wait_for_child_exit(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow::anyhow!("wait child exit timeout"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub fn pick_free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

pub fn temp_test_path(prefix: &str, suffix: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_nanos());
    std::env::temp_dir().join(format!(
        "starcoin-cpu-miner-test-{}-{}-{}.{}",
        prefix,
        std::process::id(),
        now,
        suffix,
    ))
}

async fn wait_for_server_ready(
    child: &mut Child,
    addr: SocketAddr,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    loop {
        if let Ok(stream) = TcpStream::connect(addr).await {
            drop(stream);
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(anyhow::anyhow!(
                "stratumd exited before ready, status: {}",
                status,
            ));
        }
        if start.elapsed() > timeout {
            return Err(anyhow::anyhow!("wait stratumd ready timeout: {}", addr));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
