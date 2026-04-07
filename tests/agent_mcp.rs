mod support;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use support::process::{resolve_stc_mint_agentctl_bin, temp_test_path, TEST_MUTEX};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_mcp_lists_safe_tools_and_supports_setup_then_status() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let state_path = temp_test_path("mcp-state", "json");
    let socket_path = temp_test_path("mcp-socket", "sock");
    let mut child = spawn_mcp(&state_path, &socket_path).await?;
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
            "setup",
            "set_wallet",
            "status",
            "start",
            "stop",
            "pause",
            "resume",
            "set_mode",
            "events_since",
        ]
    );

    let setup = client
        .request(
            3,
            "tools/call",
            json!({
                "name": "setup",
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

    let status = client
        .request(
            4,
            "tools/call",
            json!({
                "name": "status",
                "arguments": {}
            }),
        )
        .await?;
    assert_eq!(status["result"]["structuredContent"]["state"], "stopped");
    assert_eq!(
        status["result"]["structuredContent"]["current_budget"]["cpu_percent"],
        50
    );

    let _ = child.kill().await;
    let _ = std::fs::remove_file(state_path);
    Ok(())
}

async fn spawn_mcp(state_path: &PathBuf, socket_path: &PathBuf) -> Result<Child> {
    let ctl_bin = resolve_stc_mint_agentctl_bin()?;
    let child = Command::new(ctl_bin)
        .env("STC_MINT_AGENT_STATE_PATH", state_path)
        .arg("--socket")
        .arg(socket_path)
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn stc-mint-agentctl mcp failed")?;
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
