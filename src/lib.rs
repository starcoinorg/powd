use starcoin_types::U256;

pub mod agent;
pub mod miner;
mod mining;
pub mod protocol;
mod stratum;
mod types;

pub use miner::{
    default_agent_methods, default_budget_for_mode, default_miner_capabilities, AgentError,
    AgentErrorKind, AgentMethods, AutoHoldReason, AutoState, Budget, BudgetError, BudgetMode,
    ConfigError, EventsSinceResponse, MethodErrorSchema, MethodFieldSchema, MethodParamsSchema,
    MethodSpec, MinerCapabilities, MinerConfig, MinerEvent, MinerEventEnvelope, MinerHandle,
    MinerRunner, MinerSnapshot, MinerState, Priority, RunnerError, AGENT_API_VERSION,
};
pub use protocol::{codec, stratum_rpc};
pub use types::{
    JobId, MintNetwork, ParseMintNetworkError, ParseStratumLoginError, ParseWorkerIdError,
    ParseWorkerNameError, StratumLogin, WalletAddress, WorkerId, WorkerName,
};

pub fn difficulty_to_target_hex(difficulty: U256) -> String {
    let target = format!("{:x}", U256::from(u64::MAX) / difficulty);
    let mut temp = "0".repeat(16 - target.len());
    temp.push_str(&target);
    let mut t = hex::decode(temp).expect("decode target should not fail");
    t.reverse();
    hex::encode(&t)
}

pub fn target_hex_to_difficulty(target: &str) -> anyhow::Result<U256> {
    let mut temp = hex::decode(target)?;
    temp.reverse();
    let temp = hex::encode(temp);
    let temp = U256::from_str_radix(&temp, 16)?;
    Ok(U256::from(u64::MAX) / temp)
}
