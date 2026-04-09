use super::state::SharedState;
use crate::agent::config::MintProfile;
use crate::{
    AgentError, AgentErrorKind, BudgetMode, MinerEvent, MintNetwork, WalletAddress, WorkerName,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{unix::OwnedWriteHalf, UnixStream};
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct RpcRequest {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<RpcErrorData>,
}

#[derive(Serialize)]
struct RpcErrorData {
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<AgentErrorKind>,
}

pub struct RpcFailure {
    pub code: i64,
    pub message: String,
    pub kind: Option<AgentErrorKind>,
}

#[derive(Serialize)]
struct EventNotification<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    params: &'a MinerEvent,
}

#[derive(Deserialize)]
struct SetModeParams {
    mode: BudgetMode,
}

#[derive(Deserialize)]
struct EventsSinceParams {
    since_seq: u64,
}

#[derive(Deserialize)]
struct ConfigureParams {
    wallet_address: WalletAddress,
    worker_name: WorkerName,
    requested_mode: BudgetMode,
    network: MintNetwork,
}

type RpcResult<T> = std::result::Result<T, RpcFailure>;
type ConnectionWriter = Arc<Mutex<OwnedWriteHalf>>;

pub async fn serve_connection(
    stream: UnixStream,
    state: SharedState,
    shutdown: CancellationToken,
) -> Result<()> {
    verify_peer_credentials(&stream)?;
    let (read_half, write_half) = stream.into_split();
    let writer: ConnectionWriter = Arc::new(Mutex::new(write_half));
    let mut lines = BufReader::new(read_half).lines();
    let mut event_task: Option<JoinHandle<()>> = None;
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request: RpcRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(err) => {
                let failure = RpcFailure::invalid_request(format!("parse rpc request: {err}"));
                write_response(&writer, &failure_response(Value::Null, failure)).await?;
                continue;
            }
        };
        let id = request.id.clone().unwrap_or(Value::Null);
        match handle_request(request, &state, &shutdown).await {
            Ok(ResponseMode::Single(result)) => {
                write_response(&writer, &success_response(id, result)).await?;
            }
            Ok(ResponseMode::Subscribe(mut events)) => {
                if event_task.is_some() {
                    write_response(
                        &writer,
                        &failure_response(
                            id,
                            RpcFailure {
                                code: -32000,
                                message: "events already subscribed on this connection".to_string(),
                                kind: None,
                            },
                        ),
                    )
                    .await?;
                    continue;
                }
                write_response(
                    &writer,
                    &success_response(id, serde_json::json!({"subscribed": true})),
                )
                .await?;
                let writer = Arc::clone(&writer);
                event_task = Some(tokio::spawn(async move {
                    loop {
                        match events.recv().await {
                            Ok(event) => {
                                if write_response(
                                    &writer,
                                    &EventNotification {
                                        jsonrpc: "2.0",
                                        method: "event",
                                        params: &event,
                                    },
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }));
            }
            Err(err) => {
                write_response(&writer, &failure_response(id, err)).await?;
            }
        }
    }
    if let Some(task) = event_task {
        task.abort();
        let _ = task.await;
    }
    Ok(())
}

enum ResponseMode {
    Single(Value),
    Subscribe(broadcast::Receiver<MinerEvent>),
}

async fn handle_request(
    request: RpcRequest,
    state: &SharedState,
    shutdown: &CancellationToken,
) -> RpcResult<ResponseMode> {
    validate_request(&request)?;
    match request.method.as_str() {
        "miner.start" => Ok(ResponseMode::Single(serialize_result(
            state.start().await.map_err(|err| RpcFailure::agent(&err))?,
        )?)),
        "miner.stop" => Ok(ResponseMode::Single(serialize_result(
            state.stop().await.map_err(|err| RpcFailure::agent(&err))?,
        )?)),
        "miner.pause" => Ok(ResponseMode::Single(serialize_result(
            state.pause().await.map_err(|err| RpcFailure::agent(&err))?,
        )?)),
        "miner.resume" => Ok(ResponseMode::Single(serialize_result(
            state
                .resume()
                .await
                .map_err(|err| RpcFailure::agent(&err))?,
        )?)),
        "miner.set_mode" => {
            let params: SetModeParams = parse_params(request.params)?;
            Ok(ResponseMode::Single(serialize_result(
                state
                    .set_mode(params.mode)
                    .await
                    .map_err(|err| RpcFailure::agent(&err))?,
            )?))
        }
        "daemon.configure" => {
            let params: ConfigureParams = parse_params(request.params)?;
            Ok(ResponseMode::Single(serialize_result(
                state
                    .configure(MintProfile {
                        wallet_address: params.wallet_address,
                        worker_name: params.worker_name,
                        requested_mode: params.requested_mode,
                        network: params.network,
                    })
                    .await
                    .map_err(|err| RpcFailure::agent(&err))?,
            )?))
        }
        "status.get" => Ok(ResponseMode::Single(serialize_result(
            state.snapshot().await,
        )?)),
        "status.capabilities" => Ok(ResponseMode::Single(serialize_result(
            state.capabilities().await,
        )?)),
        "status.methods" => Ok(ResponseMode::Single(serialize_result(
            state.methods().await,
        )?)),
        "events.since" => {
            let params: EventsSinceParams = parse_params(request.params)?;
            Ok(ResponseMode::Single(serialize_result(
                state.events_since(params.since_seq).await,
            )?))
        }
        "daemon.shutdown" => {
            shutdown.cancel();
            Ok(ResponseMode::Single(serialize_result(
                serde_json::json!({ "shutting_down": true }),
            )?))
        }
        "events.stream" => Ok(ResponseMode::Subscribe(state.subscribe_events())),
        other => Err(RpcFailure::method_not_found(format!(
            "unknown method: {other}"
        ))),
    }
}

fn parse_params<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> RpcResult<T> {
    serde_json::from_value(params.unwrap_or(Value::Object(Default::default())))
        .map_err(|err| RpcFailure::invalid_params(format!("invalid params: {err}")))
}

fn validate_request(request: &RpcRequest) -> RpcResult<()> {
    if let Some(version) = request.jsonrpc.as_deref() {
        if version != "2.0" {
            return Err(RpcFailure::invalid_request(format!(
                "unsupported jsonrpc version: {version}"
            )));
        }
    }
    Ok(())
}

fn serialize_result<T: Serialize>(value: T) -> RpcResult<Value> {
    serde_json::to_value(value).map_err(|err| RpcFailure::internal(err.to_string()))
}

fn success_response(id: Value, result: Value) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn failure_response(id: Value, failure: RpcFailure) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError {
            code: failure.code,
            message: failure.message,
            data: failure.kind.map(|kind| RpcErrorData { kind: Some(kind) }),
        }),
    }
}

