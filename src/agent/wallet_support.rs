use super::client::AgentClientError;
use super::config::{default_network, default_requested_mode, MintProfile};
use super::reward::RewardError;
use super::AgentConnection;
use crate::{BudgetMode, MinerState, MintNetwork, WalletAddress, WorkerName};
use serde::Serialize;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WalletConfigSummary {
    pub wallet_address: String,
    pub worker_name: String,
    pub network: MintNetwork,
    pub login: String,
    pub state_path: String,
    pub socket_path: String,
    pub daemon_running: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DoctorReport {
    pub wallet_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<MintNetwork>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_mode: Option<BudgetMode>,
    pub daemon_running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_state: Option<MinerState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_pool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_worker_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub socket_path: String,
    pub state_path: String,
}

#[derive(Debug)]
pub(crate) enum WalletAgentError {
    NotConfigured,
    Io {
        context: &'static str,
        source: std::io::Error,
    },
    StateParse(serde_json::Error),
    Rpc(AgentClientError),
    Reward(Box<RewardError>),
    Spawn(std::io::Error),
    BinaryNotFound {
        name: &'static str,
        near: PathBuf,
    },
    DaemonExited,
    DaemonStartTimeout(Duration),
}

impl Display for WalletAgentError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => {
                f.write_str("mint not configured; run `powd wallet set --wallet-address ...` first")
            }
            Self::Io { context, source } => write!(f, "{context} failed: {source}"),
            Self::StateParse(err) => write!(f, "parse state file failed: {err}"),
            Self::Rpc(err) => err.fmt(f),
            Self::Reward(err) => err.fmt(f),
            Self::Spawn(err) => write!(f, "spawn powd failed: {err}"),
            Self::BinaryNotFound { name, near } => {
                write!(f, "cannot find {name} binary near {}", near.display())
            }
            Self::DaemonExited => f.write_str("powd exited before becoming ready"),
            Self::DaemonStartTimeout(timeout) => {
                write!(f, "powd did not become ready within {}s", timeout.as_secs())
            }
        }
    }
}

impl std::error::Error for WalletAgentError {}

pub(super) fn generate_worker_name() -> WorkerName {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_nanos());
    WorkerName::parse(format!("agent{:x}", now ^ u128::from(std::process::id())))
        .expect("generated worker_name should be valid")
}

pub(super) fn profile_with_defaults(wallet_address: WalletAddress) -> MintProfile {
    MintProfile {
        wallet_address,
        worker_name: generate_worker_name(),
        requested_mode: default_requested_mode(),
        network: default_network(),
    }
}

pub(super) fn resolve_binary_from_current_exe(
    name: &'static str,
) -> Result<PathBuf, WalletAgentError> {
    if let Ok(bin) = std::env::var(format!("CARGO_BIN_EXE_{name}")) {
        return Ok(PathBuf::from(bin));
    }
    let current = std::env::current_exe().map_err(|source| WalletAgentError::Io {
        context: "resolve current executable",
        source,
    })?;
    let direct = current.with_file_name(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    });
    if direct.exists() {
        return Ok(direct);
    }
    if let Some(debug_dir) = current.parent().and_then(|path| path.parent()) {
        let candidate = debug_dir.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        });
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(WalletAgentError::BinaryNotFound {
        name,
        near: current,
    })
}

pub(super) fn current_executable_path() -> Result<PathBuf, WalletAgentError> {
    std::env::current_exe().map_err(|source| WalletAgentError::Io {
        context: "resolve current executable",
        source,
    })
}

pub(super) fn public_entry_executable_path() -> Result<PathBuf, WalletAgentError> {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_powd") {
        return Ok(PathBuf::from(bin));
    }
    let current = current_executable_path()?;
    let is_cargo_test_dep = current
        .components()
        .any(|component| component.as_os_str() == "deps");
    if !is_cargo_test_dep {
        return Ok(current);
    }
    resolve_binary_from_current_exe("powd")
}

pub(super) fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "state path must have a parent directory",
        )
    })?;
    let tmp_name = format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos())
    );
    let tmp_path = parent.join(tmp_name);
    fs::write(&tmp_path, bytes)?;
    fs::rename(&tmp_path, path).inspect_err(|_rename_err| {
        let _ = fs::remove_file(&tmp_path);
    })
}

pub(super) async fn wait_for_daemon_ready(
    child: &mut Child,
    socket_path: &Path,
    timeout: Duration,
) -> Result<(), WalletAgentError> {
    let start = Instant::now();
    loop {
        if AgentConnection::connect(socket_path, Duration::from_millis(200))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if child
            .try_wait()
            .map_err(|source| WalletAgentError::Io {
                context: "poll daemon child",
                source,
            })?
            .is_some()
        {
            return Err(WalletAgentError::DaemonExited);
        }
        if start.elapsed() >= timeout {
            return Err(WalletAgentError::DaemonStartTimeout(timeout));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_binary_from_current_exe, write_file_atomically, WalletAgentError};
    use crate::agent::config::MintProfile;
    use crate::{BudgetMode, MintNetwork, WalletAddress, WorkerName};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        std::env::temp_dir().join(format!("powd-{label}-{}-{now}.json", std::process::id()))
    }

    #[test]
    fn atomic_write_replaces_existing_contents() {
        let path = temp_path("atomic-write");
        write_file_atomically(&path, b"first").expect("write initial state");
        write_file_atomically(&path, b"second").expect("replace state");
        let bytes = std::fs::read(&path).expect("read state");
        assert_eq!(bytes, b"second");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persisted_state_defaults_to_auto_mode_and_halley_network() {
        let encoded = br#"{"wallet_address":"0x1","worker_name":"agent1"}"#;
        let decoded: MintProfile = serde_json::from_slice(encoded).expect("decode state");
        assert_eq!(decoded.requested_mode, BudgetMode::Auto);
        assert_eq!(decoded.network, MintNetwork::Halley);
        assert_eq!(decoded.wallet_address, WalletAddress::parse("0x1").unwrap());
        assert_eq!(decoded.worker_name, WorkerName::parse("agent1").unwrap());
    }

    #[test]
    fn resolving_a_missing_sibling_binary_fails() {
        let err = resolve_binary_from_current_exe("powd-not-a-real-binary")
            .expect_err("missing sibling binary should fail");
        assert!(matches!(
            err,
            WalletAgentError::BinaryNotFound {
                name: "powd-not-a-real-binary",
                ..
            }
        ));
    }
}
