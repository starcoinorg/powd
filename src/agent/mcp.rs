use super::wallet::WalletAgent;
use crate::BudgetMode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Stdin, Stdout};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

pub async fn run_mcp(agent: WalletAgent) -> io::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut server = McpServer::new(agent, stdin, stdout);
    server.run().await
}

struct McpServer {
    agent: WalletAgent,
    reader: BufReader<Stdin>,
    writer: Stdout,
}

#[derive(Deserialize)]
struct McpRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Serialize)]
struct McpResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpError>,
}

#[derive(Serialize)]
struct McpError {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct CallToolParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Deserialize)]
struct WalletArgs {
    wallet_address: String,
}

#[derive(Deserialize)]
struct SetModeArgs {
    mode: BudgetMode,
}

#[derive(Deserialize)]
struct EventsSinceArgs {
    since_seq: u64,
}

#[derive(Serialize)]
struct ToolListResult {
    tools: Vec<ToolSpec>,
}

#[derive(Serialize)]
struct ToolSpec {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
}

#[derive(Serialize)]
struct CallToolResult {
    content: Vec<ContentBlock>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    structured_content: Option<Value>,
    #[serde(rename = "isError", skip_serializing_if = "std::ops::Not::not")]
    is_error: bool,
}

#[derive(Serialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

impl McpServer {
    fn new(agent: WalletAgent, stdin: Stdin, stdout: Stdout) -> Self {
        Self {
            agent,
            reader: BufReader::new(stdin),
            writer: stdout,
        }
    }

    async fn run(&mut self) -> io::Result<()> {
        while let Some(message) = self.read_message().await? {
            let request: McpRequest = match serde_json::from_slice(&message) {
                Ok(request) => request,
                Err(err) => {
                    self.write_response(&McpResponse {
                        jsonrpc: "2.0",
                        id: Value::Null,
                        result: None,
                        error: Some(McpError {
                            code: -32700,
                            message: format!("parse request failed: {err}"),
                        }),
                    })
                    .await?;
                    continue;
                }
            };
            if request.jsonrpc != "2.0" {
                self.write_response(&McpResponse {
                    jsonrpc: "2.0",
                    id: request.id.unwrap_or(Value::Null),
                    result: None,
                    error: Some(McpError {
                        code: -32600,
                        message: "unsupported jsonrpc version".to_string(),
                    }),
                })
                .await?;
                continue;
            }
            if request.method.starts_with("notifications/") {
                if request.method == "notifications/initialized" {
                    continue;
                }
            }
            let id = request.id.unwrap_or(Value::Null);
            let response = self
                .handle_request(id.clone(), request.method, request.params)
                .await;
            self.write_response(&response).await?;
        }
        Ok(())
    }

    async fn handle_request(
        &self,
        id: Value,
        method: String,
        params: Option<Value>,
    ) -> McpResponse {
        match method.as_str() {
            "initialize" => success(
                id,
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "stc-mint-agentctl",
                        "version": env!("CARGO_PKG_VERSION"),
                    }
                }),
            ),
            "ping" => success(id, json!({})),
            "tools/list" => success(
                id,
                serde_json::to_value(ToolListResult {
                    tools: tool_specs(),
                })
                .expect("encode tools"),
            ),
            "tools/call" => {
                let params: CallToolParams = match parse_params(params) {
                    Ok(params) => params,
                    Err(err) => return invalid_params(id, err),
                };
                success(id, self.call_tool(params).await)
            }
            other => McpResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(McpError {
                    code: -32601,
                    message: format!("unknown method: {other}"),
                }),
            },
        }
    }

    async fn call_tool(&self, params: CallToolParams) -> Value {
        let result = match params.name.as_str() {
            "setup" => {
                let args: WalletArgs = match serde_json::from_value(params.arguments) {
                    Ok(args) => args,
                    Err(err) => return tool_error(format!("invalid setup args: {err}")),
                };
                self.agent
                    .setup(&args.wallet_address)
                    .await
                    .map(|value| json!(value))
            }
            "set_wallet" => {
                let args: WalletArgs = match serde_json::from_value(params.arguments) {
                    Ok(args) => args,
                    Err(err) => return tool_error(format!("invalid set_wallet args: {err}")),
                };
                self.agent
                    .update_wallet(&args.wallet_address)
                    .await
                    .map(|value| json!(value))
            }
            "status" => self.agent.status().await.map(|value| json!(value)),
            "capabilities" => self.agent.capabilities().await.map(|value| json!(value)),
            "methods" => self.agent.methods().await,
            "start" => self.agent.start().await.map(|value| json!(value)),
            "stop" => self.agent.stop().await.map(|value| json!(value)),
            "pause" => self.agent.pause().await.map(|value| json!(value)),
            "resume" => self.agent.resume().await.map(|value| json!(value)),
            "set_mode" => {
                let args: SetModeArgs = match serde_json::from_value(params.arguments) {
                    Ok(args) => args,
                    Err(err) => return tool_error(format!("invalid set_mode args: {err}")),
                };
                self.agent
                    .set_mode(args.mode)
                    .await
                    .map(|value| json!(value))
            }
            "events_since" => {
                let args: EventsSinceArgs = match serde_json::from_value(params.arguments) {
                    Ok(args) => args,
                    Err(err) => return tool_error(format!("invalid events_since args: {err}")),
                };
                self.agent
                    .events_since(args.since_seq)
                    .await
                    .map(|value| json!(value))
            }
            other => return tool_error(format!("unknown tool: {other}")),
        };
        match result {
            Ok(value) => tool_ok(value),
            Err(err) => tool_error(err.to_string()),
        }
    }

    async fn read_message(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut content_length = None;
        loop {
            let mut line = String::new();
            let bytes = self.reader.read_line(&mut line).await?;
            if bytes == 0 {
                return Ok(None);
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
        let length = content_length.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
        })?;
        let mut payload = vec![0_u8; length];
        self.reader.read_exact(&mut payload).await?;
        Ok(Some(payload))
    }

    async fn write_response<T: Serialize>(&mut self, response: &T) -> io::Result<()> {
        let payload = serde_json::to_vec(response)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        self.writer
            .write_all(format!("Content-Length: {}\r\n\r\n", payload.len()).as_bytes())
            .await?;
        self.writer.write_all(&payload).await?;
        self.writer.flush().await
    }
}

fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "setup",
            description: "Configure the payout wallet and create a stable worker id.",
            input_schema: wallet_schema(),
        },
        ToolSpec {
            name: "set_wallet",
            description: "Change the payout wallet. The stable worker id is preserved.",
            input_schema: wallet_schema(),
        },
        ToolSpec {
            name: "status",
            description: "Read the current miner snapshot.",
            input_schema: object_schema(&[]),
        },
        ToolSpec {
            name: "capabilities",
            description: "Read supported modes, priorities, and thread limits.",
            input_schema: object_schema(&[]),
        },
        ToolSpec {
            name: "methods",
            description: "Read the self-describing local API method schema.",
            input_schema: object_schema(&[]),
        },
        ToolSpec {
            name: "start",
            description: "Start mining with the configured payout wallet.",
            input_schema: object_schema(&[]),
        },
        ToolSpec {
            name: "stop",
            description: "Stop mining and disconnect from the pool.",
            input_schema: object_schema(&[]),
        },
        ToolSpec {
            name: "pause",
            description: "Pause solving while keeping the daemon alive.",
            input_schema: object_schema(&[]),
        },
        ToolSpec {
            name: "resume",
            description: "Resume solving; starts the miner if it was stopped.",
            input_schema: object_schema(&[]),
        },
        ToolSpec {
            name: "set_mode",
            description: "Switch between safe preset mining modes.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["conservative", "idle", "balanced", "aggressive"]
                    }
                },
                "required": ["mode"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "events_since",
            description: "Fetch buffered miner events after a sequence number.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "since_seq": { "type": "integer", "minimum": 0 }
                },
                "required": ["since_seq"],
                "additionalProperties": false
            }),
        },
    ]
}

fn wallet_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "wallet_address": { "type": "string" }
        },
        "required": ["wallet_address"],
        "additionalProperties": false
    })
}

fn object_schema(required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": {},
        "required": required,
        "additionalProperties": false
    })
}

fn parse_params<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> Result<T, String> {
    serde_json::from_value(params.unwrap_or_else(|| json!({})))
        .map_err(|err| format!("invalid params: {err}"))
}

fn success(id: Value, result: Value) -> McpResponse {
    McpResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn invalid_params(id: Value, message: String) -> McpResponse {
    McpResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(McpError {
            code: -32602,
            message,
        }),
    }
}

fn tool_ok(value: Value) -> Value {
    serde_json::to_value(CallToolResult {
        content: vec![ContentBlock {
            kind: "text",
            text: "ok".to_string(),
        }],
        structured_content: Some(value),
        is_error: false,
    })
    .expect("encode successful tool result")
}

fn tool_error(message: String) -> Value {
    serde_json::to_value(CallToolResult {
        content: vec![ContentBlock {
            kind: "text",
            text: message,
        }],
        structured_content: None,
        is_error: true,
    })
    .expect("encode error tool result")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_specs_expose_only_safe_surface() {
        let names = tool_specs()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "setup",
                "set_wallet",
                "status",
                "capabilities",
                "methods",
                "start",
                "stop",
                "pause",
                "resume",
                "set_mode",
                "events_since",
            ]
        );
    }
}
