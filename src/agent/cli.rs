use super::command::{AgentCommand, AgentReply, MinerAction, WalletAction};
use super::dashboard::run_dashboard;
use super::default_socket_path;
use super::mcp::run_mcp;
use super::render::{
    print_doctor_report, print_json_or_text, print_status, print_wallet_reward,
    print_wallet_summary,
};
use super::wallet::WalletAgent;
use super::wallet_support::WalletAgentError;
use crate::{BudgetMode, MintNetwork, WalletAddress};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "powd",
    about = "Operate the local powd runtime and MCP bridge",
    after_help = "Examples:\n  powd wallet set --wallet-address 0xabc...\n  powd wallet show\n  powd wallet reward\n  powd miner start\n  powd miner set-mode auto\n  powd miner watch\n  powd doctor\n  powd mcp config"
)]
pub struct AgentCliArgs {
    #[arg(
        long,
        global = true,
        help = "Unix socket path for the local powd daemon"
    )]
    socket: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "Emit machine-readable JSON output for non-TUI commands"
    )]
    json: bool,
    #[arg(
        long,
        global = true,
        default_value_t = 5,
        help = "RPC timeout in seconds for non-stream requests"
    )]
    timeout_secs: u64,
    #[command(subcommand)]
    command: AgentCliCommand,
}

#[derive(Subcommand, Debug)]
enum AgentCliCommand {
    #[command(about = "Configure and inspect the payout wallet and stable worker name")]
    Wallet {
        #[command(subcommand)]
        command: WalletCliCommand,
    },
    #[command(about = "Inspect and operate the local miner runtime")]
    Miner {
        #[command(subcommand)]
        command: MinerCliCommand,
    },
    #[command(
        about = "Check wallet configuration, daemon reachability, and current runtime state"
    )]
    Doctor,
    #[command(about = "Host-facing MCP configuration and server entrypoints")]
    Mcp {
        #[command(subcommand)]
        command: McpCliCommand,
    },
}

#[derive(Subcommand, Debug)]
enum WalletCliCommand {
    #[command(
        about = "Persist the payout wallet and optional network. On first use this creates a stable worker name.",
        after_help = "This is the only wallet write command.\n- First use: creates a stable worker name and defaults network to halley.\n- Later use: updates the wallet address, preserves worker name, and keeps the current network unless --network is given.\n- If the daemon is already running, the effective login is updated immediately."
    )]
    Set {
        #[arg(
            long,
            help = "Payout wallet address. The stable worker name is created on first use and preserved on later updates."
        )]
        wallet_address: String,
        #[arg(
            long,
            value_enum,
            help = "Network profile to use. Defaults to halley on first use; omitted later means keep the current network."
        )]
        network: Option<CliNetwork>,
    },
    #[command(
        about = "Show the persisted wallet address, worker name, network, and derived login"
    )]
    Show,
    #[command(
        about = "Query account reward totals from the configured pool-service HTTP API",
        after_help = "This is an external account query based on the persisted wallet address and network.\nIt does not depend on the local miner daemon and does not change miner runtime state."
    )]
    Reward,
}

#[derive(Subcommand, Debug)]
enum MinerCliCommand {
    #[command(about = "Read the current miner snapshot")]
    Status,
    #[command(about = "Start mining with the configured wallet identity")]
    Start,
    #[command(about = "Stop mining and disconnect from the pool")]
    Stop,
    #[command(
        about = "Pause solving without deleting wallet or daemon state",
        after_help = "When the requested mode is auto, pause keeps auto selected and places it into a hold state until you explicitly resume or start again."
    )]
    Pause,
    #[command(
        about = "Resume solving. If the miner is stopped, this starts it.",
        after_help = "When the requested mode is auto, resume clears the auto hold state and lets the daemon manage the budget again."
    )]
    Resume,
    #[command(
        about = "Set the miner mode",
        after_help = "Mode semantics:\n  auto         daemon adjusts threads and cpu_percent from system CPU and memory usage\n               auto starts from a conservative budget and never raises above the balanced ceiling by default\n  conservative fixed preset: threads=1, cpu_percent=50, priority=background\n  idle         fixed preset: threads=ceil(logical_cpus/4), cpu_percent=15, priority=background\n  balanced     fixed preset: threads=ceil(logical_cpus/2), cpu_percent=40, priority=background\n  aggressive   fixed preset: threads=ceil(logical_cpus/2), cpu_percent=80, priority=background"
    )]
    SetMode {
        #[arg(value_enum, help = "Mode to apply")]
        mode: CliBudgetMode,
    },
    #[command(
        about = "Open the local TUI dashboard for status, recent events, and basic controls",
        after_help = "The dashboard is a human-facing monitor. It is not part of the MCP tool surface."
    )]
    Watch,
}