impl RpcFailure {
    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            kind: None,
        }
    }

    fn method_not_found(message: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: message.into(),
            kind: None,
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            kind: None,
        }
    }

    fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            code: -32000,
            message: err.to_string(),
            kind: None,
        }
    }

    fn agent(err: &AgentError) -> Self {
        Self {
            code: -32000,
            message: err.to_string(),
            kind: Some(err.kind()),
        }
    }
}

async fn write_response<T: Serialize>(writer: &ConnectionWriter, payload: &T) -> Result<()> {
    let mut guard = writer.lock().await;
    let mut encoded = serde_json::to_vec(payload).context("encode response")?;
    encoded.push(b'\n');
    guard.write_all(&encoded).await.context("write response")?;
    guard.flush().await.context("flush response")
}

fn verify_peer_credentials(stream: &UnixStream) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsRawFd;

        let fd = stream.as_raw_fd();
        let mut ucred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut ucred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error()).context("read peer credentials");
        }
        let expected_uid = unsafe { libc::geteuid() };
        if ucred.uid != expected_uid {
            anyhow::bail!(
                "reject peer uid {} on local api socket; expected {expected_uid}",
                ucred.uid
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;

        let fd = stream.as_raw_fd();
        let mut peer_euid: libc::uid_t = 0;
        let mut peer_egid: libc::gid_t = 0;
        let rc = unsafe { libc::getpeereid(fd, &mut peer_euid, &mut peer_egid) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error()).context("read peer credentials");
        }
        let expected_uid = unsafe { libc::geteuid() };
        if peer_euid != expected_uid {
            anyhow::bail!(
                "reject peer uid {peer_euid} on local api socket; expected {expected_uid}"
            );
        }
    }

    Ok(())
}
