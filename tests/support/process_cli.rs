use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

pub static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub fn resolve_powd_bin() -> Result<PathBuf> {
    resolve_binary("powd")
}

pub fn resolve_powctl_bin() -> Result<PathBuf> {
    resolve_binary("powctl")
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
        "powd-test-{}-{}-{}.{}",
        prefix,
        std::process::id(),
        now,
        suffix,
    ))
}

fn resolve_binary(name: &str) -> Result<PathBuf> {
    let env_var = format!("CARGO_BIN_EXE_{name}");
    if let Ok(bin) = std::env::var(&env_var) {
        return Ok(PathBuf::from(bin));
    }
    let current = std::env::current_exe().context("resolve current test executable failed")?;
    let debug_dir = current
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| anyhow::anyhow!("cannot locate target/debug directory from {current:?}"))?;
    let candidate = debug_dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    });
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(anyhow::anyhow!(
        "cannot find {name} binary via env var or target/debug"
    ))
}