#[derive(Subcommand, Debug)]
enum McpCliCommand {
    #[command(
        about = "Print an OpenClaw MCP registration snippet for this machine",
        after_help = "Paste the emitted JSON into the OpenClaw MCP registration workflow."
    )]
    Config {
        #[arg(
            long,
            help = "Emit just the single MCP server object for host CLIs such as `openclaw mcp set`"
        )]
        server_only: bool,
    },
    #[command(
        about = "Run the stdio MCP server that OpenClaw launches",
        after_help = "This is a host integration entrypoint. It is not a normal business command for daily manual use."
    )]
    Serve,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliBudgetMode {
    Auto,
    Conservative,
    Idle,
    Balanced,
    Aggressive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliNetwork {
    Main,
    Halley,
}

pub async fn run_cli(args: AgentCliArgs) -> ExitCode {
    match execute(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if err.json {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "error": {
                            "code": err.exit_code,
                            "message": err.message,
                        }
                    }))
                    .expect("encode cli error json")
                );
            } else {
                eprintln!("{}", err.message);
            }
            ExitCode::from(err.exit_code)
        }
    }
}

struct CliError {
    exit_code: u8,
    message: String,
    json: bool,
}

impl CliError {
    fn new(exit_code: u8, message: impl Into<String>, json: bool) -> Self {
        Self {
            exit_code,
            message: message.into(),
            json,
        }
    }
}

async fn execute(args: AgentCliArgs) -> Result<(), CliError> {
    let AgentCliArgs {
        socket,
        json,
        timeout_secs,
        command,
    } = args;
    let timeout = Duration::from_secs(timeout_secs.max(1));
    let agent = WalletAgent::new(Some(socket.unwrap_or_else(default_socket_path)), timeout);
    match command {
        AgentCliCommand::Wallet { command } => run_wallet_command(&agent, command, json).await,
        AgentCliCommand::Miner { command } => run_miner_command(&agent, command, json).await,
        AgentCliCommand::Doctor => run_doctor_command(&agent, json).await,
        AgentCliCommand::Mcp { command } => run_mcp_command(&agent, command, json).await,
    }
}

async fn run_wallet_command(
    agent: &WalletAgent,
    command: WalletCliCommand,
    json: bool,
) -> Result<(), CliError> {
    let command = match command {
        WalletCliCommand::Set {
            wallet_address,
            network,
        } => AgentCommand::Wallet(WalletAction::Set {
            wallet_address: parse_wallet_address(&wallet_address, json)?,
            network: network.map(map_network),
        }),
        WalletCliCommand::Show => AgentCommand::Wallet(WalletAction::Show),
        WalletCliCommand::Reward => AgentCommand::Wallet(WalletAction::Reward),
    };
    let reply = agent
        .execute(command)
        .await
        .map_err(|err| map_wallet_error(err, json))?;
    print_reply(reply, json);
    Ok(())
}

