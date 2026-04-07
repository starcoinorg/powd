mod cli;
mod client;
mod config;
mod dashboard;
mod mcp;
mod render;
mod rpc;
mod state;
mod wallet;
mod wallet_support;

pub use cli::{run_cli, AgentCliArgs};
pub use client::{AgentClientError, AgentConnection, RpcFailure};
pub use config::{default_socket_path, AgentArgs};

use anyhow::{Context, Result};
use config::{prepare_socket_path, restrict_socket_permissions};
use rpc::serve_connection;
use state::SharedState;
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

pub async fn run(args: AgentArgs) -> Result<()> {
    let config = args.into_config()?;
    prepare_socket_path(&config.socket_path)?;
    let runner = crate::MinerRunner::new(config.miner_config.clone())?;
    let state = SharedState::new(config.miner_config, runner, config.initial_budget);
    let shutdown = CancellationToken::new();
    let shutdown_listener = shutdown.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        shutdown_listener.cancel();
    });
    let listener = UnixListener::bind(&config.socket_path)
        .with_context(|| format!("bind unix socket {}", config.socket_path.display()))?;
    restrict_socket_permissions(&config.socket_path)?;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _) = accept?;
                let state = state.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    if let Err(err) = serve_connection(stream, state, shutdown).await {
                        starcoin_logger::prelude::warn!(target: "stc_mint_agent", "connection failed: {err}");
                    }
                });
            }
        }
    }
    state.stop_on_shutdown().await;
    let _ = tokio::fs::remove_file(&config.socket_path).await;
    Ok(())
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigint = signal(SignalKind::interrupt()).expect("register SIGINT handler");
        let mut sigterm = signal(SignalKind::terminate()).expect("register SIGTERM handler");
        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
