#[path = "support/fake_reward_api.rs"]
mod fake_reward_api;
#[path = "support/process_mcp.rs"]
mod process;

use anyhow::{Context, Result};
use fake_reward_api::FakeRewardApi;
use process::{resolve_powctl_bin, temp_test_path, TEST_MUTEX};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_mcp_lists_public_business_tools_and_handles_wallet_and_mode() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let state_path = temp_test_path("mcp-state", "json");
    let socket_path = temp_test_path("mcp-socket", "sock");
    let reward_api = FakeRewardApi::start_json(json!({
        "account": "0x44444444444444444444444444444444",
        "generated_at_millis": 123,
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
            "confirmed_blocks_24h": 1,
            "orphaned_blocks_24h": 0,
            "confirmed_total": "1000000000",
            "paid_total": "200000000",
            "confirmed_through_height": 999,
            "estimated_pending_total": "300000000",
            "last_share_at_millis": null
        },
        "workers": []
    }))
    .await?;
    let reward_api_base = reward_api.base_url();
    let mut child = spawn_mcp(&state_path, &socket_path, &reward_api_base).await?;
    let stdin = child.stdin.take().context("take mcp stdin failed")?;
    let stdout = child.stdout.take().context("take mcp stdout failed")?;
    let mut client = McpClient::new(stdin, stdout);

    let initialize = client
        .request(
            1,
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.1.0" }
            }),
        )
        .await?;
    assert_eq!(initialize["result"]["protocolVersion"], "2024-11-05");

    let tools = client.request(2, "tools/list", json!({})).await?;
    let names = tools["result"]["tools"]
        .as_array()
        .context("tools list should be an array")?
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "wallet_set",
            "wallet_show",
            "wallet_reward",
            "miner_status",
            "miner_start",
            "miner_stop",
            "miner_pause",
            "miner_resume",
            "miner_set_mode",
        ]
    );

    let setup = client
        .request(
            3,
            "tools/call",
            json!({
                "name": "wallet_set",
                "arguments": {
                    "wallet_address": "0x44444444444444444444444444444444"
                }
            }),
        )
        .await?;
    assert_eq!(
        setup["result"]["structuredContent"]["wallet_address"],
        "0x44444444444444444444444444444444"
    );
    assert_eq!(setup["result"]["structuredContent"]["network"], "main");

    let show = client
        .request(
            4,
            "tools/call",
            json!({
                "name": "wallet_show",
                "arguments": {}
            }),
        )
        .await?;
    assert_eq!(
        show["result"]["structuredContent"]["wallet_address"],
        "0x44444444444444444444444444444444"
    );

    let reward = client
        .request(
            5,
            "tools/call",
            json!({
                "name": "wallet_reward",
                "arguments": {}
            }),
        )
        .await?;
    assert_eq!(
        reward["result"]["structuredContent"]["confirmed_total_display"],
        "1.0 STC"
    );
    assert_eq!(
        reward_api.last_request_path().as_deref(),
        Some("/v1/mining/dashboard/0x44444444444444444444444444444444?window_secs=300")
    );

    let status = client
        .request(
            6,
            "tools/call",
            json!({
                "name": "miner_status",
                "arguments": {}
            }),
        )
        .await?;
    assert_eq!(status["result"]["structuredContent"]["state"], "stopped");
    assert_eq!(
        status["result"]["structuredContent"]["requested_mode"],
        "auto"
    );
    assert_eq!(status["result"]["structuredContent"]["auto_state"], "held");

    let mode = client
        .request(
            7,
            "tools/call",
            json!({
                "name": "miner_set_mode",
                "arguments": { "mode": "auto" }
            }),
        )
        .await?;
    assert_eq!(
        mode["result"]["structuredContent"]["requested_mode"],
        "auto"
    );

    let set_wallet = client
        .request(
            8,
            "tools/call",
            json!({
                "name": "wallet_set",
                "arguments": {
                    "wallet_address": "0x55555555555555555555555555555555"
                }
            }),
        )
        .await?;
    assert_eq!(
        set_wallet["result"]["structuredContent"]["wallet_address"],
        "0x55555555555555555555555555555555"
    );
    assert_eq!(
        set_wallet["result"]["structuredContent"]["worker_id"],
        setup["result"]["structuredContent"]["worker_id"]
    );

    let _ = child.kill().await;
    let _ = std::fs::remove_file(state_path);
    Ok(())
}

async fn spawn_mcp(
    state_path: &PathBuf,
    socket_path: &PathBuf,
    reward_api_base: &str,
) -> Result<Child> {
    let ctl_bin = resolve_powctl_bin()?;
    let child = Command::new(ctl_bin)
        .env("POWD_STATE_PATH", state_path)
        .env("POWD_MAIN_REWARD_API", reward_api_base)
        .arg("--socket")
        .arg(socket_path)
        .arg("integrate")
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn powctl integrate mcp failed")?;
    Ok(child)
}

struct McpClient {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpClient {
    fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin,
            stdout: BufReader::new(stdout),
        }
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let encoded = serde_json::to_vec(&payload)?;
        self.stdin
            .write_all(format!("Content-Length: {}\r\n\r\n", encoded.len()).as_bytes())
            .await?;
        self.stdin.write_all(&encoded).await?;
        self.stdin.flush().await?;
        self.read_response().await
    }

    async fn read_response(&mut self) -> Result<Value> {
        let mut content_length = None;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).await?;
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                content_length = Some(value.trim().parse::<usize>()?);
            }
        }
        let length = content_length.context("missing Content-Length header")?;
        let mut payload = vec![0_u8; length];
        self.stdout.read_exact(&mut payload).await?;
        serde_json::from_slice(&payload).context("parse MCP response failed")
    }
}
