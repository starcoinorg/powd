#![allow(dead_code)]

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use powd::difficulty_to_target_hex;
use powd::protocol::codec::JsonStreamCodec;
use powd::protocol::stratum_rpc::StratumJob;
use serde_json::json;
use starcoin_types::U256;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::codec::Framed;

use super::process::pick_free_port;

pub struct SilentKeepalivePool {
    _task: JoinHandle<()>,
    addr: SocketAddr,
}

pub struct DisconnectOncePool {
    _task: JoinHandle<()>,
    addr: SocketAddr,
}

impl SilentKeepalivePool {
    pub async fn start() -> Result<Self> {
        let (task, addr) =
            spawn_fake_pool("silent keepalive", handle_silent_keepalive_connection).await?;
        Ok(Self { _task: task, addr })
    }

    pub fn pool_addr(&self) -> SocketAddr {
        self.addr
    }
}

impl DisconnectOncePool {
    pub async fn start() -> Result<Self> {
        let (task, addr) =
            spawn_fake_pool("disconnect-once", handle_disconnect_once_connection).await?;
        Ok(Self { _task: task, addr })
    }

    pub fn pool_addr(&self) -> SocketAddr {
        self.addr
    }
}

async fn spawn_fake_pool<F, Fut>(label: &str, handler: F) -> Result<(JoinHandle<()>, SocketAddr)>
where
    F: Fn(TcpStream, usize) -> Fut + Send + Sync + 'static + Copy,
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), pick_free_port()?);
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {label} pool failed"))?;
    let connection_seq = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn({
        let connection_seq = Arc::clone(&connection_seq);
        async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seq = connection_seq.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let _ = handler(stream, seq).await;
                });
            }
        }
    });
    Ok((task, addr))
}

async fn handle_silent_keepalive_connection(stream: TcpStream, seq: usize) -> Result<()> {
    let mut framed = Framed::new(stream, JsonStreamCodec::stream_incoming());
    while let Some(line) = framed.next().await {
        let line = line.context("read fake pool frame failed")?;
        let value: serde_json::Value =
            serde_json::from_str(&line).context("parse fake pool request failed")?;
        let method = value
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
        match method {
            "login" => {
                let login = value
                    .get("params")
                    .and_then(|params| params.get("login"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("0x0.worker");
                let worker_id = login.split('.').nth(1).unwrap_or("worker");
                let job = if seq == 0 {
                    build_stratum_job(worker_id, "stall-job", U256::from(u64::MAX), 1)
                } else {
                    build_stratum_job(worker_id, "resume-job", U256::from(1u64), 2)
                };
                framed
                    .send(
                        json!({
                            "id": id,
                            "jsonrpc": "2.0",
                            "result": { "id": worker_id, "status": "OK", "job": job }
                        })
                        .to_string(),
                    )
                    .await
                    .context("send fake login response failed")?;
            }
            "keepalived" => {
                if seq != 0 {
                    framed
                        .send(
                            json!({
                                "id": id,
                                "jsonrpc": "2.0",
                                "result": { "status": "KEEPALIVED" }
                            })
                            .to_string(),
                        )
                        .await
                        .context("send fake keepalive response failed")?;
                }
            }
            "submit" => {
                if seq != 0 {
                    framed
                        .send(
                            json!({
                                "id": id,
                                "jsonrpc": "2.0",
                                "result": { "status": "OK" }
                            })
                            .to_string(),
                        )
                        .await
                        .context("send fake submit response failed")?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn handle_disconnect_once_connection(stream: TcpStream, seq: usize) -> Result<()> {
    let mut framed = Framed::new(stream, JsonStreamCodec::stream_incoming());
    while let Some(line) = framed.next().await {
        let line = line.context("read disconnect-once pool frame failed")?;
        let value: serde_json::Value =
            serde_json::from_str(&line).context("parse disconnect-once pool request failed")?;
        let method = value
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
        match method {
            "login" => {
                let login = value
                    .get("params")
                    .and_then(|params| params.get("login"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("0x0.worker");
                let worker_id = login.split('.').nth(1).unwrap_or("worker");
                let difficulty = if seq == 0 {
                    U256::from(u64::MAX)
                } else {
                    U256::from(1u64)
                };
                let job_id = if seq == 0 {
                    "disconnect-job"
                } else {
                    "resume-job"
                };
                let height = if seq == 0 { 1 } else { 2 };
                let job = build_stratum_job(worker_id, job_id, difficulty, height);
                framed
                    .send(
                        json!({
                            "id": id,
                            "jsonrpc": "2.0",
                            "result": { "id": worker_id, "status": "OK", "job": job }
                        })
                        .to_string(),
                    )
                    .await
                    .context("send disconnect-once login response failed")?;
                if seq == 0 {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    break;
                }
            }
            "keepalived" => {
                if seq != 0 {
                    framed
                        .send(
                            json!({
                                "id": id,
                                "jsonrpc": "2.0",
                                "result": { "status": "KEEPALIVED" }
                            })
                            .to_string(),
                        )
                        .await
                        .context("send disconnect-once keepalive response failed")?;
                }
            }
            "submit" => {
                if seq != 0 {
                    framed
                        .send(
                            json!({
                                "id": id,
                                "jsonrpc": "2.0",
                                "result": { "status": "OK" }
                            })
                            .to_string(),
                        )
                        .await
                        .context("send disconnect-once submit response failed")?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn build_stratum_job(worker_id: &str, job_id: &str, difficulty: U256, height: u64) -> StratumJob {
    let mut blob = vec![0u8; 76];
    blob[35..39].copy_from_slice(&[0, 0, 0, 0]);
    StratumJob {
        height,
        id: worker_id.to_string(),
        target: difficulty_to_target_hex(difficulty),
        job_id: job_id.to_string(),
        blob: hex::encode(blob),
    }
}
