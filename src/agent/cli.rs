use super::dashboard::run_dashboard;
use super::mcp::run_mcp;
use super::render::{
    format_event, print_capabilities, print_doctor_report, print_events_since, print_json_or_text,
    print_methods, print_status, print_wallet_summary,
};
use super::wallet::{WalletAgent, WalletAgentError};
use super::{default_socket_path, AgentClientError, AgentConnection};
use crate::{AgentMethods, BudgetMode, MinerSnapshot, Priority};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "stc-mint-agentctl",
    about = "Operate a local stc-mint-agent daemon over its Unix socket",
    after_help = "Examples:\n  stc-mint-agentctl setup --wallet-address 0xabc...\n  stc-mint-agentctl --json status\n  stc-mint-agentctl set-mode conservative\n  stc-mint-agentctl doctor\n  stc-mint-agentctl mcp-config"
)]
pub struct AgentCliArgs {
    #[arg(
        long,
        global = true,
        help = "Unix socket path for the local stc-mint-agent daemon"
    )]
    socket: Option<PathBuf>,
    #[arg(long, global = true, help = "Emit machine-readable JSON output")]
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
    #[command(about = "Configure the payout wallet and create a stable worker id")]
    Setup {
        #[arg(long, help = "Payout wallet address")]
        wallet_address: String,
    },
    #[command(about = "Set or replace the payout wallet while keeping the stable worker id")]
    SetWallet {
        #[arg(long, help = "New payout wallet address")]
        wallet_address: String,
    },
    #[command(about = "Show the current miner snapshot or local stopped state")]
    Status,
    #[command(about = "Show runtime capabilities such as supported modes and limits")]
    Capabilities,
    #[command(about = "Show self-describing local API methods and parameter schema")]
    Methods,
    #[command(about = "Start mining with the configured payout wallet")]
    Start,
    #[command(about = "Stop mining and disconnect from the pool")]
    Stop,
    #[command(about = "Pause solving while keeping config and daemon alive")]
    Pause,
    #[command(about = "Resume solving; starts mining if currently stopped")]
    Resume,
    #[command(
        about = "Apply a preset budget mode",
        after_help = "Mode mapping:\n  conservative threads=1, cpu_percent=50, priority=background\n  idle         threads=ceil(logical_cpus/4), cpu_percent=15, priority=background\n  balanced     threads=ceil(logical_cpus/2), cpu_percent=40, priority=background\n  aggressive   threads=ceil(logical_cpus/2), cpu_percent=80, priority=background"
    )]
    SetMode {
        #[arg(
            value_enum,
            help = "Budget mode to apply; see the mode mapping below for exact threads/cpu_percent values"
        )]
        mode: CliBudgetMode,
    },
    #[command(about = "Set one or more budget fields explicitly")]
    SetBudget(SetBudgetArgs),
    #[command(about = "Fetch buffered events after a sequence number")]
    EventsSince {
        #[arg(
            long,
            help = "Return events with seq greater than this value; use 0 to read the current buffer from the beginning"
        )]
        since_seq: u64,
    },
    #[command(about = "Stream live events until interrupted")]
    Events,
    #[command(about = "Check wallet setup, daemon reachability, and current runtime state")]
    Doctor,
    #[command(about = "Print an MCP server registration snippet for OpenClaw")]
    McpConfig,
    #[command(about = "Run a stdio MCP server for OpenClaw")]
    Mcp,
    #[command(about = "Open a local TUI dashboard for status and basic operations")]
    Dashboard,
}

