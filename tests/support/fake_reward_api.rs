use anyhow::{Context, Result};
use serde_json::Value;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct FakeRewardApi {
    _task: JoinHandle<()>,
    addr: SocketAddr,
    last_request_path: Arc<Mutex<Option<String>>>,
}

fn pick_free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

impl FakeRewardApi {
    pub async fn start_json(body: Value) -> Result<Self> {
        Self::start_response("200 OK", &serde_json::to_string(&body)?, "application/json").await
    }

    pub async fn start_response(status: &str, body: &str, content_type: &str) -> Result<Self> {
        let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), pick_free_port()?);
        let listener = TcpListener::bind(addr)
            .await
            .context("bind fake reward api failed")?;
        let status = status.to_string();
        let body = body.to_string();
        let content_type = content_type.to_string();
        let last_request_path = Arc::new(Mutex::new(None));
        let path_cell = Arc::clone(&last_request_path);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let status = status.clone();
                let body = body.clone();
                let content_type = content_type.clone();
                let path_cell = Arc::clone(&path_cell);
                tokio::spawn(async move {
                    let _ =
                        handle_connection(stream, &status, &body, &content_type, path_cell).await;
                });
            }
        });
        Ok(Self {
            _task: task,
            addr,
            last_request_path,
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn last_request_path(&self) -> Option<String> {
        self.last_request_path
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    status: &str,
    body: &str,
    content_type: &str,
    last_request_path: Arc<Mutex<Option<String>>>,
) -> Result<()> {
    let mut buffer = vec![0_u8; 4096];
    let mut total = 0usize;
    loop {
        let read = stream.read(&mut buffer[total..]).await?;
        if read == 0 {
            break;
        }
        total += read;
        if total >= 4 && buffer[..total].windows(4).any(|value| value == b"\r\n\r\n") {
            break;
        }
        if total == buffer.len() {
            buffer.resize(buffer.len() * 2, 0);
        }
    }
    let request = String::from_utf8_lossy(&buffer[..total]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    if let Ok(mut slot) = last_request_path.lock() {
        *slot = Some(path);
    }
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}
