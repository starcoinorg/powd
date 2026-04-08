use super::config::{build_miner_config, default_socket_path, default_state_path, MintProfile};
use super::reward::{fetch_wallet_reward, WalletRewardSnapshot};
use super::wallet_support::{
    profile_with_defaults, resolve_binary_from_current_exe, wait_for_daemon_ready,
    write_file_atomically, DoctorReport, WalletAgentError, WalletConfigSummary,
};
use super::{AgentClientError, AgentConnection};
use crate::{
    default_budget_for_mode, AutoHoldReason, AutoState, BudgetMode, EventsSinceResponse,
    MinerSnapshot, MinerState, MintNetwork, WalletAddress,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

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

    pub async fn status(&self) -> Result<MinerSnapshot, WalletAgentError> {
        let profile = self
            .load_profile_optional()?
            .ok_or(WalletAgentError::NotConfigured)?;
        match self.connect().await {
            Ok(mut connection) => {
                self.configure_connection(&mut connection, &profile).await?;
                connection
                    .status(self.timeout)
                    .await
                    .map_err(WalletAgentError::Rpc)
            }
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                self.synthetic_snapshot(&profile)
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
        let profile = self
            .load_profile_optional()?
            .ok_or(WalletAgentError::NotConfigured)?;
        match self.connect().await {
            Ok(mut connection) => {
                self.configure_connection(&mut connection, &profile).await?;
                connection
                    .call("miner.stop", None, self.timeout)
                    .await
                    .map_err(WalletAgentError::Rpc)
            }
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                self.synthetic_snapshot(&profile)
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn pause(&self) -> Result<MinerSnapshot, WalletAgentError> {
        let profile = self
            .load_profile_optional()?
            .ok_or(WalletAgentError::NotConfigured)?;
        match self.connect().await {
            Ok(mut connection) => {
                self.configure_connection(&mut connection, &profile).await?;
                connection
                    .call("miner.pause", None, self.timeout)
                    .await
                    .map_err(WalletAgentError::Rpc)
            }
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                self.synthetic_snapshot(&profile)
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn resume(&self) -> Result<MinerSnapshot, WalletAgentError> {
        let mut connection = self.ensure_daemon().await?;
        connection
            .call("miner.resume", None, self.timeout)
            .await
            .map_err(WalletAgentError::Rpc)
    }

    pub async fn set_mode(&self, mode: BudgetMode) -> Result<MinerSnapshot, WalletAgentError> {
        let mut profile = self
            .load_profile_optional()?
            .ok_or(WalletAgentError::NotConfigured)?;
        profile.requested_mode = mode;
        self.save_profile(&profile)?;
        match self.connect().await {
            Ok(mut connection) => self.configure_connection(&mut connection, &profile).await,
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                self.synthetic_snapshot(&profile)
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn events_since(
        &self,
        since_seq: u64,
    ) -> Result<EventsSinceResponse, WalletAgentError> {
        let Some(profile) = self.load_profile_optional()? else {
            return Ok(EventsSinceResponse {
                next_seq: 1,
                events: Vec::new(),
            });
        };
        match self.connect().await {
            Ok(mut connection) => {
                self.configure_connection(&mut connection, &profile).await?;
                connection
                    .events_since(since_seq, self.timeout)
                    .await
                    .map_err(WalletAgentError::Rpc)
            }
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                Ok(EventsSinceResponse {
                    next_seq: 1,
                    events: Vec::new(),
                })
            }
            Err(err) => Err(WalletAgentError::Rpc(err)),
        }
    }

    pub async fn set_wallet(
        &self,
        wallet_address: WalletAddress,
        network: Option<MintNetwork>,
    ) -> Result<WalletConfigSummary, WalletAgentError> {
        let mut profile = self
            .load_profile_optional()?
            .unwrap_or_else(|| profile_with_defaults(wallet_address.clone()));
        profile.wallet_address = wallet_address;
        if let Some(network) = network {
            profile.network = network;
        }
        self.save_profile(&profile)?;
        if let Ok(mut connection) = self.connect().await {
            self.configure_connection(&mut connection, &profile).await?;
        }
        self.summary(profile).await
    }

    pub async fn show_wallet(&self) -> Result<WalletConfigSummary, WalletAgentError> {
        let profile = self
            .load_profile_optional()?
            .ok_or(WalletAgentError::NotConfigured)?;
        self.summary(profile).await
    }

    pub async fn reward(&self) -> Result<WalletRewardSnapshot, WalletAgentError> {
        let profile = self
            .load_profile_optional()?
            .ok_or(WalletAgentError::NotConfigured)?;
        fetch_wallet_reward(&profile, self.timeout)
            .await
            .map_err(WalletAgentError::Reward)
    }

    pub async fn doctor(&self) -> Result<DoctorReport, WalletAgentError> {
        let profile = self.load_profile_optional()?;
        match self.connect().await {
            Ok(mut connection) => {
                let snapshot = connection
                    .status(self.timeout)
                    .await
                    .map_err(WalletAgentError::Rpc)?;
                Ok(DoctorReport {
                    wallet_configured: profile.is_some(),
                    wallet_address: profile
                        .as_ref()
                        .map(|value| value.wallet_address.to_string()),
                    worker_name: profile.as_ref().map(|value| value.worker_name.to_string()),
                    network: profile.as_ref().map(|value| value.network),
                    login: profile.as_ref().map(MintProfile::login_string),
                    requested_mode: profile.as_ref().map(|value| value.requested_mode),
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
                    wallet_address: profile
                        .as_ref()
                        .map(|value| value.wallet_address.to_string()),
                    worker_name: profile.as_ref().map(|value| value.worker_name.to_string()),
                    network: profile.as_ref().map(|value| value.network),
                    login: profile.as_ref().map(MintProfile::login_string),
                    requested_mode: profile.as_ref().map(|value| value.requested_mode),
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

    pub fn mcp_config(&self, server_only: bool) -> Result<Value, WalletAgentError> {
        let command = resolve_binary_from_current_exe("powctl")?
            .display()
            .to_string();
        let server = json!({
            "command": command,
            "args": ["mcp", "serve"],
            "env": {},
        });
        if server_only {
            Ok(server)
        } else {
            Ok(json!({
                "mcpServers": {
                    "powd": server,
                }
            }))
        }
    }

    fn synthetic_snapshot(&self, profile: &MintProfile) -> Result<MinerSnapshot, WalletAgentError> {
        let derived = build_miner_config(profile).map_err(|source| WalletAgentError::Io {
            context: "derive miner config from wallet profile",
            source: std::io::Error::other(source.to_string()),
        })?;
        Ok(MinerSnapshot {
            state: MinerState::Stopped,
            connected: false,
            pool: derived.miner_config.pool,
            worker_name: derived.miner_config.login.worker_name().to_string(),
            requested_mode: profile.requested_mode,
            effective_budget: default_budget_for_mode(
                profile.requested_mode,
                derived.miner_config.max_threads,
                std::thread::available_parallelism()
                    .map(|parallelism| parallelism.get())
                    .unwrap_or(1),
            ),
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
            system_cpu_percent: 0.0,
            system_memory_percent: 0.0,
            system_cpu_percent_1m: 0.0,
            system_memory_percent_1m: 0.0,
            auto_state: if profile.requested_mode == BudgetMode::Auto {
                AutoState::Held
            } else {
                AutoState::Inactive
            },
            auto_hold_reason: if profile.requested_mode == BudgetMode::Auto {
                Some(AutoHoldReason::NotRunning)
            } else {
                None
            },
            last_error: None,
        })
    }

    async fn summary(&self, profile: MintProfile) -> Result<WalletConfigSummary, WalletAgentError> {
        Ok(WalletConfigSummary {
            wallet_address: profile.wallet_address.to_string(),
            worker_name: profile.worker_name.to_string(),
            network: profile.network,
            login: profile.login_string(),
            state_path: self.state_path.display().to_string(),
            socket_path: self.socket_path.display().to_string(),
            daemon_running: self.daemon_running().await,
        })
    }

    async fn daemon_running(&self) -> bool {
        self.connect().await.is_ok()
    }

    async fn connect(&self) -> Result<AgentConnection, AgentClientError> {
        AgentConnection::connect(&self.socket_path, self.timeout).await
    }

    async fn ensure_daemon(&self) -> Result<AgentConnection, WalletAgentError> {
        let profile = self
            .load_profile_optional()?
            .ok_or(WalletAgentError::NotConfigured)?;
        let mut connection = match self.connect().await {
            Ok(connection) => connection,
            Err(AgentClientError::Connect { .. } | AgentClientError::Timeout { .. }) => {
                self.spawn_daemon().await?;
                self.connect().await.map_err(WalletAgentError::Rpc)?
            }
            Err(err) => return Err(WalletAgentError::Rpc(err)),
        };
        self.configure_connection(&mut connection, &profile).await?;
        Ok(connection)
    }

    async fn configure_connection(
        &self,
        connection: &mut AgentConnection,
        profile: &MintProfile,
    ) -> Result<MinerSnapshot, WalletAgentError> {
        connection
            .call(
                "daemon.configure",
                Some(json!({
                    "wallet_address": profile.wallet_address,
                    "worker_name": profile.worker_name,
                    "requested_mode": profile.requested_mode,
                    "network": profile.network,
                })),
                self.timeout,
            )
            .await
            .map_err(WalletAgentError::Rpc)
    }

    async fn spawn_daemon(&self) -> Result<(), WalletAgentError> {
        let bin = resolve_binary_from_current_exe("powd")?;
        let mut child = Command::new(bin)
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

    fn load_profile_optional(&self) -> Result<Option<MintProfile>, WalletAgentError> {
        match std::fs::read(&self.state_path) {
            Ok(bytes) => serde_json::from_slice::<MintProfile>(&bytes)
                .map(Some)
                .map_err(WalletAgentError::StateParse),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(WalletAgentError::Io {
                context: "read state file",
                source,
            }),
        }
    }

    fn save_profile(&self, profile: &MintProfile) -> Result<(), WalletAgentError> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| WalletAgentError::Io {
                context: "create state parent",
                source,
            })?;
        }
        let encoded = serde_json::to_vec_pretty(profile).map_err(WalletAgentError::StateParse)?;
        write_file_atomically(&self.state_path, &encoded).map_err(|source| WalletAgentError::Io {
            context: "write state file",
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WalletAgent;
    use crate::{MintNetwork, WalletAddress};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str, suffix: &str) -> std::path::PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        std::env::temp_dir().join(format!(
            "powd-{label}-{}-{now}.{suffix}",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn set_wallet_keeps_worker_name_and_network_defaults() {
        let socket_path = temp_path("wallet-socket", "sock");
        let state_path = temp_path("wallet-state", "json");
        let agent =
            WalletAgent::with_paths(socket_path, state_path.clone(), Duration::from_secs(1));

        let first = agent
            .set_wallet(
                WalletAddress::parse("0x11111111111111111111111111111111").unwrap(),
                None,
            )
            .await
            .expect("wallet set should succeed");
        let second = agent
            .set_wallet(
                WalletAddress::parse("0x22222222222222222222222222222222").unwrap(),
                None,
            )
            .await
            .expect("wallet update should succeed");
        assert_eq!(first.worker_name, second.worker_name);
        assert_eq!(second.network, MintNetwork::Main);

        let _ = std::fs::remove_file(state_path);
    }

    #[tokio::test]
    async fn set_wallet_can_change_network_without_replacing_worker_name() {
        let socket_path = temp_path("wallet-socket-network", "sock");
        let state_path = temp_path("wallet-state-network", "json");
        let agent =
            WalletAgent::with_paths(socket_path, state_path.clone(), Duration::from_secs(1));

        let first = agent
            .set_wallet(
                WalletAddress::parse("0x11111111111111111111111111111111").unwrap(),
                None,
            )
            .await
            .expect("wallet set should succeed");
        let second = agent
            .set_wallet(
                WalletAddress::parse("0x11111111111111111111111111111111").unwrap(),
                Some(MintNetwork::Halley),
            )
            .await
            .expect("wallet set should update network");
        assert_eq!(first.worker_name, second.worker_name);
        assert_eq!(second.network, MintNetwork::Halley);

        let _ = std::fs::remove_file(state_path);
    }

    #[test]
    fn mcp_config_points_to_powctl_mcp_command() {
        let socket_path = temp_path("mcp-config-socket", "sock");
        let state_path = temp_path("mcp-config-state", "json");
        let agent = WalletAgent::with_paths(socket_path, state_path, Duration::from_secs(1));

        let config = agent.mcp_config(false).expect("mcp_config should succeed");
        assert_eq!(
            config["mcpServers"]["powd"]["args"],
            serde_json::json!(["mcp", "serve"])
        );
        assert_eq!(config["mcpServers"]["powd"]["env"], serde_json::json!({}));
        let command = config["mcpServers"]["powd"]["command"]
            .as_str()
            .expect("command should be a string");
        assert!(std::path::Path::new(command).is_absolute());
    }

    #[test]
    fn server_only_mcp_config_returns_the_single_server_object() {
        let socket_path = temp_path("mcp-config-server-only-socket", "sock");
        let state_path = temp_path("mcp-config-server-only-state", "json");
        let agent = WalletAgent::with_paths(socket_path, state_path, Duration::from_secs(1));

        let config = agent
            .mcp_config(true)
            .expect("server_only mcp_config should succeed");
        assert_eq!(config["args"], serde_json::json!(["mcp", "serve"]));
        assert_eq!(config["env"], serde_json::json!({}));
        assert!(config.get("mcpServers").is_none());
    }
}
