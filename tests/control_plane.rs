mod support;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use support::control_plane::{AgentProcess, RpcClient};
use support::fake_pool::SilentKeepalivePool;
use support::process::{pick_free_port, TEST_MUTEX};

const RPC_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_plane_reports_capabilities_and_updates_budget() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = format!("127.0.0.1:{}", pick_free_port()?);
    let agent = AgentProcess::spawn(&pool, "cryptonight", &[]).await?;
    let mut rpc = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;

    let caps = rpc
        .call_value("status.capabilities", None, RPC_TIMEOUT)
        .await?;
    assert!(caps["max_threads"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(caps["supports_cpu_percent"].as_bool(), Some(true));
    assert_eq!(
        caps["supported_modes"],
        json!(["conservative", "idle", "balanced", "aggressive"])
    );
    assert_eq!(caps["supported_priorities"], json!(["background"]));

    let methods = rpc.call_value("status.methods", None, RPC_TIMEOUT).await?;
    assert_eq!(methods["control_plane_version"], 1);
    assert_eq!(methods["agent_version"], json!("0.1.0"));
    assert_eq!(
        methods["methods"]["events.since"]["params"]["fields"]["since_seq"]["type"],
        "u64"
    );
    assert_eq!(
        methods["methods"]["budget.set"]["errors"][1]["kind"],
        "invalid_budget"
    );

    let status = rpc.call_value("status.get", None, RPC_TIMEOUT).await?;
    assert_eq!(status["state"], "stopped");
    assert_eq!(status["hashrate_5m"], 0.0);
    assert_eq!(status["submitted_5m"], 0);

    let resumed_stopped = rpc.call_value("miner.resume", None, RPC_TIMEOUT).await?;
    assert_eq!(resumed_stopped["state"], "stopped");

    let idle = rpc
        .call_value(
            "budget.set_mode",
            Some(json!({"mode": "conservative"})),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(idle["state"], "stopped");
    assert_eq!(idle["current_budget"]["threads"], 1);
    assert_eq!(idle["current_budget"]["cpu_percent"], 50);

    let custom = rpc
        .call_value(
            "budget.set",
            Some(json!({"threads": 2, "cpu_percent": 33, "priority": "background"})),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(custom["current_budget"]["threads"], 2);
    assert_eq!(custom["current_budget"]["cpu_percent"], 33);
    assert_eq!(custom["current_budget"]["priority"], "background");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_plane_lifecycle_and_events_work() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(
        &pool.pool_addr().to_string(),
        "keccak",
        &[
            "--keepalive-interval-secs",
            "30",
            "--status-interval-secs",
            "1",
        ],
    )
    .await?;
    let mut ctl = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;
    let mut events = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;

    let subscribed = events.subscribe_events(RPC_TIMEOUT).await?;
    assert_eq!(subscribed["subscribed"], true);

    let start = ctl.call_value("miner.start", None, RPC_TIMEOUT).await?;
    assert_eq!(start["state"], "starting");
    assert_eq!(
        wait_for_event_type(&mut events, "started").await?["params"]["type"],
        "started"
    );
    assert_eq!(
        wait_for_state(&mut ctl, "running").await?["state"],
        "running"
    );

    let paused = ctl.call_value("miner.pause", None, RPC_TIMEOUT).await?;
    assert_eq!(paused["state"], "paused");
    assert_eq!(
        wait_for_event_type(&mut events, "paused").await?["params"]["type"],
        "paused"
    );

    let resumed = ctl.call_value("miner.resume", None, RPC_TIMEOUT).await?;
    assert_eq!(resumed["state"], "running");
    assert_eq!(
        wait_for_event_type(&mut events, "resumed").await?["params"]["type"],
        "resumed"
    );

    let resumed_again = ctl.call_value("miner.resume", None, RPC_TIMEOUT).await?;
    assert_eq!(resumed_again["state"], "running");

    let trend = wait_for_hashrate_window(&mut ctl).await?;
    assert!(trend["hashrate_5m"].as_f64().unwrap_or(0.0) > 0.0);

    let stopped = ctl.call_value("miner.stop", None, RPC_TIMEOUT).await?;
    assert_eq!(stopped["state"], "stopped");
    assert_eq!(
        wait_for_event_type(&mut events, "stopped").await?["params"]["type"],
        "stopped"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_plane_events_since_returns_buffered_events() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let agent = AgentProcess::spawn(&pool.pool_addr().to_string(), "keccak", &[]).await?;
    let mut ctl = RpcClient::connect(agent.socket_path(), RPC_TIMEOUT).await?;

    let baseline = ctl
        .call_value("events.since", Some(json!({"since_seq": 0})), RPC_TIMEOUT)
        .await?;
    let baseline_next_seq = baseline["next_seq"].as_u64().unwrap_or(0);

    let _ = ctl.call_value("miner.start", None, RPC_TIMEOUT).await?;
    let _ = wait_for_state(&mut ctl, "running").await?;
    let _ = ctl.call_value("miner.pause", None, RPC_TIMEOUT).await?;

    let events = wait_for_events_since(&mut ctl, baseline_next_seq.saturating_sub(1), 2).await?;
    assert!(events["next_seq"].as_u64().unwrap_or(0) >= baseline_next_seq + 2);
    let event_list = events["events"]
        .as_array()
        .context("events must be array")?;
    assert_eq!(event_list[0]["event"]["type"], "started");
    assert_eq!(event_list[1]["event"]["type"], "paused");
    assert!(
        event_list[0]["seq"].as_u64().unwrap_or(0) < event_list[1]["seq"].as_u64().unwrap_or(0)
    );

    let empty = ctl
        .call_value(
            "events.since",
            Some(json!({"since_seq": events["next_seq"].as_u64().unwrap_or(0)})),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(empty["events"], json!([]));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_plane_rejects_invalid_requests() -> Result<()> {
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

    let invalid_params = rpc
        .raw(
            json!({"jsonrpc": "2.0", "id": 3, "method": "budget.set_mode", "params": {"mode": "bogus"}}),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(invalid_params["error"]["code"], -32602);

    let paused_mode = rpc
        .raw(
            json!({"jsonrpc": "2.0", "id": 4, "method": "budget.set_mode", "params": {"mode": "paused"}}),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(paused_mode["error"]["code"], -32602);

    let invalid_priority = rpc
        .raw(
            json!({"jsonrpc": "2.0", "id": 5, "method": "budget.set", "params": {"priority": "normal"}}),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(invalid_priority["error"]["code"], -32602);

    let caps = rpc
        .call_value("status.capabilities", None, RPC_TIMEOUT)
        .await?;
    let max_threads = caps["max_threads"].as_u64().unwrap_or(1);
    let invalid_budget = rpc
        .raw(
            json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "budget.set",
                "params": {"threads": max_threads + 1}
            }),
            RPC_TIMEOUT,
        )
        .await?;
    assert_eq!(invalid_budget["error"]["code"], -32000);
    assert_eq!(invalid_budget["error"]["data"]["kind"], "invalid_budget");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_plane_events_stream_allows_follow_up_requests_on_same_connection() -> Result<()> {
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

async fn wait_for_hashrate_window(ctl: &mut RpcClient) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = ctl.status(RPC_TIMEOUT).await?;
        let value = serde_json::to_value(&status)?;
        if status.hashrate_5m > 0.0 {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(anyhow::anyhow!("wait hashrate_5m timeout"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_events_since(
    ctl: &mut RpcClient,
    since_seq: u64,
    expected_min: usize,
) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let events = ctl.events_since(since_seq, RPC_TIMEOUT).await?;
        let value = serde_json::to_value(&events)?;
        if events.events.len() >= expected_min {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(anyhow::anyhow!("wait events.since timeout"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
