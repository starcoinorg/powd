use super::wallet::WalletAgent;
use super::wallet_support::{WalletAgentError, WalletConfigSummary};
use crate::{BudgetMode, MinerSnapshot, MintNetwork, WalletAddress};
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgentCommand {
    Wallet(WalletAction),
    Miner(MinerAction),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WalletAction {
    Set {
        wallet_address: WalletAddress,
        network: Option<MintNetwork>,
    },
    Show,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MinerAction {
    Status,
    Start,
    Stop,
    Pause,
    Resume,
    SetMode { mode: BudgetMode },
}

#[derive(Clone, Debug)]
pub(crate) enum AgentReply {
    WalletSummary(WalletConfigSummary),
    MinerSnapshot(MinerSnapshot),
}

impl AgentReply {
    pub(crate) fn to_value(&self) -> Value {
        match self {
            Self::WalletSummary(value) => {
                serde_json::to_value(value).expect("encode wallet summary")
            }
            Self::MinerSnapshot(value) => {
                serde_json::to_value(value).expect("encode miner snapshot")
            }
        }
    }
}

impl WalletAgent {
    pub async fn execute(&self, command: AgentCommand) -> Result<AgentReply, WalletAgentError> {
        match command {
            AgentCommand::Wallet(command) => self.execute_wallet(command).await,
            AgentCommand::Miner(command) => self.execute_miner(command).await,
        }
    }

    async fn execute_wallet(&self, command: WalletAction) -> Result<AgentReply, WalletAgentError> {
        match command {
            WalletAction::Set {
                wallet_address,
                network,
            } => self
                .set_wallet(wallet_address, network)
                .await
                .map(AgentReply::WalletSummary),
            WalletAction::Show => self.show_wallet().await.map(AgentReply::WalletSummary),
        }
    }

    async fn execute_miner(&self, command: MinerAction) -> Result<AgentReply, WalletAgentError> {
        match command {
            MinerAction::Status => self.status().await.map(AgentReply::MinerSnapshot),
            MinerAction::Start => self.start().await.map(AgentReply::MinerSnapshot),
            MinerAction::Stop => self.stop().await.map(AgentReply::MinerSnapshot),
            MinerAction::Pause => self.pause().await.map(AgentReply::MinerSnapshot),
            MinerAction::Resume => self.resume().await.map(AgentReply::MinerSnapshot),
            MinerAction::SetMode { mode } => {
                self.set_mode(mode).await.map(AgentReply::MinerSnapshot)
            }
        }
    }
}
