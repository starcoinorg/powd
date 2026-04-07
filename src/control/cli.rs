use super::app::{AppError, MintApp};
use super::cli_output::{
    format_event, print_capabilities, print_doctor_report, print_events_since, print_json_or_text,
    print_methods, print_status, print_wallet_summary,
};
use super::dashboard::run_dashboard;
use super::mcp::run_mcp;
use super::{default_socket_path, ControlClientError, ControlConnection};
use crate::{BudgetMode, ControlPlaneMethods, MinerSnapshot, Priority};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "stc-mint-agentctl",
    about = "Control a local stc-mint-agent daemon over its Unix socket",
    after_help = "Examples:\n  stc-mint-agentctl setup --wallet-address 0xabc...\n  stc-mint-agentctl --json status\n  stc-mint-agentctl set-mode conservative\n  stc-mint-agentctl doctor\n  stc-mint-agentctl mcp-config"
)]
pub struct ControlCliArgs {
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
    command: ControlCliCommand,
}

#[derive(Subcommand, Debug)]
enum ControlCliCommand {
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
    #[command(about = "Show self-describing control-plane methods and parameter schema")]
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
    #[command(about = "Open a local TUI dashboard for status and basic control")]
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

pub async fn run_cli(args: ControlCliArgs) -> ExitCode {
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

async fn execute(args: ControlCliArgs) -> Result<(), CliError> {
    let socket_path = args.socket.clone().unwrap_or_else(default_socket_path);
    let timeout = Duration::from_secs(args.timeout_secs.max(1));
    let app = MintApp::new(Some(socket_path.clone()), timeout);
    match args.command {
        ControlCliCommand::Setup { wallet_address } => {
            let summary = app
                .setup(&wallet_address)
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_json_or_text(&summary, args.json, print_wallet_summary);
        }
        ControlCliCommand::SetWallet { wallet_address } => {
            let summary = app
                .update_wallet(&wallet_address)
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_json_or_text(&summary, args.json, print_wallet_summary);
        }
        ControlCliCommand::Status => {
            let snapshot = app
                .status()
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::Capabilities => {
            let caps = app
                .capabilities()
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_capabilities(caps, args.json);
        }
        ControlCliCommand::Methods => {
            let methods = app
                .methods()
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string(&methods).expect("encode methods json")
                );
            } else {
                let methods: ControlPlaneMethods =
                    serde_json::from_value(methods).expect("decode methods json");
                print_methods(methods, false);
            }
        }
        ControlCliCommand::Start => {
            let snapshot = app
                .start()
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::Stop => {
            let snapshot = app
                .stop()
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::Pause => {
            let snapshot = app
                .pause()
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::Resume => {
            let snapshot = app
                .resume()
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::SetMode { mode } => {
            let snapshot = app
                .set_mode(map_budget_mode(mode))
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::SetBudget(command) => {
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
        ControlCliCommand::EventsSince { since_seq } => {
            let response = app
                .events_since(since_seq)
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_events_since(response, args.json);
        }
        ControlCliCommand::Events => {
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
        ControlCliCommand::Doctor => {
            let report = app
                .doctor()
                .await
                .map_err(|err| map_app_error(err, args.json))?;
            print_json_or_text(&report, args.json, print_doctor_report);
        }
        ControlCliCommand::McpConfig => {
            let config = app
                .mcp_config()
                .map_err(|err| map_app_error(err, args.json))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&config).expect("encode mcp config")
            );
        }
        ControlCliCommand::Mcp => {
            run_mcp(app)
                .await
                .map_err(|err| CliError::new(4, format!("mcp server failed: {err}"), args.json))?;
        }
        ControlCliCommand::Dashboard => {
            run_dashboard(app)
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
) -> Result<ControlConnection, CliError> {
    ControlConnection::connect(socket_path, timeout)
        .await
        .map_err(|err| map_client_error(err, json))
}

async fn client_call<T: serde::de::DeserializeOwned>(
    client: &mut ControlConnection,
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

fn map_client_error(err: ControlClientError, json: bool) -> CliError {
    let exit_code = match err {
        ControlClientError::Connect { .. } => 3,
        ControlClientError::Timeout { .. } => 5,
        ControlClientError::Rpc(_) => 4,
        ControlClientError::Io(_)
        | ControlClientError::Parse(_)
        | ControlClientError::Protocol(_) => 4,
    };
    CliError::new(exit_code, err.to_string(), json)
}

fn map_app_error(err: AppError, json: bool) -> CliError {
    let exit_code = match err {
        AppError::NotConfigured => 4,
        AppError::InvalidWallet(_) => 2,
        AppError::Control(ref inner) => match inner {
            ControlClientError::Connect { .. } => 3,
            ControlClientError::Timeout { .. } => 5,
            ControlClientError::Rpc(_)
            | ControlClientError::Io(_)
            | ControlClientError::Parse(_)
            | ControlClientError::Protocol(_) => 4,
        },
        AppError::Io { .. }
        | AppError::StateParse(_)
        | AppError::Spawn(_)
        | AppError::DaemonBinaryNotFound(_)
        | AppError::DaemonExited
        | AppError::DaemonStartTimeout(_)
        | AppError::DaemonStopTimeout(_) => 4,
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
