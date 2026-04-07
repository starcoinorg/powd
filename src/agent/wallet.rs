use super::config::{
    default_agent_name, default_keepalive_interval, default_main_pass, default_main_pool,
    default_main_strategy, default_socket_path, default_state_path, default_status_interval,
};
use super::wallet_support::{
    consensus_strategy_name, default_max_threads, generate_worker_id,
    resolve_binary_from_current_exe, wait_for_daemon_ready, write_file_atomically, PersistedState,
};
pub(crate) use super::wallet_support::{DoctorReport, WalletAgentError, WalletConfigSummary};
use super::{AgentClientError, AgentConnection};
use crate::{
    default_budget_for_mode, BudgetMode, EventsSinceResponse, MinerCapabilities, MinerConfig,
    MinerSnapshot, MinerState, WalletAddress,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct WalletAgent {
    socket_path: PathBuf,
    state_path: PathBuf,
    timeout: Duration,
}

impl WalletAgent {
    pub fn new(socket_path: Option<PathBuf>, timeout: Duration) -> Self {
        Self::with_paths(
            socket_path.unwrap_or_else(default_socket_path),
            default_state_path(),
            timeout,
        )
    }

    pub(crate) fn with_paths(socket_path: PathBuf, state_path: PathBuf, timeout: Duration) -> Self {
        Self {
            socket_path,
            state_path,
            timeout,
        }
    }

    pub async fn capabilities(&self) -> Result<MinerCapabilities, WalletAgentError> {
        match self.connect().await {
            Ok(mut connection) => connection
                .capabilities(self.timeout)
                .await
                .map_err(WalletAgentError::Rpc),
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                Ok(self.local_config(None)?.capabilities())
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn methods(&self) -> Result<Value, WalletAgentError> {
        match self.connect().await {
            Ok(mut connection) => connection
                .call_value("status.methods", None, self.timeout)
                .await
                .map_err(WalletAgentError::Rpc),
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                serde_json::to_value(self.local_config(None)?.methods())
                    .map_err(WalletAgentError::StateParse)
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn status(&self) -> Result<MinerSnapshot, WalletAgentError> {
        match self.connect().await {
            Ok(mut connection) => connection
                .status(self.timeout)
                .await
                .map_err(WalletAgentError::Rpc),
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                let profile = self.load_state_optional()?;
                profile
                    .map(|profile| self.synthetic_snapshot(Some(&profile)))
                    .transpose()?
                    .ok_or(WalletAgentError::NotConfigured)
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn start(&self) -> Result<MinerSnapshot, WalletAgentError> {
        let mut connection = self.ensure_daemon().await?;
        connection
            .call("miner.start", None, self.timeout)
            .await
            .map_err(WalletAgentError::Rpc)
    }

    pub async fn stop(&self) -> Result<MinerSnapshot, WalletAgentError> {
        match self.connect().await {
            Ok(mut connection) => connection
                .call("miner.stop", None, self.timeout)
                .await
                .map_err(WalletAgentError::Rpc),
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                let profile = self.load_state_optional()?;
                profile
                    .map(|profile| self.synthetic_snapshot(Some(&profile)))
                    .transpose()?
                    .ok_or(WalletAgentError::NotConfigured)
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn pause(&self) -> Result<MinerSnapshot, WalletAgentError> {
        match self.connect().await {
            Ok(mut connection) => connection
                .call("miner.pause", None, self.timeout)
                .await
                .map_err(WalletAgentError::Rpc),
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                let profile = self.load_state_optional()?;
                profile
                    .map(|profile| self.synthetic_snapshot(Some(&profile)))
                    .transpose()?
                    .ok_or(WalletAgentError::NotConfigured)
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn resume(&self) -> Result<MinerSnapshot, WalletAgentError> {
        let mut connection = self.ensure_daemon().await?;
        let snapshot: MinerSnapshot = connection
            .call("miner.resume", None, self.timeout)
            .await
            .map_err(WalletAgentError::Rpc)?;
        if snapshot.state == MinerState::Stopped {
            return connection
                .call("miner.start", None, self.timeout)
                .await
                .map_err(WalletAgentError::Rpc);
        }
        Ok(snapshot)
    }

    pub async fn set_mode(&self, mode: BudgetMode) -> Result<MinerSnapshot, WalletAgentError> {
        let mut connection = self.ensure_daemon().await?;
        connection
            .call(
                "budget.set_mode",
                Some(json!({ "mode": mode })),
                self.timeout,
            )
            .await
            .map_err(WalletAgentError::Rpc)
    }

    pub async fn events_since(
        &self,
        since_seq: u64,
    ) -> Result<EventsSinceResponse, WalletAgentError> {
        match self.connect().await {
            Ok(mut connection) => connection
                .events_since(since_seq, self.timeout)
                .await
                .map_err(WalletAgentError::Rpc),
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                Ok(EventsSinceResponse {
                    next_seq: 1,
                    events: Vec::new(),
                })
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn setup(
        &self,
        wallet_address: &str,
    ) -> Result<WalletConfigSummary, WalletAgentError> {
        let profile = self.persist_wallet(wallet_address)?;
        if self.daemon_running().await {
            self.restart_daemon(&profile).await?;
        }
        self.summary(profile).await
    }

    pub async fn update_wallet(
        &self,
        wallet_address: &str,
    ) -> Result<WalletConfigSummary, WalletAgentError> {
        let profile = self.persist_wallet(wallet_address)?;
        if self.daemon_running().await {
            self.restart_daemon(&profile).await?;
        }
        self.summary(profile).await
    }

    pub async fn doctor(&self) -> Result<DoctorReport, WalletAgentError> {
        let profile = self.load_state_optional()?;
        match self.connect().await {
            Ok(mut connection) => {
                let snapshot = connection
                    .status(self.timeout)
                    .await
                    .map_err(WalletAgentError::Rpc)?;
                Ok(DoctorReport {
                    wallet_configured: profile.is_some(),
                    wallet_address: profile.as_ref().map(|state| state.wallet_address.clone()),
                    worker_id: profile.as_ref().map(|state| state.worker_id.clone()),
                    login: profile.as_ref().map(PersistedState::login),
                    daemon_running: true,
                    current_state: Some(snapshot.state),
                    current_pool: Some(snapshot.pool),
                    daemon_worker_name: Some(snapshot.worker_name),
                    last_error: snapshot.last_error,
                    socket_path: self.socket_path.display().to_string(),
                    state_path: self.state_path.display().to_string(),
                })
            }
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                Ok(DoctorReport {
                    wallet_configured: profile.is_some(),
                    wallet_address: profile.as_ref().map(|state| state.wallet_address.clone()),
                    worker_id: profile.as_ref().map(|state| state.worker_id.clone()),
                    login: profile.as_ref().map(PersistedState::login),
                    daemon_running: false,
                    current_state: None,
                    current_pool: None,
                    daemon_worker_name: None,
                    last_error: None,
                    socket_path: self.socket_path.display().to_string(),
                    state_path: self.state_path.display().to_string(),
                })
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub fn mcp_config(&self) -> Result<Value, WalletAgentError> {
        let command = resolve_binary_from_current_exe("stc-mint-agentctl")
            .unwrap_or_else(|_| PathBuf::from("stc-mint-agentctl"));
        Ok(json!({
            "mcpServers": {
                "stc-mint": {
                    "command": command,
                    "args": ["mcp"],
                }
            }
        }))
    }

    fn local_config(
        &self,
        profile: Option<&PersistedState>,
    ) -> Result<MinerConfig, WalletAgentError> {
        let login = match profile {
            Some(profile) => profile
                .login()
                .parse()
                .map_err(WalletAgentError::InvalidWallet)?,
            None => "0x00000000000000000000000000000000.agent"
                .parse()
                .map_err(WalletAgentError::InvalidWallet)?,
        };
        Ok(MinerConfig {
            pool: default_main_pool(),
            login,
            pass: default_main_pass(),
            agent: default_agent_name(),
            max_threads: default_max_threads(),
            strategy: default_main_strategy(),
            keepalive_interval: default_keepalive_interval(),
            status_interval: default_status_interval(),
            exit_after_accepted: None,
        })
    }

    fn synthetic_snapshot(
        &self,
        profile: Option<&PersistedState>,
    ) -> Result<MinerSnapshot, WalletAgentError> {
        let config = self.local_config(profile)?;
        Ok(MinerSnapshot {
            state: MinerState::Stopped,
            connected: false,
            pool: config.pool,
            worker_name: config.login.worker_name().to_string(),
            hashrate: 0.0,
            hashrate_5m: 0.0,
            accepted: 0,
            accepted_5m: 0,
            rejected: 0,
            rejected_5m: 0,
            submitted: 0,
            submitted_5m: 0,
            reject_rate_5m: 0.0,
            reconnects: 0,
            uptime_secs: 0,
            current_budget: default_budget_for_mode(
                BudgetMode::Conservative,
                config.max_threads,
                1,
            ),
            last_error: None,
        })
    }

    async fn summary(
        &self,
        profile: PersistedState,
    ) -> Result<WalletConfigSummary, WalletAgentError> {
        Ok(WalletConfigSummary {
            wallet_address: profile.wallet_address.clone(),
            worker_id: profile.worker_id.clone(),
            login: profile.login(),
            state_path: self.state_path.display().to_string(),
            socket_path: self.socket_path.display().to_string(),
            daemon_running: self.daemon_running().await,
        })
    }

    fn persist_wallet(&self, wallet_address: &str) -> Result<PersistedState, WalletAgentError> {
        let wallet_address = WalletAddress::parse(wallet_address.to_string())
            .map_err(WalletAgentError::InvalidWallet)?
            .to_string();
        let worker_id = self
            .load_state_optional()?
            .map(|state| state.worker_id)
            .unwrap_or_else(generate_worker_id);
        let state = PersistedState {
            wallet_address,
            worker_id,
        };
        self.save_state(&state)?;
        Ok(state)
    }

    async fn daemon_running(&self) -> bool {
        self.connect().await.is_ok()
    }

    async fn connect(&self) -> Result<AgentConnection, AgentClientError> {
        AgentConnection::connect(&self.socket_path, self.timeout).await
    }

    async fn ensure_daemon(&self) -> Result<AgentConnection, WalletAgentError> {
        match self.connect().await {
            Ok(connection) => Ok(connection),
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                let state = self
                    .load_state_optional()?
                    .ok_or(WalletAgentError::NotConfigured)?;
                self.spawn_daemon(&state).await?;
                self.connect().await.map_err(WalletAgentError::Rpc)
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    async fn restart_daemon(&self, state: &PersistedState) -> Result<(), WalletAgentError> {
        if let Ok(mut connection) = self.connect().await {
            let _ = connection
                .call_value("daemon.shutdown", None, self.timeout)
                .await
                .map_err(WalletAgentError::Rpc)?;
            self.wait_for_daemon_stop().await?;
        }
        self.spawn_daemon(state).await
    }

    async fn wait_for_daemon_stop(&self) -> Result<(), WalletAgentError> {
        let start = Instant::now();
        loop {
            match self.connect().await {
                Ok(_) => {}
                Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                    return Ok(());
                }
                Err(err) => return Err(WalletAgentError::Rpc(err)),
            }
            if start.elapsed() >= self.timeout {
                return Err(WalletAgentError::DaemonStopTimeout(self.timeout));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn spawn_daemon(&self, state: &PersistedState) -> Result<(), WalletAgentError> {
        let bin = resolve_binary_from_current_exe("stc-mint-agent")?;
        let mut child = Command::new(bin)
            .arg("--pool")
            .arg(default_main_pool())
            .arg("--login")
            .arg(state.login())
            .arg("--pass")
            .arg(default_main_pass())
            .arg("--consensus-strategy")
            .arg(consensus_strategy_name(default_main_strategy()))
            .arg("--socket")
            .arg(&self.socket_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(WalletAgentError::Spawn)?;
        wait_for_daemon_ready(&mut child, &self.socket_path, self.timeout).await?;
        let _ = child; // Detach after readiness; the daemon continues as the long-lived process.
        Ok(())
    }

    fn load_state_optional(&self) -> Result<Option<PersistedState>, WalletAgentError> {
        match std::fs::read(&self.state_path) {
            Ok(bytes) => serde_json::from_slice::<PersistedState>(&bytes)
                .map(Some)
                .map_err(WalletAgentError::StateParse),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(WalletAgentError::Io {
                context: "read state file",
                source,
            }),
        }
    }

    fn save_state(&self, state: &PersistedState) -> Result<(), WalletAgentError> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| WalletAgentError::Io {
                context: "create state directory",
                source,
            })?;
        }
        let encoded = serde_json::to_vec_pretty(state).map_err(WalletAgentError::StateParse)?;
        write_file_atomically(&self.state_path, &encoded).map_err(|source| WalletAgentError::Io {
            context: "write state file",
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn setup_and_update_wallet_keep_worker_id() {
        let socket_path = temp_path("socket");
        let state_path = temp_path("state");
        let agent =
            WalletAgent::with_paths(socket_path, state_path.clone(), Duration::from_secs(1));

        let first = agent
            .setup("0x11111111111111111111111111111111")
            .await
            .expect("setup should succeed");
        let second = agent
            .update_wallet("0x22222222222222222222222222222222")
            .await
            .expect("update_wallet should succeed");

        assert_eq!(first.worker_id, second.worker_id);
        assert_ne!(first.wallet_address, second.wallet_address);

        let persisted: PersistedState =
            serde_json::from_slice(&std::fs::read(&state_path).expect("read state file"))
                .expect("parse state file");
        assert_eq!(persisted.wallet_address, second.wallet_address);
        assert_eq!(persisted.worker_id, second.worker_id);
        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn mcp_config_points_to_agentctl_mcp_command() {
        let agent = WalletAgent::with_paths(
            PathBuf::from("/tmp/stc-mint.sock"),
            temp_path("state"),
            Duration::from_secs(1),
        );
        let config = agent.mcp_config().expect("mcp_config should succeed");
        assert_eq!(config["mcpServers"]["stc-mint"]["args"], json!(["mcp"]));
    }
}