#[derive(Args, Debug)]
struct SetBudgetArgs {
    #[arg(long, help = "Set the active worker thread count")]
    threads: Option<u16>,
    #[arg(long, help = "Set the target CPU usage percentage within 1..=100")]
    cpu_percent: Option<u8>,
    #[arg(long, value_enum, help = "Set the scheduling priority profile")]
    priority: Option<CliPriority>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliBudgetMode {
    Conservative,
    Idle,
    Balanced,
    Aggressive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliPriority {
    Background,
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
    let socket_path = args.socket.clone().unwrap_or_else(default_socket_path);
    let timeout = Duration::from_secs(args.timeout_secs.max(1));
    let agent = WalletAgent::new(Some(socket_path.clone()), timeout);
    match args.command {
        AgentCliCommand::Setup { wallet_address } => {
            let summary = agent
                .setup(&wallet_address)
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_json_or_text(&summary, args.json, print_wallet_summary);
        }
        AgentCliCommand::SetWallet { wallet_address } => {
            let summary = agent
                .update_wallet(&wallet_address)
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_json_or_text(&summary, args.json, print_wallet_summary);
        }
        AgentCliCommand::Status => {
            let snapshot = agent
                .status()
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        AgentCliCommand::Capabilities => {
            let caps = agent
                .capabilities()
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_capabilities(caps, args.json);
        }
        AgentCliCommand::Methods => {
            let methods = agent
                .methods()
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string(&methods).expect("encode methods json")
                );
            } else {
                let methods: AgentMethods =
                    serde_json::from_value(methods).expect("decode methods json");
                print_methods(methods, false);
            }
        }
        AgentCliCommand::Start => {
            let snapshot = agent
                .start()
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        AgentCliCommand::Stop => {
            let snapshot = agent
                .stop()
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        AgentCliCommand::Pause => {
            let snapshot = agent
                .pause()
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        AgentCliCommand::Resume => {
            let snapshot = agent
                .resume()
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        AgentCliCommand::SetMode { mode } => {
            let snapshot = agent
                .set_mode(map_budget_mode(mode))
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        AgentCliCommand::SetBudget(command) => {
            if command.threads.is_none()
                && command.cpu_percent.is_none()
                && command.priority.is_none()
            {
                return Err(CliError::new(
                    2,
                    "set-budget requires at least one of --threads, --cpu-percent, or --priority",
                    args.json,
                ));
            }
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let snapshot: MinerSnapshot = client_call(
                &mut client,
                "budget.set",
                Some(json!({
                    "threads": command.threads,
                    "cpu_percent": command.cpu_percent,
                    "priority": command.priority.map(map_priority),
                })),
                timeout,
                args.json,
            )
            .await?;
            print_status(snapshot, args.json);
        }
        AgentCliCommand::EventsSince { since_seq } => {
            let response = agent
                .events_since(since_seq)
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_events_since(response, args.json);
        }
        AgentCliCommand::Events => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            client
                .subscribe_events(timeout)
                .await
                .map_err(|err| map_client_error(err, args.json))?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"subscribed": true}))
                        .expect("encode subscribe ack")
                );
                loop {
                    let message = client
                        .read_message(None)
                        .await
                        .map_err(|err| map_client_error(err, args.json))?;
                    println!(
                        "{}",
                        serde_json::to_string(&message).expect("encode event json")
                    );
                }
            } else {
                println!("subscribed");
                loop {
                    let event = client
                        .read_event(None)
                        .await
                        .map_err(|err| map_client_error(err, args.json))?;
                    println!("{}", format_event(&event));
                }
            }
        }
        AgentCliCommand::Doctor => {
            let report = agent
                .doctor()
                .await
                .map_err(|err| map_wallet_error(err, args.json))?;
            print_json_or_text(&report, args.json, print_doctor_report);
        }
        AgentCliCommand::McpConfig => {
            let config = agent
                .mcp_config()
                .map_err(|err| map_wallet_error(err, args.json))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&config).expect("encode mcp config")
            );
        }
        AgentCliCommand::Mcp => {
            run_mcp(agent)
                .await
                .map_err(|err| CliError::new(4, format!("mcp server failed: {err}"), args.json))?;
        }
        AgentCliCommand::Dashboard => {
            run_dashboard(agent)
                .await
                .map_err(|err| CliError::new(4, format!("dashboard failed: {err}"), args.json))?;
        }
    }
    Ok(())
}

async fn connect(
    socket_path: &PathBuf,
    timeout: Duration,
    json: bool,
) -> Result<AgentConnection, CliError> {
    AgentConnection::connect(socket_path, timeout)
        .await
        .map_err(|err| map_client_error(err, json))
}

async fn client_call<T: serde::de::DeserializeOwned>(
    client: &mut AgentConnection,
    method: &str,
    params: Option<serde_json::Value>,
    timeout: Duration,
    json: bool,
) -> Result<T, CliError> {
    client
        .call(method, params, timeout)
        .await
        .map_err(|err| map_client_error(err, json))
}

fn map_client_error(err: AgentClientError, json: bool) -> CliError {
    let exit_code = match err {
        AgentClientError::Connect { .. } => 3,
        AgentClientError::Timeout { .. } => 5,
        AgentClientError::Rpc(_) => 4,
        AgentClientError::Io(_) | AgentClientError::Parse(_) | AgentClientError::Protocol(_) => 4,
    };
    CliError::new(exit_code, err.to_string(), json)
}

fn map_wallet_error(err: WalletAgentError, json: bool) -> CliError {
    let exit_code = match err {
        WalletAgentError::NotConfigured => 4,
        WalletAgentError::InvalidWallet(_) => 2,
        WalletAgentError::Rpc(ref inner) => match inner {
            AgentClientError::Connect { .. } => 3,
            AgentClientError::Timeout { .. } => 5,
            AgentClientError::Rpc(_)
            | AgentClientError::Io(_)
            | AgentClientError::Parse(_)
            | AgentClientError::Protocol(_) => 4,
        },
        WalletAgentError::Io { .. }
        | WalletAgentError::StateParse(_)
        | WalletAgentError::Spawn(_)
        | WalletAgentError::DaemonBinaryNotFound(_)
        | WalletAgentError::DaemonExited
        | WalletAgentError::DaemonStartTimeout(_)
        | WalletAgentError::DaemonStopTimeout(_) => 4,
    };
    CliError::new(exit_code, err.to_string(), json)
}

fn map_budget_mode(value: CliBudgetMode) -> BudgetMode {
    match value {
        CliBudgetMode::Conservative => BudgetMode::Conservative,
        CliBudgetMode::Idle => BudgetMode::Idle,
        CliBudgetMode::Balanced => BudgetMode::Balanced,
        CliBudgetMode::Aggressive => BudgetMode::Aggressive,
    }
}

fn map_priority(value: CliPriority) -> Priority {
    match value {
        CliPriority::Background => Priority::Background,
    }
}
