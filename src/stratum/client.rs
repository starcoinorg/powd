use crate::protocol::codec::JsonStreamCodec;
use crate::protocol::stratum_rpc::{
    LoginRequest, ShareRequest, Status, StratumJob, StratumJobResponse,
};
use crate::types::WorkerId;
use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

const MAX_PENDING_REQUESTS: usize = 64;
const MAX_PENDING_REQUEST_AGE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy)]
enum PendingKind {
    Submit,
    Keepalive,
}

#[derive(Debug, Clone, Copy)]
struct PendingRequest {
    kind: PendingKind,
    queued_at: Instant,
}

#[derive(Debug)]
pub enum ClientEvent {
    Job(Box<StratumJobResponse>),
    SubmitAccepted,
    SubmitRejected(String),
    KeepaliveOk,
}

#[derive(Debug)]
pub enum LoginError {
    Retryable(anyhow::Error),
    Permanent(anyhow::Error),
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retryable(err) | Self::Permanent(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for LoginError {}

#[derive(Debug)]
pub struct StratumClient {
    framed: Framed<TcpStream, JsonStreamCodec>,
    next_id: u64,
    pending: BTreeMap<u64, PendingRequest>,
}

impl StratumClient {
    pub async fn connect(pool: &str) -> Result<Self> {
        let stream = TcpStream::connect(pool)
            .await
            .with_context(|| format!("connect to pool {pool} failed"))?;
        Ok(Self {
            framed: Framed::new(stream, JsonStreamCodec::stream_incoming()),
            next_id: 1,
            pending: BTreeMap::new(),
        })
    }

    pub async fn login(
        &mut self,
        request: LoginRequest,
    ) -> std::result::Result<StratumJobResponse, LoginError> {
        let id = self.next_request_id();
        self.send_request(id, "login", &request)
            .await
            .map_err(LoginError::Retryable)?;

        loop {
            let value = self.read_value().await.map_err(LoginError::Retryable)?;
            if !response_id_matches(&value, id) {
                continue;
            }
            if let Some(err) = extract_error_message(&value) {
                return Err(LoginError::Permanent(anyhow::anyhow!(
                    "login failed: {err}"
                )));
            }
            if let Some(job) = extract_job(&value) {
                return Ok(job);
            }
            return Err(LoginError::Permanent(anyhow::anyhow!(
                "login response missing initial job"
            )));
        }
    }

    pub async fn submit_share(&mut self, share: ShareRequest) -> Result<()> {
        let id = self.next_request_id();
        self.insert_pending(id, PendingKind::Submit);
        self.send_request(id, "submit", &share).await
    }

    pub async fn send_keepalive(&mut self, worker_id: &WorkerId) -> Result<()> {
        let id = self.next_request_id();
        self.insert_pending(id, PendingKind::Keepalive);
        self.send_request(id, "keepalived", &json!({ "id": worker_id.as_str() }))
            .await
    }

    pub async fn next_event(&mut self) -> Result<ClientEvent> {
        loop {
            self.prune_pending();
            let value = self.read_value().await?;
            if let Some(job) = extract_job(&value) {
                return Ok(ClientEvent::Job(Box::new(job)));
            }
            let Some(id) = response_id(&value) else {
                continue;
            };
            let Some(request) = self.pending.remove(&id) else {
                continue;
            };
            match request.kind {
                PendingKind::Submit => return parse_submit_response(&value),
                PendingKind::Keepalive => return parse_keepalive_response(&value),
            }
        }
    }

    fn next_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn insert_pending(&mut self, id: u64, kind: PendingKind) {
        self.prune_pending();
        while self.pending.len() >= MAX_PENDING_REQUESTS {
            self.pending.pop_first();
        }
        self.pending.insert(
            id,
            PendingRequest {
                kind,
                queued_at: Instant::now(),
            },
        );
    }

    fn prune_pending(&mut self) {
        let now = Instant::now();
        while self.pending.first_key_value().is_some_and(|(_, request)| {
            now.duration_since(request.queued_at) >= MAX_PENDING_REQUEST_AGE
        }) {
            self.pending.pop_first();
        }
    }

    async fn send_request<T: Serialize>(
        &mut self,
        id: u64,
        method: &str,
        params: &T,
    ) -> Result<()> {
        let request = json!({
            "id": id,
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.framed.send(request.to_string()).await?;
        Ok(())
    }

    async fn read_value(&mut self) -> Result<Value> {
        let line = self
            .framed
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("stratum connection closed"))??;
        serde_json::from_str(&line).context("parse stratum json failed")
    }
}

fn parse_submit_response(value: &Value) -> Result<ClientEvent> {
    if let Some(err) = extract_error_message(value) {
        return Ok(ClientEvent::SubmitRejected(err));
    }
    let accepted: Status = serde_json::from_value(
        value
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("submit response missing result"))?,
    )?;
    if accepted.status == "OK" {
        return Ok(ClientEvent::SubmitAccepted);
    }
    Ok(ClientEvent::SubmitRejected(format!(
        "unexpected submit status: {status}",
        status = accepted.status
    )))
}

fn parse_keepalive_response(value: &Value) -> Result<ClientEvent> {
    if let Some(err) = extract_error_message(value) {
        return Err(anyhow::anyhow!("keepalive failed: {err}"));
    }
    let keepalive: Status = serde_json::from_value(
        value
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("keepalive response missing result"))?,
    )?;
    if keepalive.status != "KEEPALIVED" {
        return Err(anyhow::anyhow!(
            "unexpected keepalive status: {status}",
            status = keepalive.status
        ));
    }
    Ok(ClientEvent::KeepaliveOk)
}

fn extract_job(value: &Value) -> Option<StratumJobResponse> {
    if let Some(result) = value.get("result") {
        if result.get("job").is_some() {
            return serde_json::from_value(result.clone()).ok();
        }
    }
    if value.get("method").and_then(|m| m.as_str()) == Some("job") {
        if let Some(params) = value.get("params") {
            if let Ok(job) = serde_json::from_value::<StratumJob>(params.clone()) {
                return Some(StratumJobResponse {
                    login: None,
                    id: job.id.clone(),
                    status: "OK".to_string(),
                    job,
                });
            }
        }
    }
    None
}

fn response_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(|id| match id {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse::<u64>().ok(),
        _ => None,
    })
}

fn response_id_matches(value: &Value, request_id: u64) -> bool {
    response_id(value) == Some(request_id)
}

fn extract_error_message(value: &Value) -> Option<String> {
    value
        .get("error")
        .and_then(|err| err.get("message"))
        .and_then(|message| message.as_str())
        .map(str::to_owned)
}
