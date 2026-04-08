#[path = "support/agent_process.rs"]
mod agent_process;
#[path = "support/fake_pool.rs"]
mod fake_pool;
#[path = "support/fake_reward_api.rs"]
mod fake_reward_api;
#[path = "support/process_cli.rs"]
mod process;

use agent_process::AgentProcess;
use anyhow::{Context, Result};
use fake_pool::SilentKeepalivePool;
use fake_reward_api::FakeRewardApi;
use process::{resolve_stc_mint_agent_bin, resolve_stc_mint_agentctl_bin, TEST_MUTEX};
use serde_json::Value;
use std::process::Command;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_wallet_set_show_doctor_and_mcp_config_work() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let state_path = process::temp_test_path("mint-state", "json");
    let socket_path = process::temp_test_path("mint-socket", "sock");
    let wallet = "0x33333333333333333333333333333333";

    let setup = run_ctl_with_env_json(
        &socket_path,
        &[(
            "STC_MINT_AGENT_STATE_PATH",
            state_path.to_string_lossy().as_ref(),
        )],
        &["wallet", "set", "--wallet-address", wallet],
    )
    .await?;
    assert_eq!(setup["wallet_address"], wallet);
    assert_eq!(setup["network"], "main");
    assert!(setup["worker_id"]
        .as_str()
        .unwrap_or_default()
        .starts_with("agent"));
    assert_eq!(setup["daemon_running"], false);

    let show = run_ctl_with_env_json(
        &socket_path,
        &[(
            "STC_MINT_AGENT_STATE_PATH",
            state_path.to_string_lossy().as_ref(),
        )],
        &["wallet", "show"],
    )
    .await?;
    assert_eq!(show["wallet_address"], wallet);
    assert_eq!(show["worker_id"], setup["worker_id"]);
    assert_eq!(show["network"], "main");

    let doctor = run_ctl_with_env_json(
        &socket_path,
        &[(
            "STC_MINT_AGENT_STATE_PATH",
            state_path.to_string_lossy().as_ref(),
        )],
        &["integrate", "doctor"],
    )
    .await?;
    assert_eq!(doctor["wallet_configured"], true);
    assert_eq!(doctor["wallet_address"], wallet);
    assert_eq!(doctor["requested_mode"], "auto");
    assert_eq!(doctor["daemon_running"], false);

    let mcp_config = run_ctl_with_env_json(
        &socket_path,
        &[(
            "STC_MINT_AGENT_STATE_PATH",
            state_path.to_string_lossy().as_ref(),
        )],
        &["integrate", "mcp-config"],
    )
    .await?;
    assert_eq!(
        mcp_config["mcpServers"]["stc-mint"]["args"],
        serde_json::json!(["integrate", "mcp"])
    );

    let _ = std::fs::remove_file(state_path);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_wallet_reward_reads_external_account_totals() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let reward_api = FakeRewardApi::start_json(serde_json::json!({
        "account": "0x33333333333333333333333333333333",
        "generated_at_millis": 123,
        "window_secs": 300,
        "online_threshold_secs": 120,
        "summary": {
            "active_workers": 1,
            "total_workers": 1,
            "hashrate_1m": 0.0,
            "hashrate_window": 0.0,
            "observed_hashrate_1m": 0.0,
            "observed_hashrate_window": 0.0,
            "assigned_hashrate_floor": 0.0,
            "accepted_shares_1m": 0,
            "accepted_shares_window": 0,
            "miner_valid_shares_1m": 0,
            "miner_valid_shares_window": 0,
            "pending_submits": 0,
            "confirmed_blocks_24h": 2,
            "orphaned_blocks_24h": 1,
            "confirmed_total": "1500000000",
            "paid_total": "200000000",
            "confirmed_through_height": 12345,
            "estimated_pending_total": "50000000",
            "last_share_at_millis": null
        },
        "workers": []
    }))
    .await?;
    let reward_api_base = reward_api.base_url();
    let halley_reward_api = FakeRewardApi::start_json(serde_json::json!({
        "account": "0x33333333333333333333333333333333",
        "generated_at_millis": 456,
        "window_secs": 300,
        "online_threshold_secs": 120,
        "summary": {
            "active_workers": 0,
            "total_workers": 0,
            "hashrate_1m": 0.0,
            "hashrate_window": 0.0,
            "observed_hashrate_1m": 0.0,
            "observed_hashrate_window": 0.0,
            "assigned_hashrate_floor": 0.0,
            "accepted_shares_1m": 0,
            "accepted_shares_window": 0,
            "miner_valid_shares_1m": 0,
            "miner_valid_shares_window": 0,
            "pending_submits": 0,
            "confirmed_blocks_24h": 3,
            "orphaned_blocks_24h": 0,
            "confirmed_total": "2500000000",
            "paid_total": "400000000",
            "confirmed_through_height": 23456,
            "estimated_pending_total": null,
            "last_share_at_millis": null
        },
        "workers": []
    }))
    .await?;
    let halley_reward_api_base = halley_reward_api.base_url();
    let state_path = process::temp_test_path("mint-state-reward", "json");
    let socket_path = process::temp_test_path("mint-socket-reward", "sock");

    let _setup = run_ctl_with_env_json(
        &socket_path,
        &[
            (
                "STC_MINT_AGENT_STATE_PATH",
                state_path.to_string_lossy().as_ref(),
            ),
            ("STC_MINT_AGENT_MAIN_REWARD_API", reward_api_base.as_str()),
            (
                "STC_MINT_AGENT_HALLEY_REWARD_API",
                halley_reward_api_base.as_str(),
            ),
        ],
        &[
            "wallet",
            "set",
            "--wallet-address",
            "0x33333333333333333333333333333333",
        ],
    )
    .await?;

    let reward = run_ctl_with_env_json(
        &socket_path,
        &[
            (
                "STC_MINT_AGENT_STATE_PATH",
                state_path.to_string_lossy().as_ref(),
            ),
            ("STC_MINT_AGENT_MAIN_REWARD_API", reward_api_base.as_str()),
            (
                "STC_MINT_AGENT_HALLEY_REWARD_API",
                halley_reward_api_base.as_str(),
            ),
        ],
        &["wallet", "reward"],
    )
    .await?;
    assert_eq!(reward["account"], "0x33333333333333333333333333333333");
    assert_eq!(reward["network"], "main");
    assert_eq!(reward["confirmed_total_raw"], "1500000000");
    assert_eq!(reward["confirmed_total_display"], "1.5 STC");
    assert_eq!(reward["estimated_pending_total_display"], "0.1 STC");
    assert_eq!(reward["paid_total_display"], "0.2 STC");
    assert_eq!(reward["confirmed_blocks_24h"], 2);
    assert_eq!(reward["orphaned_blocks_24h"], 1);
    assert_eq!(
        reward_api.last_request_path().as_deref(),
        Some("/v1/mining/dashboard/0x33333333333333333333333333333333?window_secs=300")
    );

    let _halley = run_ctl_with_env_json(
        &socket_path,
        &[
            (
                "STC_MINT_AGENT_STATE_PATH",
                state_path.to_string_lossy().as_ref(),
            ),
            ("STC_MINT_AGENT_MAIN_REWARD_API", reward_api_base.as_str()),
            (
                "STC_MINT_AGENT_HALLEY_REWARD_API",
                halley_reward_api_base.as_str(),
            ),
        ],
        &[
            "wallet",
            "set",
            "--wallet-address",
            "0x33333333333333333333333333333333",
            "--network",
            "halley",
        ],
    )
    .await?;
    let halley_reward = run_ctl_with_env_json(
        &socket_path,
        &[
            (
                "STC_MINT_AGENT_STATE_PATH",
                state_path.to_string_lossy().as_ref(),
            ),
            ("STC_MINT_AGENT_MAIN_REWARD_API", reward_api_base.as_str()),
            (
                "STC_MINT_AGENT_HALLEY_REWARD_API",
                halley_reward_api_base.as_str(),
            ),
        ],
        &["wallet", "reward"],
    )
    .await?;
    assert_eq!(halley_reward["network"], "halley");
    assert_eq!(halley_reward["confirmed_total_display"], "2.5 STC");
    assert_eq!(
        halley_reward_api.last_request_path().as_deref(),
        Some("/v1/mining/dashboard/0x33333333333333333333333333333333?window_secs=300")
    );

    let _ = std::fs::remove_file(state_path);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_help_shows_wallet_miner_integrate_and_auto_mode() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let ctl_bin = resolve_stc_mint_agentctl_bin()?;

    let top_help = Command::new(&ctl_bin)
        .arg("--help")
        .output()
        .context("run stc-mint-agentctl --help failed")?;
    assert!(top_help.status.success());
    let top_stdout = String::from_utf8(top_help.stdout).context("decode top help failed")?;
    assert!(top_stdout.contains("wallet"));
    assert!(top_stdout.contains("miner"));
    assert!(top_stdout.contains("integrate"));
    assert!(!top_stdout.contains("governor"));

    let wallet_set_help = Command::new(&ctl_bin)
        .args(["wallet", "set", "--help"])
        .output()
        .context("run wallet set --help failed")?;
    assert!(wallet_set_help.status.success());
    let wallet_set_stdout =
        String::from_utf8(wallet_set_help.stdout).context("decode wallet set help failed")?;
    assert!(wallet_set_stdout.contains("Payout wallet address"));
    assert!(wallet_set_stdout.contains("stable worker id"));
    assert!(wallet_set_stdout.contains("Defaults to main on first use"));

    let wallet_reward_help = Command::new(&ctl_bin)
        .args(["wallet", "reward", "--help"])
        .output()
        .context("run wallet reward --help failed")?;
    assert!(wallet_reward_help.status.success());
    let wallet_reward_stdout =
        String::from_utf8(wallet_reward_help.stdout).context("decode wallet reward help failed")?;
    assert!(wallet_reward_stdout.contains("external account query"));
    assert!(wallet_reward_stdout.contains("does not depend on the local miner daemon"));

    let set_mode_help = Command::new(&ctl_bin)
        .args(["miner", "set-mode", "--help"])
        .output()
        .context("run stc-mint-agentctl miner set-mode --help failed")?;
    assert!(set_mode_help.status.success());
    let set_mode_stdout =
        String::from_utf8(set_mode_help.stdout).context("decode set-mode help failed")?;
    assert!(
        set_mode_stdout.contains("possible values: auto, conservative, idle, balanced, aggressive")
    );
    assert!(set_mode_stdout.contains("daemon adjusts threads and cpu_percent"));
    assert!(set_mode_stdout.contains("conservative fixed preset"));

    let daemon_help = Command::new(resolve_stc_mint_agent_bin()?)
        .arg("--help")
        .output()
        .context("run stc-mint-agent --help failed")?;
    assert!(daemon_help.status.success());
    let daemon_stdout =
        String::from_utf8(daemon_help.stdout).context("decode stc-mint-agent help failed")?;
    assert!(daemon_stdout.contains("Internal daemon for stc-mint-agentctl"));
    assert!(daemon_stdout.contains("Use `stc-mint-agentctl wallet set`"));
    assert!(!daemon_stdout.contains("--login"));
    assert!(!daemon_stdout.contains("--pool"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_status_reports_requested_mode_effective_budget_and_auto_state() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "cryptonight", &[]).await?;

    let state_path = agent.state_path().to_string_lossy().to_string();
    let envs = [("STC_MINT_AGENT_STATE_PATH", state_path.as_str())];

    let status_text =
        run_ctl_with_env_text(agent.socket_path(), &envs, &["miner", "status"]).await?;
    assert!(status_text.contains("state: stopped"));
    assert!(status_text.contains("requested_mode: auto"));
    assert!(status_text.contains("auto_state: held"));
    assert!(status_text.contains("auto_hold_reason: not_running"));
    assert!(status_text.contains("effective_budget: threads=1 cpu_percent=50 priority=background"));

    let status_json =
        run_ctl_with_env_json(agent.socket_path(), &envs, &["miner", "status"]).await?;
    assert_eq!(status_json["requested_mode"], "auto");
    assert_eq!(status_json["auto_state"], "held");
    assert_eq!(status_json["auto_hold_reason"], "not_running");
    assert_eq!(status_json["effective_budget"]["threads"], 1);
    assert_eq!(status_json["effective_budget"]["cpu_percent"], 50);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_lifecycle_and_set_mode_work() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "keccak", &[]).await?;

    let state_path = agent.state_path().to_string_lossy().to_string();
    let envs = [("STC_MINT_AGENT_STATE_PATH", state_path.as_str())];

    let started = run_ctl_with_env_json(agent.socket_path(), &envs, &["miner", "start"]).await?;
    assert!(matches!(
        started["state"].as_str(),
        Some("starting" | "running")
    ));

    let paused = run_ctl_with_env_json(agent.socket_path(), &envs, &["miner", "pause"]).await?;
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["requested_mode"], "auto");
    assert_eq!(paused["auto_state"], "held");
    assert_eq!(paused["auto_hold_reason"], "manual_pause");

    let resumed = run_ctl_with_env_json(agent.socket_path(), &envs, &["miner", "resume"]).await?;
    assert!(matches!(
        resumed["state"].as_str(),
        Some("starting" | "running")
    ));
    assert_eq!(resumed["requested_mode"], "auto");

    let manual = run_ctl_with_env_json(
        agent.socket_path(),
        &envs,
        &["miner", "set-mode", "conservative"],
    )
    .await?;
    assert_eq!(manual["requested_mode"], "conservative");
    assert_eq!(manual["auto_state"], "inactive");
    assert_eq!(manual["effective_budget"]["threads"], 1);
    assert_eq!(manual["effective_budget"]["cpu_percent"], 50);

    let back_to_auto =
        run_ctl_with_env_json(agent.socket_path(), &envs, &["miner", "set-mode", "auto"]).await?;
    assert_eq!(back_to_auto["requested_mode"], "auto");
    assert!(matches!(
        back_to_auto["auto_state"].as_str(),
        Some("active" | "held")
    ));

    let stopped = run_ctl_with_env_json(agent.socket_path(), &envs, &["miner", "stop"]).await?;
    assert_eq!(stopped["state"], "stopped");
    assert_eq!(stopped["requested_mode"], "auto");
    assert_eq!(stopped["auto_state"], "held");
    assert_eq!(stopped["auto_hold_reason"], "manual_stop");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_cli_rejects_invalid_arguments() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let state_path = process::temp_test_path("mint-state-invalid", "json");
    let socket_path = process::temp_test_path("mint-socket-invalid", "sock");

    let invalid_wallet = run_ctl_with_env(
        &socket_path,
        &[(
            "STC_MINT_AGENT_STATE_PATH",
            state_path.to_string_lossy().as_ref(),
        )],
        &["wallet", "set", "--wallet-address", ""],
        false,
    )
    .await?;
    assert_eq!(invalid_wallet.status.code(), Some(2));

    let invalid_mode = run_ctl(&socket_path, &["miner", "set-mode", "paused"], false).await?;
    assert_eq!(invalid_mode.status.code(), Some(2));
    let invalid_mode_stderr = String::from_utf8_lossy(&invalid_mode.stderr);
    assert!(invalid_mode_stderr.contains("invalid value 'paused'"));
    assert!(invalid_mode_stderr
        .contains("possible values: auto, conservative, idle, balanced, aggressive"));

    let watch_json = run_ctl(&socket_path, &["miner", "watch"], true).await?;
    assert_eq!(watch_json.status.code(), Some(2));
    let watch_json_stdout = String::from_utf8_lossy(&watch_json.stdout);
    assert!(watch_json_stdout.contains("--json is not supported with `miner watch`"));
    Ok(())
}

async fn run_ctl_with_env_text(
    socket_path: &std::path::Path,
    envs: &[(&str, &str)],
    args: &[&str],
) -> Result<String> {
    let output = run_ctl_with_env(socket_path, envs, args, false).await?;
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
