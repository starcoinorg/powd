use super::client::ControlClientError;
use super::ControlConnection;
use serde::{Deserialize, Serialize};
use starcoin_types::genesis_config::ConsensusStrategy;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WalletConfigSummary {
    pub wallet_address: String,
    pub worker_id: String,
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
    pub worker_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    pub daemon_running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_state: Option<crate::MinerState>,
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
pub(crate) enum AppError {
    NotConfigured,
    InvalidWallet(crate::ParseStratumLoginError),
    Io {
        context: &'static str,
        source: std::io::Error,
    },
    StateParse(serde_json::Error),
    Control(ControlClientError),
    Spawn(std::io::Error),
    DaemonBinaryNotFound(PathBuf),
    DaemonExited,
    DaemonStartTimeout(Duration),
    DaemonStopTimeout(Duration),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PersistedState {
    pub(super) wallet_address: String,
    pub(super) worker_id: String,
}

impl PersistedState {
    pub(super) fn login(&self) -> String {
        format!("{}.{}", self.wallet_address, self.worker_id)
    }
}

impl Display for AppError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => {
                f.write_str("mint not configured; run setup with a wallet address first")
            }
            Self::InvalidWallet(err) => err.fmt(f),
            Self::Io { context, source } => write!(f, "{context} failed: {source}"),
            Self::StateParse(err) => write!(f, "parse state file failed: {err}"),
            Self::Control(err) => err.fmt(f),
            Self::Spawn(err) => write!(f, "spawn stc-mint-agent failed: {err}"),
            Self::DaemonBinaryNotFound(path) => {
                write!(
                    f,
                    "cannot find stc-mint-agent binary near {}",
                    path.display()
                )
            }
            Self::DaemonExited => f.write_str("stc-mint-agent exited before becoming ready"),
            Self::DaemonStartTimeout(timeout) => {
                write!(
                    f,
                    "stc-mint-agent did not become ready within {}s",
                    timeout.as_secs()
                )
            }
            Self::DaemonStopTimeout(timeout) => {
                write!(
                    f,
                    "stc-mint-agent did not stop within {}s",
                    timeout.as_secs()
                )
            }
        }
    }
}

impl std::error::Error for AppError {}

pub(super) fn default_max_threads() -> u16 {
    let threads = std::thread::available_parallelism()
        .map(|parallelism| usize::max(1, parallelism.get() / 2))
        .unwrap_or(1);
    u16::try_from(threads).unwrap_or(u16::MAX)
}

pub(super) fn generate_worker_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_nanos());
    format!("agent{:x}", now ^ u128::from(std::process::id()))
}

pub(super) fn resolve_binary_from_current_exe(name: &str) -> Result<PathBuf, AppError> {
    if let Ok(bin) = std::env::var(format!("CARGO_BIN_EXE_{name}")) {
        return Ok(PathBuf::from(bin));
    }
    let current = std::env::current_exe().map_err(|source| AppError::Io {
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
    Err(AppError::DaemonBinaryNotFound(current))
}

pub(super) fn consensus_strategy_name(strategy: ConsensusStrategy) -> &'static str {
    match strategy {
        ConsensusStrategy::Dummy => "dummy",
        ConsensusStrategy::Argon => "argon",
        ConsensusStrategy::Keccak => "keccak",
        ConsensusStrategy::CryptoNight => "cryptonight",
    }
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
    fs::rename(&tmp_path, path).or_else(|rename_err| {
        let _ = fs::remove_file(&tmp_path);
        Err(rename_err)
    })
}

pub(super) async fn wait_for_daemon_ready(
    child: &mut Child,
    socket_path: &Path,
    timeout: Duration,
) -> Result<(), AppError> {
    let start = Instant::now();
    loop {
        if socket_path.exists()
            && ControlConnection::connect(socket_path, Duration::from_millis(200))
                .await
                .is_ok()
        {
            return Ok(());
        }
        if child
            .try_wait()
            .map_err(|source| AppError::Io {
                context: "poll daemon child",
                source,
            })?
            .is_some()
        {
            return Err(AppError::DaemonExited);
        }
        if start.elapsed() >= timeout {
            return Err(AppError::DaemonStartTimeout(timeout));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::write_file_atomically;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        std::env::temp_dir().join(format!(
            "stc-mint-agent-{label}-{}-{now}.json",
            std::process::id()
        ))
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
}
