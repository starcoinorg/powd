use crate::{
    default_budget_for_mode, Budget, BudgetMode, ConfigError, MinerConfig, MintNetwork,
    StratumLogin, WalletAddress, WorkerName,
};
#[cfg(not(windows))]
use anyhow::Context;
use anyhow::Result;
use clap::Parser;
use serde::{Deserialize, Serialize};
use starcoin_types::genesis_config::ConsensusStrategy;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_MAIN_POOL: &str = "main-stratum.starcoin.org:9889";
const DEFAULT_HALLEY_POOL: &str = "halley-stratum.starcoin.org:9889";
const DEFAULT_PASS: &str = "x";
const DEFAULT_MAIN_REWARD_API: &str = "https://main-pool.starcoin.org";
const DEFAULT_HALLEY_REWARD_API: &str = "https://halley-pool.starcoin.org";
const DEFAULT_AGENT_NAME: &str = "powd";
const DEFAULT_KEEPALIVE_INTERVAL_SECS: u64 = 30;
const DEFAULT_JOB_STALE_TIMEOUT_SECS: u64 = 90;
const DEFAULT_STATUS_INTERVAL_SECS: u64 = 10;

#[derive(Parser, Debug)]
#[command(
    name = "powd",
    about = "Internal daemon mode for powd. Not for direct use.",
    after_help = "This parser is only used by powd's internal hidden daemon mode."
)]
pub struct AgentArgs {
    #[arg(long, help = "Local endpoint path or name for the local API")]
    pub socket: Option<PathBuf>,
}

pub struct AgentConfig {
    pub socket_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MintProfile {
    pub wallet_address: WalletAddress,
    pub worker_name: WorkerName,
    #[serde(default = "default_requested_mode")]
    pub requested_mode: BudgetMode,
    #[serde(default = "default_network")]
    pub network: MintNetwork,
}

#[derive(Clone, Debug)]
pub(crate) struct DerivedMinerConfig {
    pub miner_config: MinerConfig,
    pub initial_budget: Budget,
}

#[derive(Clone, Debug)]
pub(crate) struct NetworkDefaults {
    pub pool: String,
    pub pass: String,
    pub strategy: ConsensusStrategy,
}

impl AgentArgs {
    pub fn into_config(self) -> AgentConfig {
        AgentConfig {
            socket_path: self.socket.unwrap_or_else(default_socket_path),
        }
    }
}

impl MintProfile {
    pub fn login_string(&self) -> String {
        format!("{}.{}", self.wallet_address, self.worker_name)
    }