async fn run_miner_command(
    agent: &WalletAgent,
    command: MinerCliCommand,
    json: bool,
) -> Result<(), CliError> {
    if matches!(command, MinerCliCommand::Watch) && json {
        return Err(CliError::new(
            2,
            "--json is not supported with `miner watch`",
            json,
        ));
    }
    match command {
        MinerCliCommand::Watch => run_dashboard(agent.clone())
            .await
            .map_err(|err| CliError::new(4, format!("dashboard failed: {err}"), json)),
        other => {
            let command = match other {
                MinerCliCommand::Status => AgentCommand::Miner(MinerAction::Status),
                MinerCliCommand::Start => AgentCommand::Miner(MinerAction::Start),
                MinerCliCommand::Stop => AgentCommand::Miner(MinerAction::Stop),
                MinerCliCommand::Pause => AgentCommand::Miner(MinerAction::Pause),
                MinerCliCommand::Resume => AgentCommand::Miner(MinerAction::Resume),
                MinerCliCommand::SetMode { mode } => AgentCommand::Miner(MinerAction::SetMode {
                    mode: map_budget_mode(mode),
                }),
                MinerCliCommand::Watch => unreachable!("handled above"),
            };
            let reply = agent
                .execute(command)
                .await
                .map_err(|err| map_wallet_error(err, json))?;
            print_reply(reply, json);
            Ok(())
        }
    }
}

async fn run_doctor_command(agent: &WalletAgent, json: bool) -> Result<(), CliError> {
    let report = agent
        .doctor()
        .await
        .map_err(|err| map_wallet_error(err, json))?;
    print_json_or_text(&report, json, print_doctor_report);
    Ok(())
}

async fn run_mcp_command(
    agent: &WalletAgent,
    command: McpCliCommand,
    json: bool,
) -> Result<(), CliError> {
    match command {
        McpCliCommand::Config { server_only } => {
            let config = agent
                .mcp_config(server_only)
                .map_err(|err| map_wallet_error(err, json))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&config).expect("encode mcp config json")
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&config).expect("encode mcp config")
                );
            }
            Ok(())
        }
        McpCliCommand::Serve => run_mcp(agent.clone())
            .await
            .map_err(|err| CliError::new(4, format!("mcp server failed: {err}"), json)),
    }
}

fn print_reply(reply: AgentReply, json: bool) {
    match reply {
        AgentReply::WalletSummary(summary) => {
            print_json_or_text(&summary, json, print_wallet_summary);
        }
        AgentReply::WalletReward(snapshot) => {
            print_json_or_text(&snapshot, json, print_wallet_reward);
        }
        AgentReply::MinerSnapshot(snapshot) => {
            print_status(snapshot, json);
        }
    }
}

fn parse_wallet_address(wallet_address: &str, json: bool) -> Result<WalletAddress, CliError> {
    WalletAddress::parse(wallet_address.to_string())
        .map_err(|err| CliError::new(2, err.to_string(), json))
}

fn map_wallet_error(err: WalletAgentError, json: bool) -> CliError {
    let exit_code = match err {
        WalletAgentError::NotConfigured => 4,
        WalletAgentError::Rpc(ref inner) => match inner {
            super::AgentClientError::Connect { .. } => 3,
            super::AgentClientError::Timeout { .. } => 5,
            super::AgentClientError::Rpc(_)
            | super::AgentClientError::Io(_)
            | super::AgentClientError::Parse(_)
            | super::AgentClientError::Protocol(_) => 4,
        },
        WalletAgentError::Io { .. }
        | WalletAgentError::StateParse(_)
        | WalletAgentError::Reward(_)
        | WalletAgentError::Spawn(_)
        | WalletAgentError::BinaryNotFound { .. }
        | WalletAgentError::DaemonExited
        | WalletAgentError::DaemonStartTimeout(_) => 4,
    };
    CliError::new(exit_code, err.to_string(), json)
}

fn map_budget_mode(mode: CliBudgetMode) -> BudgetMode {
    match mode {
        CliBudgetMode::Auto => BudgetMode::Auto,
        CliBudgetMode::Conservative => BudgetMode::Conservative,
        CliBudgetMode::Idle => BudgetMode::Idle,
        CliBudgetMode::Balanced => BudgetMode::Balanced,
        CliBudgetMode::Aggressive => BudgetMode::Aggressive,
    }
}

fn map_network(network: CliNetwork) -> MintNetwork {
    match network {
        CliNetwork::Main => MintNetwork::Main,
        CliNetwork::Halley => MintNetwork::Halley,
    }
}
