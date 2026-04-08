use super::command::{AgentCommand, MinerAction, WalletAction};
use super::wallet::WalletAgent;
use crate::{BudgetMode, MintNetwork, WalletAddress};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Stdin, Stdout};

const MCP_PROTOCOL_VERSION_LATEST: McpProtocolVersion = McpProtocolVersion::V20251125;

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
    protocol_version: McpProtocolVersion,
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
struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'static str>,
    description: &'static str,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
    annotations: ToolAnnotations,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolAnnotations {
    read_only_hint: bool,
    destructive_hint: bool,
    idempotent_hint: bool,
    open_world_hint: bool,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpProtocolVersion {
    V20251125,
    V20250618,
    V20250326,
    V20241105,
}

impl McpProtocolVersion {
    const SUPPORTED: [Self; 4] = [
        Self::V20251125,
        Self::V20250618,
        Self::V20250326,
        Self::V20241105,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::V20251125 => "2025-11-25",
            Self::V20250618 => "2025-06-18",
            Self::V20250326 => "2025-03-26",
            Self::V20241105 => "2024-11-05",
        }
    }

    fn supports_tool_title(self) -> bool {
        matches!(self, Self::V20251125 | Self::V20250618)
    }

    fn negotiate(requested: &str) -> Self {
        if let Some(exact) = Self::SUPPORTED
            .iter()
            .copied()
            .find(|version| version.as_str() == requested)
        {
            return exact;
        }

        if is_iso_date(requested) {
            return Self::SUPPORTED
                .iter()
                .copied()
                .find(|version| version.as_str() <= requested)
                .unwrap_or(Self::V20241105);
        }

        MCP_PROTOCOL_VERSION_LATEST
    }
}

