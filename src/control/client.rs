use crate::{
    ControlErrorKind, ControlPlaneMethods, EventsSinceResponse, MinerCapabilities, MinerEvent,
    MinerSnapshot,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{unix::OwnedWriteHalf, UnixStream};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RpcFailure {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<RpcFailureData>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RpcFailureData {
    #[serde(default)]
    pub kind: Option<ControlErrorKind>,
}

#[derive(Debug)]
pub enum ControlClientError {
    Connect {
        path: PathBuf,
        source: std::io::Error,
    },
    Io(std::io::Error),
    Timeout {
        operation: &'static str,
        timeout: Duration,
    },
    Parse(serde_json::Error),
    Rpc(RpcFailure),
    Protocol(String),
}

impl Display for ControlClientError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect { path, source } => {
                write!(f, "connect {} failed: {source}", path.display())
            }
            Self::Io(err) => err.fmt(f),
            Self::Timeout { operation, timeout } => {
                write!(f, "{operation} timed out after {}s", timeout.as_secs())
            }
            Self::Parse(err) => write!(f, "parse rpc payload failed: {err}"),
            Self::Rpc(err) => write!(f, "rpc error {}: {}", err.code, err.message),
            Self::Protocol(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ControlClientError {}

pub struct ControlConnection {
    reader: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    next_id: u64,
}

#[derive(Deserialize)]
struct RpcEnvelope {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcFailure>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
}

impl ControlConnection {
    pub async fn connect(path: &Path, timeout: Duration) -> Result<Self, ControlClientError> {
        let stream = match tokio::time::timeout(timeout, UnixStream::connect(path)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(source)) => {
                return Err(ControlClientError::Connect {
                    path: path.to_path_buf(),
                    source,
                })
            }
            Err(_) => {
                return Err(ControlClientError::Timeout {
                    operation: "connect",
                    timeout,
                })
            }
        };
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read_half).lines(),
            writer: write_half,
            next_id: 1,
        })
    }

    pub async fn call<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<T, ControlClientError> {
        let value = self.call_value(method, params, timeout).await?;
        serde_json::from_value(value).map_err(ControlClientError::Parse)
    }

    pub async fn call_value(
        &mut self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, ControlClientError> {
        let id = self.next_request_id();
        self.send_request(method, params, id).await?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ControlClientError::Timeout {
                    operation: "read rpc response",
                    timeout,
                });
            }
            let message = self.read_message(Some(remaining)).await?;
            if message
                .get("method")
                .and_then(Value::as_str)
                .is_some_and(|method| method == "event")
            {
                continue;
            }
            return parse_result(message, id);
        }
    }

    pub async fn subscribe_events(
        &mut self,
        timeout: Duration,
    ) -> Result<Value, ControlClientError> {
        self.call_value("events.stream", None, timeout).await
    }

    pub async fn methods(
        &mut self,
        timeout: Duration,
    ) -> Result<ControlPlaneMethods, ControlClientError> {
        self.call("status.methods", None, timeout).await
    }

    pub async fn capabilities(
        &mut self,
        timeout: Duration,
    ) -> Result<MinerCapabilities, ControlClientError> {
        self.call("status.capabilities", None, timeout).await
    }

    pub async fn status(&mut self, timeout: Duration) -> Result<MinerSnapshot, ControlClientError> {
        self.call("status.get", None, timeout).await
    }

    pub async fn events_since(
        &mut self,
        since_seq: u64,
        timeout: Duration,
    ) -> Result<EventsSinceResponse, ControlClientError> {
        self.call(
            "events.since",
            Some(serde_json::json!({ "since_seq": since_seq })),
            timeout,
        )
        .await
    }

    pub async fn raw(
        &mut self,
        request: Value,
        timeout: Duration,
    ) -> Result<Value, ControlClientError> {
        self.raw_send(&request).await?;
        self.read_message(Some(timeout)).await
    }

    pub async fn send_request(
        &mut self,
        method: &str,
        params: Option<Value>,
        id: u64,
    ) -> Result<(), ControlClientError> {
        let mut request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(params) = params {
            request["params"] = params;
        }
        self.raw_send(&request).await
    }

    pub async fn read_message(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<Value, ControlClientError> {
        let future = self.reader.next_line();
        let line = match timeout {
            Some(timeout) => with_timeout("read rpc response", timeout, future).await,
            None => future.await.map_err(ControlClientError::Io),
        }?
        .ok_or_else(|| ControlClientError::Protocol("socket closed".to_string()))?;
        serde_json::from_str(&line).map_err(ControlClientError::Parse)
    }

    pub async fn read_event(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<MinerEvent, ControlClientError> {
        let message = self.read_message(timeout).await?;
        let envelope: RpcEnvelope =
            serde_json::from_value(message).map_err(ControlClientError::Parse)?;
        if envelope.method.as_deref() != Some("event") {
            return Err(ControlClientError::Protocol(
                "expected event notification".to_string(),
            ));
        }
        let params = envelope.params.ok_or_else(|| {
            ControlClientError::Protocol("event notification missing params".to_string())
        })?;
        serde_json::from_value(params).map_err(ControlClientError::Parse)
    }

    async fn raw_send(&mut self, request: &Value) -> Result<(), ControlClientError> {
        let mut encoded = serde_json::to_vec(request).map_err(ControlClientError::Parse)?;
        encoded.push(b'\n');
        self.writer
            .write_all(&encoded)
            .await
            .map_err(ControlClientError::Io)?;
        self.writer.flush().await.map_err(ControlClientError::Io)
    }

    fn next_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

fn parse_result(message: Value, expected_id: u64) -> Result<Value, ControlClientError> {
    let envelope: RpcEnvelope =
        serde_json::from_value(message).map_err(ControlClientError::Parse)?;
    let response_id = envelope
        .id
        .as_ref()
        .and_then(Value::as_u64)
        .ok_or_else(|| ControlClientError::Protocol("rpc response missing id".to_string()))?;
    if response_id != expected_id {
        return Err(ControlClientError::Protocol(format!(
            "rpc response id mismatch: expected {expected_id}, got {response_id}"
        )));
    }
    if let Some(error) = envelope.error {
        return Err(ControlClientError::Rpc(error));
    }
    envelope
        .result
        .ok_or_else(|| ControlClientError::Protocol("rpc response missing result".to_string()))
}

async fn with_timeout<F, T>(
    operation: &'static str,
    timeout: Duration,
    future: F,
) -> Result<T, ControlClientError>
where
    F: std::future::Future<Output = Result<T, std::io::Error>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| ControlClientError::Timeout { operation, timeout })?
        .map_err(ControlClientError::Io)
}
