use super::{default_socket_path, ControlClientError, ControlConnection};
use crate::{
    BudgetMode, ControlPlaneMethods, EventsSinceResponse, MinerCapabilities, MinerEvent,
    MinerSnapshot, Priority,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "stc-mint-agentctl",
    about = "Control a local stc-mint-agent daemon over its Unix socket",
    after_help = "Examples:\n  stc-mint-agentctl --json status\n  stc-mint-agentctl start\n  stc-mint-agentctl set-mode conservative\n  stc-mint-agentctl set-budget --threads 4 --cpu-percent 30 --priority background\n  stc-mint-agentctl --json events-since --since-seq 0"
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
    #[command(about = "Show the current miner snapshot")]
    Status,
    #[command(about = "Show runtime capabilities such as supported modes and limits")]
    Capabilities,
    #[command(about = "Show self-describing control-plane methods and parameter schema")]
    Methods,
    #[command(about = "Start mining with the daemon's static pool/login configuration")]
    Start,
    #[command(about = "Stop mining and disconnect from the pool")]
    Stop,
    #[command(about = "Pause solving while keeping config and connection state")]
    Pause,
    #[command(about = "Resume solving if the miner is paused")]
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
    match args.command {
        ControlCliCommand::Status => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let snapshot: MinerSnapshot =
                client_call(&mut client, "status.get", None, timeout, args.json).await?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::Capabilities => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let caps: MinerCapabilities = client
                .capabilities(timeout)
                .await
                .map_err(|err| map_client_error(err, args.json))?;
            print_capabilities(caps, args.json);
        }
        ControlCliCommand::Methods => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let methods: ControlPlaneMethods = client
                .methods(timeout)
                .await
                .map_err(|err| map_client_error(err, args.json))?;
            print_methods(methods, args.json);
        }
        ControlCliCommand::Start => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let snapshot: MinerSnapshot =
                client_call(&mut client, "miner.start", None, timeout, args.json).await?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::Stop => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let snapshot: MinerSnapshot =
                client_call(&mut client, "miner.stop", None, timeout, args.json).await?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::Pause => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let snapshot: MinerSnapshot =
                client_call(&mut client, "miner.pause", None, timeout, args.json).await?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::Resume => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let snapshot: MinerSnapshot =
                client_call(&mut client, "miner.resume", None, timeout, args.json).await?;
            print_status(snapshot, args.json);
        }
        ControlCliCommand::SetMode { mode } => {
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let snapshot: MinerSnapshot = client_call(
                &mut client,
                "budget.set_mode",
                Some(json!({ "mode": map_budget_mode(mode) })),
                timeout,
                args.json,
            )
            .await?;
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
            let mut client = connect(&socket_path, timeout, args.json).await?;
            let response: EventsSinceResponse = client
                .events_since(since_seq, timeout)
                .await
                .map_err(|err| map_client_error(err, args.json))?;
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

fn print_status(snapshot: MinerSnapshot, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&snapshot).expect("encode snapshot json")
        );
        return;
    }
    println!("state: {}", serde_name(&snapshot.state));
    println!("connected: {}", snapshot.connected);
    println!("pool: {}", snapshot.pool);
    println!("worker_name: {}", snapshot.worker_name);
    println!("hashrate: {:.2} H/s", snapshot.hashrate);
    println!("hashrate_5m: {:.2} H/s", snapshot.hashrate_5m);
    println!("accepted: {}", snapshot.accepted);
    println!("accepted_5m: {}", snapshot.accepted_5m);
    println!("rejected: {}", snapshot.rejected);
    println!("rejected_5m: {}", snapshot.rejected_5m);
    println!("submitted: {}", snapshot.submitted);
    println!("submitted_5m: {}", snapshot.submitted_5m);
    println!("reject_rate_5m: {:.4}", snapshot.reject_rate_5m);
    println!("reconnects: {}", snapshot.reconnects);
    println!("uptime_secs: {}", snapshot.uptime_secs);
    println!(
        "budget: threads={} cpu_percent={} priority={}",
        snapshot.current_budget.threads,
        snapshot.current_budget.cpu_percent,
        serde_name(&snapshot.current_budget.priority)
    );
    println!(
        "last_error: {}",
        snapshot.last_error.unwrap_or_else(|| "-".to_string())
    );
}

fn print_capabilities(caps: MinerCapabilities, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&caps).expect("encode capabilities json")
        );
        return;
    }
    println!("max_threads: {}", caps.max_threads);
    println!(
        "supported_modes: {}",
        caps.supported_modes
            .iter()
            .map(serde_name)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "supported_priorities: {}",
        caps.supported_priorities
            .iter()
            .map(serde_name)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("supports_cpu_percent: {}", caps.supports_cpu_percent);
    println!("supports_priority: {}", caps.supports_priority);
}

fn print_methods(methods: ControlPlaneMethods, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&methods).expect("encode methods json")
        );
        return;
    }
    println!("control_plane_version: {}", methods.control_plane_version);
    println!("agent_version: {}", methods.agent_version);
    for (name, method) in methods.methods {
        println!("{name}:");
        match method.params {
            Some(params) => {
                for (field, schema) in params.fields {
                    let mut line = format!(
                        "  param {}: {}{}",
                        field,
                        schema.type_name,
                        if schema.optional { "?" } else { "" }
                    );
                    if !schema.enum_values.is_empty() {
                        line.push_str(&format!(" enum={:?}", schema.enum_values));
                    }
                    println!("{line}");
                }
            }
            None => println!("  params: none"),
        }
        println!("  result: {}", method.result);
    }
}

fn print_events_since(response: EventsSinceResponse, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&response).expect("encode events since json")
        );
        return;
    }
    println!("next_seq: {}", response.next_seq);
    for envelope in response.events {
        println!("#{} {}", envelope.seq, format_event(&envelope.event));
    }
}

fn format_event(event: &MinerEvent) -> String {
    match event {
        MinerEvent::Started { snapshot }
        | MinerEvent::Paused { snapshot }
        | MinerEvent::Resumed { snapshot }
        | MinerEvent::Stopped { snapshot }
        | MinerEvent::Reconnecting { snapshot }
        | MinerEvent::BudgetChanged { snapshot }
        | MinerEvent::ShareAccepted { snapshot } => format!(
            "{} state={} connected={} accepted={} rejected={} hashrate={:.2}",
            event_type(event),
            serde_name(&snapshot.state),
            snapshot.connected,
            snapshot.accepted,
            snapshot.rejected,
            snapshot.hashrate
        ),
        MinerEvent::ShareRejected { snapshot, reason } => format!(
            "{} state={} accepted={} rejected={} reason={}",
            event_type(event),
            serde_name(&snapshot.state),
            snapshot.accepted,
            snapshot.rejected,
            reason
        ),
        MinerEvent::Error { snapshot, message } => format!(
            "{} state={} message={}",
            event_type(event),
            serde_name(&snapshot.state),
            message
        ),
    }
}

fn event_type(event: &MinerEvent) -> &'static str {
    match event {
        MinerEvent::Started { .. } => "started",
        MinerEvent::Paused { .. } => "paused",
        MinerEvent::Resumed { .. } => "resumed",
        MinerEvent::Stopped { .. } => "stopped",
        MinerEvent::Reconnecting { .. } => "reconnecting",
        MinerEvent::BudgetChanged { .. } => "budget_changed",
        MinerEvent::ShareAccepted { .. } => "share_accepted",
        MinerEvent::ShareRejected { .. } => "share_rejected",
        MinerEvent::Error { .. } => "error",
    }
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

fn serde_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .expect("encode serde name")
        .trim_matches('"')
        .to_string()
}
