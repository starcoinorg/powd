mod support;

use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;
use std::time::Duration;
use support::agent_rpc::{AgentProcess, RpcClient};
use support::fake_pool::SilentKeepalivePool;
use support::process::{resolve_stc_mint_agentctl_bin, TEST_MUTEX};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

const RPC_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_supports_status_and_capabilities() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "cryptonight", &[]).await?;

    let capabilities = run_ctl_json(agent.socket_path(), &["capabilities"]).await?;
    assert!(capabilities["max_threads"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(
        capabilities["supported_modes"],
        serde_json::json!(["conservative", "idle", "balanced", "aggressive"])
    );

    let status_text = run_ctl_text(agent.socket_path(), &["status"]).await?;
    assert!(status_text.contains("state: stopped"));
    assert!(status_text.contains("worker_name: agent"));

    let status_json = run_ctl_json(agent.socket_path(), &["status"]).await?;
    assert_eq!(status_json["current_budget"]["threads"], 1);
    assert_eq!(status_json["current_budget"]["cpu_percent"], 50);
    assert_eq!(status_json["current_budget"]["priority"], "background");

    let methods = run_ctl_json(agent.socket_path(), &["methods"]).await?;
    assert_eq!(methods["agent_api_version"], 1);
    assert_eq!(
        methods["methods"]["budget.set_mode"]["params"]["fields"]["mode"]["enum_values"],
        serde_json::json!(["conservative", "idle", "balanced", "aggressive"])
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_supports_wallet_setup_doctor_and_mcp_config() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let state_path = support::process::temp_test_path("mint-state", "json");
    let socket_path = support::process::temp_test_path("mint-socket", "sock");
    let wallet = "0x33333333333333333333333333333333";

    let setup = run_ctl_with_env_json(
        &socket_path,
        &[(
            "STC_MINT_AGENT_STATE_PATH",
            state_path.to_string_lossy().as_ref(),
        )],
        &["setup", "--wallet-address", wallet],
    )
    .await?;
    assert_eq!(setup["wallet_address"], wallet);
    assert!(setup["worker_id"]
        .as_str()
        .unwrap_or_default()
        .starts_with("agent"));
    assert_eq!(setup["daemon_running"], false);

    let doctor = run_ctl_with_env_json(
        &socket_path,
        &[(
            "STC_MINT_AGENT_STATE_PATH",
            state_path.to_string_lossy().as_ref(),
        )],
        &["doctor"],
    )
    .await?;
    assert_eq!(doctor["wallet_configured"], true);
    assert_eq!(doctor["wallet_address"], wallet);
    assert_eq!(doctor["daemon_running"], false);

    let mcp_config = run_ctl_with_env_json(
        &socket_path,
        &[(
            "STC_MINT_AGENT_STATE_PATH",
            state_path.to_string_lossy().as_ref(),
        )],
        &["mcp-config"],
    )
    .await?;
    assert_eq!(
        mcp_config["mcpServers"]["stc-mint"]["args"],
        serde_json::json!(["mcp"])
    );

    let _ = std::fs::remove_file(state_path);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_help_shows_mode_mapping_and_daemon_default_budget() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let ctl_bin = resolve_stc_mint_agentctl_bin()?;
    let set_mode_help = Command::new(&ctl_bin)
        .args(["set-mode", "--help"])
        .output()
        .context("run stc-mint-agentctl set-mode --help failed")?;
    assert!(set_mode_help.status.success());
    let set_mode_stdout =
        String::from_utf8(set_mode_help.stdout).context("decode set-mode help failed")?;
    assert!(set_mode_stdout.contains("conservative threads=1, cpu_percent=50, priority=background"));
    assert!(set_mode_stdout.contains(
        "idle         threads=ceil(logical_cpus/4), cpu_percent=15, priority=background"
    ));
    assert!(set_mode_stdout.contains(
        "balanced     threads=ceil(logical_cpus/2), cpu_percent=40, priority=background"
    ));
    assert!(set_mode_stdout.contains(
        "aggressive   threads=ceil(logical_cpus/2), cpu_percent=80, priority=background"
    ));

    let agent_bin = support::process::resolve_stc_mint_agent_bin()?;
    let daemon_help = Command::new(agent_bin)
        .arg("--help")
        .output()
        .context("run stc-mint-agent --help failed")?;
    assert!(daemon_help.status.success());
    let daemon_stdout =
        String::from_utf8(daemon_help.stdout).context("decode stc-mint-agent help failed")?;
    assert!(daemon_stdout.contains("Initial mode when the daemon starts: conservative"));
    assert!(daemon_stdout.contains("conservative = threads=1, cpu_percent=50, priority=background"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_lifecycle_and_budget_commands_work() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "keccak", &[]).await?;

    let started = run_ctl_json(agent.socket_path(), &["start"]).await?;
    assert_eq!(started["state"], "starting");

    let paused = run_ctl_json(agent.socket_path(), &["pause"]).await?;
    assert_eq!(paused["state"], "paused");

    let resumed = run_ctl_json(agent.socket_path(), &["resume"]).await?;
    assert_eq!(resumed["state"], "running");

    let mode = run_ctl_json(agent.socket_path(), &["set-mode", "conservative"]).await?;
    assert_eq!(mode["current_budget"]["threads"], 1);
    assert_eq!(mode["current_budget"]["cpu_percent"], 50);

    let budget = run_ctl_json(
        agent.socket_path(),
        &[
            "set-budget",
            "--threads",
            "2",
            "--cpu-percent",
            "25",
            "--priority",
            "background",
        ],
    )
    .await?;
    assert_eq!(budget["current_budget"]["threads"], 2);
    assert_eq!(budget["current_budget"]["cpu_percent"], 25);

    let stopped = run_ctl_json(agent.socket_path(), &["stop"]).await?;
    assert_eq!(stopped["state"], "stopped");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_rejects_invalid_arguments() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "cryptonight", &[]).await?;

    let empty_budget = run_ctl(agent.socket_path(), &["set-budget"], false).await?;
    assert_eq!(empty_budget.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&empty_budget.stderr).contains("requires at least one"));

    let paused_mode = run_ctl(agent.socket_path(), &["set-mode", "paused"], false).await?;
    assert_eq!(paused_mode.status.code(), Some(2));
    let paused_stderr = String::from_utf8_lossy(&paused_mode.stderr);
    assert!(paused_stderr.contains("invalid value 'paused'"));
    assert!(paused_stderr.contains("possible values: conservative, idle, balanced, aggressive"));

    let invalid_priority = run_ctl(
        agent.socket_path(),
        &["set-budget", "--priority", "normal"],
        false,
    )
    .await?;
    assert_eq!(invalid_priority.status.code(), Some(2));
    let invalid_priority_stderr = String::from_utf8_lossy(&invalid_priority.stderr);
    assert!(invalid_priority_stderr.contains("invalid value 'normal'"));
    assert!(invalid_priority_stderr.contains("possible values: background"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_streams_events() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "keccak", &[]).await?;
    let ctl_bin = resolve_stc_mint_agentctl_bin()?;
    let mut child = TokioCommand::new(ctl_bin)
        .arg("--socket")
        .arg(agent.socket_path())
        .arg("--json")
        .arg("events")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("spawn stc-mint-agentctl events failed")?;

    let stdout = child.stdout.take().context("take events stdout failed")?;
    let mut lines = BufReader::new(stdout).lines();
    let subscribed = tokio::time::timeout(RPC_TIMEOUT, lines.next_line())
        .await
        .context("wait subscribed line timeout")??
        .context("events cli closed before subscribed")?;
    let subscribed: Value = serde_json::from_str(&subscribed)?;
    assert_eq!(subscribed["subscribed"], true);

    let mut rpc = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;
    let _ = rpc.call_value("miner.start", None, RPC_TIMEOUT).await?;

    let event_line = tokio::time::timeout(RPC_TIMEOUT, lines.next_line())
        .await
        .context("wait event line timeout")??
        .context("events cli closed before event")?;
    let event: Value = serde_json::from_str(&event_line)?;
    assert_eq!(event["method"], "event");
    assert_eq!(event["params"]["type"], "started");

    let _ = child.kill().await;
    let _ = child.wait().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_reads_events_since() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "keccak", &[]).await?;

    let mut rpc = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;
    let baseline = rpc.events_since(0, RPC_TIMEOUT).await?;
    let _ = rpc.call_value("miner.start", None, RPC_TIMEOUT).await?;

    let response =
        wait_for_cli_events_since(agent.socket_path(), baseline.next_seq.saturating_sub(1)).await?;
    let events = response["events"]
        .as_array()
        .context("events must be array")?;
    assert!(!events.is_empty());
    assert_eq!(events[0]["event"]["type"], "started");
    Ok(())
}

async fn run_ctl_json(socket_path: &std::path::Path, args: &[&str]) -> Result<Value> {
    let output = run_ctl(socket_path, args, true).await?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "ctl failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }
    let stdout = String::from_utf8(output.stdout).context("decode ctl stdout failed")?;
    serde_json::from_str(stdout.trim()).context("parse ctl json output failed")
}

async fn run_ctl_text(socket_path: &std::path::Path, args: &[&str]) -> Result<String> {
    let output = run_ctl(socket_path, args, false).await?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "ctl failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }
    String::from_utf8(output.stdout).context("decode ctl stdout failed")
}

async fn run_ctl(
    socket_path: &std::path::Path,
    args: &[&str],
    json: bool,
) -> Result<std::process::Output> {
    run_ctl_with_env(socket_path, &[], args, json).await
}

async fn run_ctl_with_env_json(
    socket_path: &std::path::Path,
    envs: &[(&str, &str)],
    args: &[&str],
) -> Result<Value> {
    let output = run_ctl_with_env(socket_path, envs, args, true).await?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "ctl failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }
    let stdout = String::from_utf8(output.stdout).context("decode ctl stdout failed")?;
    serde_json::from_str(stdout.trim()).context("parse ctl json output failed")
}

async fn run_ctl_with_env(
    socket_path: &std::path::Path,
    envs: &[(&str, &str)],
    args: &[&str],
    json: bool,
) -> Result<std::process::Output> {
    let ctl_bin = resolve_stc_mint_agentctl_bin()?;
    let mut command = Command::new(ctl_bin);
    command.arg("--socket").arg(socket_path);
    for (key, value) in envs {
        command.env(key, value);
    }
    if json {
        command.arg("--json");
    }
    let output = command
        .args(args)
        .output()
        .context("run stc-mint-agentctl failed")?;
    Ok(output)
}

async fn wait_for_cli_events_since(socket_path: &std::path::Path, since_seq: u64) -> Result<Value> {
    let deadline = std::time::Instant::now() + RPC_TIMEOUT;
    loop {
        let response = run_ctl_json(
            socket_path,
            &["events-since", "--since-seq", &since_seq.to_string()],
        )
        .await?;
        if response["events"]
            .as_array()
            .is_some_and(|events| !events.is_empty())
        {
            return Ok(response);
        }
        if std::time::Instant::now() >= deadline {
            return Err(anyhow::anyhow!("wait cli events-since timeout"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