impl McpServer {
    fn new(agent: WalletAgent, stdin: Stdin, stdout: Stdout) -> Self {
        Self {
            agent,
            reader: BufReader::new(stdin),
            writer: stdout,
            protocol_version: MCP_PROTOCOL_VERSION_LATEST,
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
        &mut self,
        id: Value,
        method: String,
        params: Option<Value>,
    ) -> McpResponse {
        match method.as_str() {
            "initialize" => {
                let params: InitializeParams = match parse_params(params) {
                    Ok(params) => params,
                    Err(err) => return invalid_params(id, err),
                };
                self.protocol_version = McpProtocolVersion::negotiate(&params.protocol_version);
                success(
                    id,
                    json!({
                        "protocolVersion": self.protocol_version.as_str(),
                        "capabilities": {
                            "tools": { "listChanged": false }
                        },
                        "serverInfo": {
                            "name": "powctl",
                            "version": env!("CARGO_PKG_VERSION"),
                        }
                    }),
                )
            }
            "ping" => success(id, json!({})),
            "tools/list" => success(
                id,
                serde_json::to_value(ToolListResult {
                    tools: tool_specs(self.protocol_version),
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

fn tool_specs(protocol_version: McpProtocolVersion) -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "wallet_set",
            title: tool_title(protocol_version, "Set Wallet"),
            description: concat!(
                "Use this when the user wants to set, change, or replace the persisted payout wallet, ",
                "including requests like \"change my payout wallet\", \"switch to halley\", \"换钱包\", or \"改收款地址\". ",
                "Do not use this when the user only wants to inspect the current wallet or ask about earnings. ",
                "Prefer wallet_show for current identity questions and prefer wallet_reward for earnings or payout totals. ",
                "Confirm before calling because this changes the persisted payout identity and may immediately reconfigure a running daemon."
            ),
            input_schema: wallet_schema(),
            annotations: local_write_tool(true, true),
        },
        ToolSpec {
            name: "wallet_show",
            title: tool_title(protocol_version, "Show Wallet"),
            description: concat!(
                "Use this when the user asks which wallet is configured now, what the worker name is, ",
                "what login string is in use, or says things like \"current wallet\", \"当前钱包\", or \"收款地址是什么\". ",
                "Do not use this when the user wants to change the wallet or ask about rewards. ",
                "Prefer wallet_set for changing payout identity and prefer wallet_reward for earnings, payouts, or pending reward totals."
            ),
            input_schema: empty_object_schema(),
            annotations: read_only_tool(false),
        },
        ToolSpec {
            name: "wallet_reward",
            title: tool_title(protocol_version, "Wallet Rewards"),
            description: concat!(
                "Use this when the user asks about earnings, rewards, pending payouts, or says things like ",
                "\"how much have I earned\", \"收益\", or \"奖励\". ",
                "Do not use this when the user wants to know whether mining is running right now or which mode is active. ",
                "Prefer miner_status for live local runtime state and prefer wallet_show for current wallet identity. ",
                "This is an external account query against the configured pool-service HTTP API."
            ),
            input_schema: empty_object_schema(),
            annotations: read_only_tool(true),
        },
        ToolSpec {
            name: "miner_status",
            title: tool_title(protocol_version, "Miner Status"),
            description: concat!(
                "Use this when the user asks what is running now, what mode is active, why the miner is held, ",
                "or says things like \"show my mining status\", \"当前状态\", or \"现在在不在挖\". ",
                "Do not use this when the user wants reward totals or wants to change runtime behavior. ",
                "Prefer wallet_reward for earnings questions and prefer miner_set_mode, miner_pause, or miner_stop when the user wants to change behavior."
            ),
            input_schema: empty_object_schema(),
            annotations: read_only_tool(false),
        },
        ToolSpec {
            name: "miner_start",
            title: tool_title(protocol_version, "Start Miner"),
            description: concat!(
                "Use this when the user explicitly wants mining to begin or come online, with wording like ",
                "\"start mining\", \"begin mining\", \"启动挖矿\", or \"开始挖矿\". ",
                "Do not use this when the user clearly means to continue after a temporary pause. ",
                "Prefer miner_resume when the user says \"resume\", \"continue\", or \"恢复挖矿\" after a prior pause. ",
                "Confirm before calling if the user did not clearly ask to begin live mining, because it can start local CPU work and connect to the pool."
            ),
            input_schema: empty_object_schema(),
            annotations: runtime_write_tool(false),
        },
        ToolSpec {
            name: "miner_stop",
            title: tool_title(protocol_version, "Stop Miner"),
            description: concat!(
                "Use this when the user explicitly wants mining turned off or shut down completely, with wording like ",
                "\"stop mining completely\", \"turn mining off\", \"彻底停掉\", or \"停止挖矿\". ",
                "Do not use this when the user only wants a temporary pause and expects to resume later. ",
                "Prefer miner_pause for requests like \"pause for now\", \"先停一下\", or \"resume later\". ",
                "Confirm before calling because it halts current mining activity until the miner is started or resumed again."
            ),
            input_schema: empty_object_schema(),
            annotations: runtime_write_tool(true),
        },
        ToolSpec {
            name: "miner_pause",
            title: tool_title(protocol_version, "Pause Miner"),
            description: concat!(
                "Use this when the user wants a temporary pause without losing wallet or daemon state, with wording like ",
                "\"pause mining\", \"resume later\", \"暂停\", or \"先停一下\". ",
                "Do not use this when the user clearly wants mining shut down completely. ",
                "Prefer miner_stop for full shutdown requests like \"turn mining off\" or \"彻底停掉\". ",
                "Confirm before calling because it changes live miner behavior while preserving configuration."
            ),
            input_schema: empty_object_schema(),
            annotations: runtime_write_tool(false),
        },
        ToolSpec {
            name: "miner_resume",
            title: tool_title(protocol_version, "Resume Miner"),
            description: concat!(
                "Use this when the user wants to continue after a prior pause, with wording like ",
                "\"resume mining\", \"continue\", \"恢复挖矿\", or \"继续挖\". ",
                "Do not use this for fresh start requests where the user simply wants mining enabled from an off state. ",
                "Prefer miner_start for \"start\" or \"begin mining\" phrasing without pause context. ",
                "Confirm before calling if the user did not clearly ask to change the live runtime."
            ),
            input_schema: empty_object_schema(),
            annotations: runtime_write_tool(false),
        },
        ToolSpec {
            name: "miner_set_mode",
            title: tool_title(protocol_version, "Set Miner Mode"),
            description: concat!(
                "Use this when the user wants mining to continue but at a different intensity, with wording like ",
                "\"make mining less aggressive\", \"调低一点\", \"安静点\", \"省电点\", or \"更激进\". ",
                "Do not use this when the user wants to stop mining or only inspect the current mode. ",
                "Prefer miner_pause or miner_stop when the user wants mining to stop, and prefer miner_status when the user only wants to inspect the current mode. ",
                "Confirm before calling because it changes ongoing CPU budget selection."
            ),
            input_schema: mode_schema(),
            annotations: local_write_tool(false, false),
        },
    ]
}

fn wallet_schema() -> Value {
    with_examples(
        object_schema_with_optional(
            &[(
                "wallet_address",
                json!({
                    "type": "string",
                    "description": "New payout wallet address to persist. This is the receiving wallet for future payouts, not the worker name or login string.",
                }),
            )],
            &[(
                "network",
                json!({
                    "type": "string",
                    "enum": ["main", "halley"],
                    "description": "Optional payout network profile. main = main network payouts. halley = halley test-network payouts. Omit to keep the current network, or default to main on first use.",
                }),
            )],
        ),
        vec![
            json!({
                "wallet_address": "0x11111111111111111111111111111111"
            }),
            json!({
                "wallet_address": "0x11111111111111111111111111111111",
                "network": "halley"
            }),
        ],
    )
}

fn mode_schema() -> Value {
    with_examples(
        object_schema(&[(
            "mode",
            json!({
                "type": "string",
                "enum": ["auto", "conservative", "idle", "balanced", "aggressive"],
                "description": "Requested mining intensity. auto = let the daemon choose a safe budget tier. conservative = lower sustained CPU usage. idle = minimal background work. balanced = normal everyday mining. aggressive = highest local CPU budget. Use this to tune intensity without stopping mining.",
            }),
        )]),
        vec![
            json!({ "mode": "balanced" }),
            json!({ "mode": "conservative" }),
            json!({ "mode": "aggressive" }),
            json!({ "mode": "auto" }),
        ],
    )
}

fn empty_object_schema() -> Value {
    with_examples(object_schema::<&str>(&[]), vec![json!({})])
}

fn with_examples(mut schema: Value, examples: Vec<Value>) -> Value {
    schema
        .as_object_mut()
        .expect("object schema")
        .insert("examples".to_string(), Value::Array(examples));
    schema
}

fn read_only_tool(open_world_hint: bool) -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: true,
        destructive_hint: false,
        idempotent_hint: true,
        open_world_hint,
    }
}

fn local_write_tool(destructive_hint: bool, open_world_hint: bool) -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: false,
        destructive_hint,
        idempotent_hint: false,
        open_world_hint,
    }
}

fn runtime_write_tool(destructive_hint: bool) -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: false,
        destructive_hint,
        idempotent_hint: false,
        open_world_hint: true,
    }
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

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn tool_title(protocol_version: McpProtocolVersion, title: &'static str) -> Option<&'static str> {
    protocol_version.supports_tool_title().then_some(title)
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
