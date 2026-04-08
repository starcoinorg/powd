use super::command::{AgentCommand, MinerAction, WalletAction};
use super::wallet::WalletAgent;
use crate::{BudgetMode, MintNetwork, WalletAddress};
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
    #[serde(default)]
    network: Option<MintNetwork>,
}

#[derive(Deserialize)]
struct ModeArgs {
    mode: BudgetMode,
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
            if request.method == "notifications/initialized" {
                continue;
            }
            let id = request.id.unwrap_or(Value::Null);
            let response = self
                .handle_request(id, request.method, request.params)
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
                    "capabilities": { "tools": {} },
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
        let command = match build_command(&params.name, params.arguments) {
            Ok(command) => command,
            Err(err) => return tool_error(err),
        };
        match self.agent.execute(command).await {
            Ok(reply) => tool_ok(reply.to_value()),
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

fn build_command(name: &str, arguments: Value) -> Result<AgentCommand, String> {
    match name {
        "wallet_set" => {
            let (wallet_address, network) = parse_wallet_args(&arguments)?;
            Ok(AgentCommand::Wallet(WalletAction::Set {
                wallet_address,
                network,
            }))
        }
        "wallet_show" => Ok(AgentCommand::Wallet(WalletAction::Show)),
        "wallet_reward" => Ok(AgentCommand::Wallet(WalletAction::Reward)),
        "miner_status" => Ok(AgentCommand::Miner(MinerAction::Status)),
        "miner_start" => Ok(AgentCommand::Miner(MinerAction::Start)),
        "miner_stop" => Ok(AgentCommand::Miner(MinerAction::Stop)),
        "miner_pause" => Ok(AgentCommand::Miner(MinerAction::Pause)),
        "miner_resume" => Ok(AgentCommand::Miner(MinerAction::Resume)),
        "miner_set_mode" => Ok(AgentCommand::Miner(MinerAction::SetMode {
            mode: parse_mode_args(arguments)?,
        })),
        other => Err(format!("unknown tool: {other}")),
    }
}

fn parse_wallet_args(arguments: &Value) -> Result<(WalletAddress, Option<MintNetwork>), String> {
    let args: WalletArgs = serde_json::from_value(arguments.clone())
        .map_err(|err| format!("invalid wallet args: {err}"))?;
    Ok((
        WalletAddress::parse(args.wallet_address).map_err(|err| err.to_string())?,
        args.network,
    ))
}

fn parse_mode_args(arguments: Value) -> Result<BudgetMode, String> {
    let args: ModeArgs =
        serde_json::from_value(arguments).map_err(|err| format!("invalid mode args: {err}"))?;
    Ok(args.mode)
}

fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "wallet_set",
            description: "Persist or replace the payout wallet. First use creates a stable worker id. Later updates keep the same worker id and optionally switch network.",
            input_schema: wallet_schema(
                "Payout wallet address. On first use this creates a stable worker id; later calls preserve it.",
            ),
        },
        ToolSpec {
            name: "wallet_show",
            description: "Show the persisted wallet address, worker id, network, and derived login.",
            input_schema: empty_object_schema(),
        },
        ToolSpec {
            name: "wallet_reward",
            description: "Query external account reward totals from the configured pool-service HTTP API. This is separate from local miner runtime status.",
            input_schema: empty_object_schema(),
        },
        ToolSpec {
            name: "miner_status",
            description: "Read the current miner snapshot, including requested mode, effective budget, and auto state.",
            input_schema: empty_object_schema(),
        },
        ToolSpec {
            name: "miner_start",
            description: "Start mining with the configured wallet identity.",
            input_schema: empty_object_schema(),
        },
        ToolSpec {
            name: "miner_stop",
            description: "Stop mining and disconnect from the pool.",
            input_schema: empty_object_schema(),
        },
        ToolSpec {
            name: "miner_pause",
            description: "Pause solving without deleting wallet or daemon state. In auto mode this holds automatic budgeting until resume or start.",
            input_schema: empty_object_schema(),
        },
        ToolSpec {
            name: "miner_resume",
            description: "Resume solving. If the miner is stopped, this starts it. In auto mode this clears the hold state.",
            input_schema: empty_object_schema(),
        },
        ToolSpec {
            name: "miner_set_mode",
            description: "Set the miner mode. auto lets the daemon adjust budget from system CPU and memory usage; conservative, idle, balanced, and aggressive are fixed presets.",
            input_schema: mode_schema(
                "Mode to apply. auto lets the daemon adjust budget internally and never raises above the balanced ceiling by default.",
            ),
        },
    ]
}

fn wallet_schema(description: &str) -> Value {
    object_schema_with_optional(
        &[(
            "wallet_address",
            json!({
                "type": "string",
                "description": description,
            }),
        )],
        &[(
            "network",
            json!({
                "type": "string",
                "enum": ["main", "halley"],
                "description": "Optional network profile. Omit to keep the current network, or default to main on first use.",
            }),
        )],
    )
}

fn mode_schema(description: &str) -> Value {
    object_schema(&[(
        "mode",
        json!({
            "type": "string",
            "enum": ["auto", "conservative", "idle", "balanced", "aggressive"],
            "description": description,
        }),
    )])
}

fn empty_object_schema() -> Value {
    object_schema::<&str>(&[])
}

fn object_schema<S: AsRef<str>>(fields: &[(S, Value)]) -> Value {
    let properties = fields
        .iter()
        .map(|(name, schema)| (name.as_ref().to_string(), schema.clone()))
        .collect::<serde_json::Map<_, _>>();
    let required = fields
        .iter()
        .map(|(name, _)| Value::String(name.as_ref().to_string()))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn object_schema_with_optional<S: AsRef<str>, T: AsRef<str>>(
    required_fields: &[(S, Value)],
    optional_fields: &[(T, Value)],
) -> Value {
    let mut properties = serde_json::Map::new();
    for (name, schema) in required_fields {
        properties.insert(name.as_ref().to_string(), schema.clone());
    }
    for (name, schema) in optional_fields {
        properties.insert(name.as_ref().to_string(), schema.clone());
    }
    let required = required_fields
        .iter()
        .map(|(name, _)| Value::String(name.as_ref().to_string()))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn parse_params<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> Result<T, String> {
    serde_json::from_value(params.unwrap_or(Value::Object(Default::default())))
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
            text: serde_json::to_string_pretty(&value).expect("format tool result"),
        }],
        structured_content: Some(value),
        is_error: false,
    })
    .expect("encode tool result")
}

fn tool_error(message: impl Into<String>) -> Value {
    let message = message.into();
    serde_json::to_value(CallToolResult {
        content: vec![ContentBlock {
            kind: "text",
            text: message.clone(),
        }],
        structured_content: Some(json!({ "error": message })),
        is_error: true,
    })
    .expect("encode tool error")
}
