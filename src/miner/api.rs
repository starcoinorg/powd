use super::schema::{build_agent_methods, AgentMethods};
use crate::types::StratumLogin;
use serde::{Deserialize, Serialize};
use starcoin_types::genesis_config::ConsensusStrategy;
use std::fmt::{Display, Formatter};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Background,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetMode {
    Auto,
    Idle,
    Light,
    Balanced,
    Aggressive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoState {
    Inactive,
    Active,
    Held,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoHoldReason {
    ManualPause,
    ManualStop,
    NotRunning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub threads: u16,
    pub cpu_percent: u8,
    pub priority: Priority,
}

#[derive(Clone, Debug)]
pub struct MinerConfig {
    pub pool: String,
    pub login: StratumLogin,
    pub pass: String,
    pub agent: String,
    pub max_threads: u16,
    pub strategy: ConsensusStrategy,
    pub keepalive_interval: Duration,
    pub status_interval: Duration,
    pub exit_after_accepted: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MinerCapabilities {
    pub max_threads: u16,
    pub supported_modes: Vec<BudgetMode>,
    pub supported_priorities: Vec<Priority>,
    pub supports_cpu_percent: bool,
    pub supports_priority: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MinerState {
    Stopped,
    Starting,
    Running,
    Paused,
    Reconnecting,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MinerSnapshot {
    pub state: MinerState,
    pub connected: bool,
    pub pool: String,
    pub worker_name: String,
    pub requested_mode: BudgetMode,
    pub effective_budget: Budget,
    pub hashrate: f64,
    pub hashrate_5m: f64,
    pub accepted: u64,
    pub accepted_5m: u64,
    pub rejected: u64,
    pub rejected_5m: u64,
    pub submitted: u64,
    pub submitted_5m: u64,
    pub reject_rate_5m: f64,
    pub reconnects: u64,
    pub uptime_secs: u64,
    pub system_cpu_percent: f64,
    pub system_memory_percent: f64,
    pub system_cpu_percent_1m: f64,
    pub system_memory_percent_1m: f64,
    pub auto_state: AutoState,
    pub auto_hold_reason: Option<AutoHoldReason>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MinerEvent {
    Started {
        snapshot: MinerSnapshot,
    },
    Paused {
        snapshot: MinerSnapshot,
    },
    Resumed {
        snapshot: MinerSnapshot,
    },
    Stopped {
        snapshot: MinerSnapshot,
    },
    Reconnecting {
        snapshot: MinerSnapshot,
    },
    BudgetChanged {
        snapshot: MinerSnapshot,
    },
    ShareAccepted {
        snapshot: MinerSnapshot,
    },
    ShareRejected {
        snapshot: MinerSnapshot,
        reason: String,
    },
    Error {
        snapshot: MinerSnapshot,
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MinerEventEnvelope {
    pub seq: u64,
    pub event: MinerEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventsSinceResponse {
    pub next_seq: u64,
    pub events: Vec<MinerEventEnvelope>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetError {
    ZeroThreads,
    ThreadsAboveMaximum { max_threads: u16 },
    CpuPercentOutOfRange,
}

impl Display for BudgetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroThreads => f.write_str("threads must be greater than zero"),
            Self::ThreadsAboveMaximum { max_threads } => {
                write!(f, "threads must not exceed max_threads ({max_threads})")
            }
            Self::CpuPercentOutOfRange => f.write_str("cpu_percent must be within 1..=100"),
        }
    }
}

impl std::error::Error for BudgetError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    EmptyPool,
    ZeroMaxThreads,
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPool => f.write_str("pool must not be empty"),
            Self::ZeroMaxThreads => f.write_str("max_threads must be greater than zero"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunnerError {
    InvalidConfig(ConfigError),
    InvalidBudget(BudgetError),
    RuntimeFailed(String),
}

impl Display for RunnerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(err) => err.fmt(f),
            Self::InvalidBudget(err) => err.fmt(f),
            Self::RuntimeFailed(err) => f.write_str(err),
        }
    }
}

impl std::error::Error for RunnerError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentError {
    NotConfigured,
    InvalidConfig(ConfigError),
    NotRunning,
    InvalidBudget(BudgetError),
    CommandChannelClosed,
    RuntimeTerminated,
    TransitionTimeout(&'static str),
    RuntimeFailed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorKind {
    NotConfigured,
    InvalidConfig,
    NotRunning,
    InvalidBudget,
    CommandChannelClosed,
    RuntimeTerminated,
    TransitionTimeout,
    RuntimeFailed,
}

impl Display for AgentError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => f.write_str("mint daemon is not configured"),
            Self::InvalidConfig(err) => err.fmt(f),
            Self::NotRunning => f.write_str("miner is not running"),
            Self::InvalidBudget(err) => err.fmt(f),
            Self::CommandChannelClosed => f.write_str("miner command channel closed"),
            Self::RuntimeTerminated => f.write_str("miner runtime terminated"),
            Self::TransitionTimeout(transition) => {
                write!(f, "miner state transition timed out: {transition}")
            }
            Self::RuntimeFailed(err) => f.write_str(err),
        }
    }
}

impl std::error::Error for AgentError {}

impl AgentError {
    pub fn kind(&self) -> AgentErrorKind {
        match self {
            Self::NotConfigured => AgentErrorKind::NotConfigured,
            Self::InvalidConfig(_) => AgentErrorKind::InvalidConfig,
            Self::NotRunning => AgentErrorKind::NotRunning,
            Self::InvalidBudget(_) => AgentErrorKind::InvalidBudget,
            Self::CommandChannelClosed => AgentErrorKind::CommandChannelClosed,
            Self::RuntimeTerminated => AgentErrorKind::RuntimeTerminated,
            Self::TransitionTimeout(_) => AgentErrorKind::TransitionTimeout,
            Self::RuntimeFailed(_) => AgentErrorKind::RuntimeFailed,
        }
    }
}

impl From<RunnerError> for AgentError {
    fn from(value: RunnerError) -> Self {
        match value {
            RunnerError::InvalidConfig(err) => Self::InvalidConfig(err),
            RunnerError::InvalidBudget(err) => Self::InvalidBudget(err),
            RunnerError::RuntimeFailed(err) => Self::RuntimeFailed(err),
        }
    }
}

impl MinerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.pool.trim().is_empty() {
            return Err(ConfigError::EmptyPool);
        }
        if self.max_threads == 0 {
            return Err(ConfigError::ZeroMaxThreads);
        }
        Ok(())
    }

    pub fn capabilities(&self) -> MinerCapabilities {
        default_miner_capabilities(self.max_threads)
    }

    pub fn methods(&self) -> AgentMethods {
        build_agent_methods(&self.capabilities())
    }
}

pub fn default_miner_capabilities(max_threads: u16) -> MinerCapabilities {
    MinerCapabilities {
        max_threads,
        supported_modes: vec![
            BudgetMode::Auto,
            BudgetMode::Idle,
            BudgetMode::Light,
            BudgetMode::Balanced,
            BudgetMode::Aggressive,
        ],
        supported_priorities: vec![Priority::Background],
        supports_cpu_percent: true,
        supports_priority: true,
    }
}

pub fn default_agent_methods(max_threads: u16) -> AgentMethods {
    build_agent_methods(&default_miner_capabilities(max_threads))
}

impl Budget {
    pub fn validate(self, max_threads: u16) -> Result<Self, BudgetError> {
        if self.threads == 0 {
            return Err(BudgetError::ZeroThreads);
        }
        if self.threads > max_threads {
            return Err(BudgetError::ThreadsAboveMaximum { max_threads });
        }
        if self.cpu_percent == 0 || self.cpu_percent > 100 {
            return Err(BudgetError::CpuPercentOutOfRange);
        }
        Ok(self)
    }
}

pub fn default_budget_for_mode(mode: BudgetMode, max_threads: u16, logical_cpus: usize) -> Budget {
    let logical_cpus = logical_cpus.max(1);
    let limit = usize::from(max_threads.max(1));
    match mode {
        BudgetMode::Auto | BudgetMode::Idle => Budget {
            threads: 1.min(limit) as u16,
            cpu_percent: 50,
            priority: Priority::Background,
        },
        BudgetMode::Light => Budget {
            threads: logical_cpus.div_ceil(4).max(1).min(limit) as u16,
            cpu_percent: 15,
            priority: Priority::Background,
        },
        BudgetMode::Balanced => Budget {
            threads: logical_cpus.div_ceil(2).max(1).min(limit) as u16,
            cpu_percent: 40,
            priority: Priority::Background,
        },
        BudgetMode::Aggressive => Budget {
            threads: logical_cpus.div_ceil(2).max(1).min(limit) as u16,
            cpu_percent: 80,
            priority: Priority::Background,
        },
    }
}
