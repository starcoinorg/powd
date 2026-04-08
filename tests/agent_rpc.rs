#[path = "support/agent_process.rs"]
mod agent_process;
#[path = "support/fake_pool.rs"]
mod fake_pool;
#[path = "support/process_rpc.rs"]
mod process;

use agent_process::AgentProcess;
use anyhow::{Context, Result};
use fake_pool::SilentKeepalivePool;
use process::{pick_free_port, TEST_MUTEX};
use serde_json::{json, Value};
use starcoin_cpu_miner::agent::AgentConnection as RpcClient;
use std::time::{Duration, Instant};

const RPC_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_rpc_reports_capabilities_methods_and_initial_auto_status() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = format!("127.0.0.1:{}", pick_free_port()?);
    let agent = AgentProcess::spawn(&pool, "cryptonight", &[]).await?;
    let _ = agent.state_path();
    let mut rpc = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;

    let caps = rpc
        .call_value("status.capabilities", None, RPC_TIMEOUT)
        .await?;
    assert!(caps["max_threads"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(caps["supports_cpu_percent"], true);
    assert_eq!(caps["supported_priorities"], json!(["background"]));
    assert_eq!(
        caps["supported_modes"],
        json!(["auto", "conservative", "idle", "balanced", "aggressive"])
    );

    let methods = rpc.call_value("status.methods", None, RPC_TIMEOUT).await?;
    assert_eq!(methods["agent_api_version"], 1);
    assert_eq!(
        methods["methods"]["miner.set_mode"]["result"],
        "miner_snapshot"
    );
    assert_eq!(
        methods["methods"]["miner.set_mode"]["params"]["fields"]["mode"]["enum_values"],
        json!(["auto", "conservative", "idle", "balanced", "aggressive"])
    );
    assert_eq!(
        methods["methods"]["daemon.configure"]["params"]["fields"]["network"]["enum_values"],
        json!(["main", "halley"])
    );
    assert!(methods["methods"].get("budget.set").is_none());
    assert!(methods["methods"].get("governor.enable").is_none());

    let status = rpc.call_value("status.get", None, RPC_TIMEOUT).await?;
    assert_eq!(status["state"], "stopped");
    assert_eq!(status["requested_mode"], "auto");
    assert_eq!(status["auto_state"], "held");
    assert_eq!(status["auto_hold_reason"], "not_running");
    assert_eq!(status["effective_budget"]["threads"], 1);
    assert_eq!(status["effective_budget"]["cpu_percent"], 50);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_rpc_lifecycle_mode_and_events_work() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "keccak", &[]).await?;
    let mut ctl = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;
    let mut events = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;

    let subscribed = events.subscribe_events(RPC_TIMEOUT).await?;
    assert_eq!(subscribed["subscribed"], true);

    let start = ctl.call_value("miner.start", None, RPC_TIMEOUT).await?;
    assert!(matches!(
        start["state"].as_str(),
        Some("starting" | "running")
    ));
    assert_eq!(
        wait_for_event_type(&mut events, "started").await?["params"]["type"],
        "started"
    );
    let running = wait_for_state(&mut ctl, "running").await?;
    assert_eq!(running["requested_mode"], "auto");
    assert_eq!(running["auto_state"], "active");

    let paused = ctl.call_value("miner.pause", None, RPC_TIMEOUT).await?;
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["requested_mode"], "auto");
    assert_eq!(paused["auto_state"], "held");
    assert_eq!(paused["auto_hold_reason"], "manual_pause");
    assert_eq!(
        wait_for_event_type(&mut events, "paused").await?["params"]["type"],
        "paused"
    );

    let resumed = ctl.call_value("miner.resume", None, RPC_TIMEOUT).await?;
    assert!(matches!(
        resumed["state"].as_str(),
        Some("starting" | "running")
    ));
    let running_again = wait_for_state(&mut ctl, "running").await?;
    assert_eq!(running_again["requested_mode"], "auto");
    assert_eq!(running_again["auto_state"], "active");

    let conservative = ctl
        .call_value(
            "miner.set_mode",
            Some(json!({"mode": "conservative"})),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(conservative["requested_mode"], "conservative");
    assert_eq!(conservative["auto_state"], "inactive");
    assert_eq!(conservative["effective_budget"]["threads"], 1);
    assert_eq!(conservative["effective_budget"]["cpu_percent"], 50);

    let auto = ctl
        .call_value("miner.set_mode", Some(json!({"mode": "auto"})), RPC_TIMEOUT)
        .await?;
    assert_eq!(auto["requested_mode"], "auto");
    assert!(matches!(
        auto["auto_state"].as_str(),
        Some("active" | "held")
    ));

    let trend = ctl.call_value("status.get", None, RPC_TIMEOUT).await?;
    assert!(trend["hashrate_5m"].as_f64().unwrap_or(0.0) >= 0.0);

    let baseline = ctl
        .call_value("events.since", Some(json!({"since_seq": 0})), RPC_TIMEOUT)
        .await?;
    assert!(baseline["next_seq"].as_u64().unwrap_or(0) >= 1);

    let stopped = ctl.call_value("miner.stop", None, RPC_TIMEOUT).await?;
    assert_eq!(stopped["state"], "stopped");
    assert_eq!(stopped["requested_mode"], "auto");
    assert_eq!(stopped["auto_state"], "held");
    assert_eq!(stopped["auto_hold_reason"], "manual_stop");
    assert_eq!(
        wait_for_event_type(&mut events, "stopped").await?["params"]["type"],
        "stopped"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_rpc_rejects_invalid_requests() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = format!("127.0.0.1:{}", pick_free_port()?);
    let agent = AgentProcess::spawn(&pool, "cryptonight", &[]).await?;
    let mut rpc = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;

    let invalid_request = rpc
        .raw(
            json!({"jsonrpc": "1.0", "id": 1, "method": "status.get"}),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(invalid_request["error"]["code"], -32600);

    let method_not_found = rpc
        .raw(
            json!({"jsonrpc": "2.0", "id": 2, "method": "status.nope"}),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(method_not_found["error"]["code"], -32601);

    let invalid_mode = rpc
        .raw(
            json!({"jsonrpc": "2.0", "id": 3, "method": "miner.set_mode", "params": {"mode": "bogus"}}),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(invalid_mode["error"]["code"], -32602);

    let no_budget_method = rpc
        .raw(
            json!({"jsonrpc": "2.0", "id": 4, "method": "budget.set", "params": {"threads": 2}}),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(no_budget_method["error"]["code"], -32601);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_rpc_events_stream_allows_follow_up_requests_on_same_connection() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = format!("127.0.0.1:{}", pick_free_port()?);
    let agent = AgentProcess::spawn(&pool, "cryptonight", &[]).await?;
    let mut rpc = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;

    let subscribed = rpc.subscribe_events(RPC_TIMEOUT).await?;
    assert_eq!(subscribed["subscribed"], true);

    let status = rpc.call_value("status.get", None, RPC_TIMEOUT).await?;
    assert_eq!(status["state"], "stopped");
    Ok(())
}

async fn wait_for_event_type(events: &mut RpcClient, expected: &str) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let event = tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            events.read_message(None),
        )
        .await
        .context("wait event timeout")??;
        if event["method"] == "event" && event["params"]["type"] == expected {
            return Ok(event);
        }
    }
}

async fn wait_for_state(ctl: &mut RpcClient, expected: &str) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = ctl.call_value("status.get", None, RPC_TIMEOUT).await?;
        if status["state"] == expected {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(anyhow::anyhow!("wait state timeout: {expected}"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