    pub fn login(&self) -> StratumLogin {
        self.login_string()
            .parse()
            .expect("persisted wallet_address.worker_name should form a valid stratum login")
    }
}

pub(crate) fn build_miner_config(
    profile: &MintProfile,
) -> std::result::Result<DerivedMinerConfig, ConfigError> {
    let defaults = network_defaults(profile.network);
    let max_threads = default_max_threads();
    let miner_config = MinerConfig {
        pool: defaults.pool,
        login: profile.login(),
        pass: defaults.pass,
        agent: default_agent_name(),
        max_threads,
        strategy: defaults.strategy,
        keepalive_interval: default_keepalive_interval(),
        job_stale_timeout: default_job_stale_timeout(),
        status_interval: default_status_interval(),
        exit_after_accepted: None,
    };
    miner_config.validate()?;
    Ok(DerivedMinerConfig {
        initial_budget: default_budget_for_mode(
            profile.requested_mode,
            max_threads,
            logical_cpus(),
        ),
        miner_config,
    })
}

pub(crate) fn network_defaults(network: MintNetwork) -> NetworkDefaults {
    match network {
        MintNetwork::Main => NetworkDefaults {
            pool: std::env::var("POWD_MAIN_POOL").unwrap_or_else(|_| DEFAULT_MAIN_POOL.to_string()),
            pass: std::env::var("POWD_MAIN_PASS").unwrap_or_else(|_| DEFAULT_PASS.to_string()),
            strategy: std::env::var("POWD_MAIN_STRATEGY")
                .ok()
                .and_then(|value| parse_strategy(&value))
                .unwrap_or(ConsensusStrategy::CryptoNight),
        },
        MintNetwork::Halley => NetworkDefaults {
            pool: std::env::var("POWD_HALLEY_POOL")
                .unwrap_or_else(|_| DEFAULT_HALLEY_POOL.to_string()),
            pass: std::env::var("POWD_HALLEY_PASS").unwrap_or_else(|_| DEFAULT_PASS.to_string()),
            strategy: std::env::var("POWD_HALLEY_STRATEGY")
                .ok()
                .and_then(|value| parse_strategy(&value))
                .unwrap_or(ConsensusStrategy::CryptoNight),
        },
    }
}

pub(crate) fn reward_api_base_url(network: MintNetwork) -> String {
    let base_url = match network {
        MintNetwork::Main => std::env::var("POWD_MAIN_REWARD_API")
            .unwrap_or_else(|_| DEFAULT_MAIN_REWARD_API.to_string()),
        MintNetwork::Halley => std::env::var("POWD_HALLEY_REWARD_API")
            .unwrap_or_else(|_| DEFAULT_HALLEY_REWARD_API.to_string()),
    };
    base_url.trim_end_matches('/').to_string()
}

pub(crate) fn default_requested_mode() -> BudgetMode {
    BudgetMode::Auto
}

pub(crate) fn default_network() -> MintNetwork {
    MintNetwork::Halley
}

pub(crate) fn default_max_threads() -> u16 {
    let threads = logical_cpus().div_ceil(2).max(1);
    u16::try_from(threads).unwrap_or(u16::MAX)
}

pub(crate) fn default_agent_name() -> String {
    std::env::var("POWD_AGENT").unwrap_or_else(|_| DEFAULT_AGENT_NAME.to_string())
}

pub(crate) fn default_keepalive_interval() -> Duration {
    Duration::from_secs(
        std::env::var("POWD_KEEPALIVE_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_KEEPALIVE_INTERVAL_SECS),
    )
}

pub(crate) fn default_job_stale_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("POWD_JOB_STALE_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_JOB_STALE_TIMEOUT_SECS),
    )
}

pub(crate) fn default_status_interval() -> Duration {
    Duration::from_secs(
        std::env::var("POWD_STATUS_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_STATUS_INTERVAL_SECS),
    )
}

pub fn prepare_socket_path(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }

    #[cfg(not(windows))]
    {
        ensure_socket_parent(path)?;
        remove_stale_socket(path)?;
        Ok(())
    }
}

#[cfg(unix)]
pub fn restrict_socket_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod socket {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn restrict_socket_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn default_socket_path() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(default_windows_pipe_path())
    }

    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(default_private_runtime_dir)
            .unwrap_or_else(|| PathBuf::from("/tmp").join(private_tmp_dir_name()))
            .join("powd.sock")
    }
}

pub fn default_state_path() -> PathBuf {
    if let Some(path) = std::env::var_os("POWD_STATE_PATH") {
        return PathBuf::from(path);
    }
    default_state_root().join("state.json")
}

fn parse_strategy(value: &str) -> Option<ConsensusStrategy> {
    match value {
        "dummy" => Some(ConsensusStrategy::Dummy),
        "argon" => Some(ConsensusStrategy::Argon),
        "keccak" => Some(ConsensusStrategy::Keccak),
        "cryptonight" => Some(ConsensusStrategy::CryptoNight),
        _ => None,
    }
}

fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
}

#[cfg(not(windows))]
fn remove_stale_socket(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("remove stale socket {}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn ensure_socket_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    #[cfg(unix)]
    let existed = parent.exists();
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create socket parent {}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !existed {
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("chmod socket parent {}", parent.display()))?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn default_private_runtime_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".powd"))
}

fn default_state_root() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(path) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(path).join("powd");
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(home).join(".powd").join("state");
        }
    }

    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp").join(private_tmp_dir_name()))
        .join("powd")
}

fn private_tmp_dir_name() -> String {
    #[cfg(unix)]
    {
        format!("powd-{}", unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        "powd".to_string()
    }
}

#[cfg(windows)]
fn default_windows_pipe_path() -> String {
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string());
    let label = format!("{domain}-{user}");
    let suffix = sanitize_pipe_component(&label);
    format!(r"\\.\pipe\powd-{suffix}")
}

#[cfg(windows)]
fn sanitize_pipe_component(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
        } else if !normalized.ends_with('-') {
            normalized.push('-');
        }
    }
    let trimmed = normalized
        .trim_matches('-')
        .chars()
        .take(48)
        .collect::<String>();
    if trimmed.is_empty() {
        "user".to_string()
    } else {
        trimmed
    }
}
