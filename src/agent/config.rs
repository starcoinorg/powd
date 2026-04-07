use crate::{default_budget_for_mode, Budget, BudgetMode, MinerConfig, StratumLogin};
use anyhow::{Context, Result};
use clap::Parser;
use starcoin_types::genesis_config::ConsensusStrategy;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

const DEFAULT_MAIN_POOL: &str = "main-stratum.starcoin.org:9888";
const DEFAULT_MAIN_PASS: &str = "x";
const DEFAULT_AGENT_NAME: &str = "stc-mint-agent";
const DEFAULT_KEEPALIVE_INTERVAL_SECS: u64 = 30;
const DEFAULT_STATUS_INTERVAL_SECS: u64 = 10;

#[derive(Parser, Debug)]
#[command(
    name = "stc-mint-agent",
    after_help = "Runtime defaults:\n  Initial mode when the daemon starts: conservative\n  conservative = threads=1, cpu_percent=50, priority=background\n  Preset modes can be changed later via stc-mint-agentctl set-mode."
)]
pub struct AgentArgs {
    #[arg(
        long,
        help = "Stratum pool endpoint, for example main-stratum.starcoin.org:9888"
    )]
    pub pool: String,
    #[arg(long, help = "Stratum login in wallet.worker form")]
    pub login: String,
    #[arg(long, default_value = "x", help = "Stratum password field")]
    pub pass: String,
    #[arg(
        long,
        default_value = DEFAULT_AGENT_NAME,
        help = "Agent string sent during login"
    )]
    pub agent: String,
    #[arg(long, help = "Maximum worker threads the daemon may ever use")]
    pub max_threads: Option<usize>,
    #[arg(
        long,
        value_enum,
        default_value_t = CliConsensusStrategy::Cryptonight,
        help = "Consensus strategy used to solve shares"
    )]
    pub consensus_strategy: CliConsensusStrategy,
    #[arg(
        long,
        default_value_t = DEFAULT_KEEPALIVE_INTERVAL_SECS,
        help = "Keepalive interval in seconds"
    )]
    pub keepalive_interval_secs: u64,
    #[arg(
        long,
        default_value_t = DEFAULT_STATUS_INTERVAL_SECS,
        help = "Status log interval in seconds"
    )]
    pub status_interval_secs: u64,
    #[arg(long, help = "Unix socket path for the local API")]
    pub socket: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum CliConsensusStrategy {
    Dummy,
    Argon,
    Keccak,
    Cryptonight,
}

pub struct AgentConfig {
    pub socket_path: PathBuf,
    pub miner_config: MinerConfig,
    pub initial_budget: Budget,
}

impl AgentArgs {
    pub fn into_config(self) -> Result<AgentConfig> {
        let socket_path = self.socket.unwrap_or_else(default_socket_path);
        let login: StratumLogin = self.login.parse().with_context(|| "parse --login failed")?;
        let max_threads = parse_max_threads(self.max_threads.unwrap_or_else(default_threads))?;
        let miner_config = MinerConfig {
            pool: self.pool,
            login,
            pass: self.pass,
            agent: self.agent,
            max_threads,
            strategy: self.consensus_strategy.into(),
            keepalive_interval: Duration::from_secs(self.keepalive_interval_secs),
            status_interval: Duration::from_secs(self.status_interval_secs),
            exit_after_accepted: None,
        };
        Ok(AgentConfig {
            socket_path,
            initial_budget: default_initial_budget(max_threads),
            miner_config,
        })
    }
}

pub fn prepare_socket_path(path: &Path) -> Result<()> {
    ensure_socket_parent(path)?;
    remove_stale_socket(path)?;
    Ok(())
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

fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| usize::max(1, parallelism.get() / 2))
        .unwrap_or(1)
}

fn parse_max_threads(threads: usize) -> Result<u16> {
    u16::try_from(threads.max(1)).context("max_threads exceed u16 range")
}

fn default_initial_budget(max_threads: u16) -> Budget {
    default_budget_for_mode(BudgetMode::Conservative, max_threads, 1)
}

pub fn default_socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(default_private_runtime_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp").join(private_tmp_dir_name()))
        .join("stc-mint-agent.sock")
}

pub fn default_state_path() -> PathBuf {
    if let Some(path) = std::env::var_os("STC_MINT_AGENT_STATE_PATH") {
        return PathBuf::from(path);
    }
    default_state_root().join("state.json")
}

pub fn default_main_pool() -> String {
    std::env::var("STC_MINT_AGENT_POOL").unwrap_or_else(|_| DEFAULT_MAIN_POOL.to_string())
}

pub fn default_main_pass() -> String {
    std::env::var("STC_MINT_AGENT_PASS").unwrap_or_else(|_| DEFAULT_MAIN_PASS.to_string())
}

pub fn default_agent_name() -> String {
    std::env::var("STC_MINT_AGENT_AGENT").unwrap_or_else(|_| DEFAULT_AGENT_NAME.to_string())
}

pub fn default_keepalive_interval() -> Duration {
    Duration::from_secs(
        std::env::var("STC_MINT_AGENT_KEEPALIVE_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_KEEPALIVE_INTERVAL_SECS),
    )
}

pub fn default_status_interval() -> Duration {
    Duration::from_secs(
        std::env::var("STC_MINT_AGENT_STATUS_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_STATUS_INTERVAL_SECS),
    )
}

pub fn default_main_strategy() -> ConsensusStrategy {
    std::env::var("STC_MINT_AGENT_STRATEGY")
        .ok()
        .and_then(|value| CliConsensusStrategy::from_str(&value).ok())
        .map(ConsensusStrategy::from)
        .unwrap_or(ConsensusStrategy::CryptoNight)
}

fn remove_stale_socket(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("remove stale socket {}", path.display()))?;
    }
    Ok(())
}

fn ensure_socket_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
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

fn default_private_runtime_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".stc-mint-agent"))
}

fn default_state_root() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp").join(private_tmp_dir_name()))
        .join("stc-mint-agent")
}

fn private_tmp_dir_name() -> String {
    #[cfg(unix)]
    {
        format!("stc-mint-agent-{}", unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        "stc-mint-agent".to_string()
    }
}

impl From<CliConsensusStrategy> for ConsensusStrategy {
    fn from(value: CliConsensusStrategy) -> Self {
        match value {
            CliConsensusStrategy::Dummy => ConsensusStrategy::Dummy,
            CliConsensusStrategy::Argon => ConsensusStrategy::Argon,
            CliConsensusStrategy::Keccak => ConsensusStrategy::Keccak,
            CliConsensusStrategy::Cryptonight => ConsensusStrategy::CryptoNight,
        }
    }
}

impl FromStr for CliConsensusStrategy {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "dummy" => Ok(Self::Dummy),
            "argon" => Ok(Self::Argon),
            "keccak" => Ok(Self::Keccak),
            "cryptonight" | "cnr" => Ok(Self::Cryptonight),
            other => Err(format!("unsupported consensus strategy: {other}")),
        }
    }
}
