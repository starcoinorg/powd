use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use starcoin_cpu_miner::{Budget, MinerConfig, MinerRunner, Priority, StratumLogin};
use starcoin_types::genesis_config::ConsensusStrategy;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Parser, Debug)]
#[command(
    name = "stc-mint-miner",
    about = "Direct CPU miner that connects to a Stratum pool without the local API"
)]
struct Cli {
    #[arg(
        long,
        help = "Stratum pool endpoint, for example main-stratum.starcoin.org:9888"
    )]
    pool: String,
    #[arg(long, help = "Stratum login in wallet.worker form")]
    login: String,
    #[arg(long, default_value = "x", help = "Stratum password field")]
    pass: String,
    #[arg(
        long,
        default_value = "stc-mint-miner",
        help = "Agent string sent during login"
    )]
    agent: String,
    #[arg(long, help = "Worker thread count; defaults to half the logical CPUs")]
    threads: Option<usize>,
    #[arg(
        long,
        value_enum,
        default_value_t = CliConsensusStrategy::Cryptonight,
        help = "Consensus strategy used to solve shares"
    )]
    consensus_strategy: CliConsensusStrategy,
    #[arg(long, default_value_t = 30, help = "Keepalive interval in seconds")]
    keepalive_interval_secs: u64,
    #[arg(long, default_value_t = 10, help = "Status log interval in seconds")]
    status_interval_secs: u64,
    #[arg(long, help = "Exit after this many accepted shares")]
    exit_after_accepted: Option<u64>,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum CliConsensusStrategy {
    Dummy,
    Argon,
    Keccak,
    Cryptonight,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _logger = starcoin_logger::init();
    let cli = Cli::parse();
    let login: StratumLogin = cli.login.parse().with_context(|| "parse --login failed")?;
    let max_threads = parse_max_threads(cli.threads.unwrap_or_else(default_threads))?;
    let runner = MinerRunner::new(MinerConfig {
        pool: cli.pool,
        login,
        pass: cli.pass,
        agent: cli.agent,
        max_threads,
        strategy: cli.consensus_strategy.into(),
        keepalive_interval: Duration::from_secs(cli.keepalive_interval_secs),
        status_interval: Duration::from_secs(cli.status_interval_secs),
        exit_after_accepted: cli.exit_after_accepted,
    })?;
    let shutdown = CancellationToken::new();
    let shutdown_listener = shutdown.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        shutdown_listener.cancel();
    });
    let initial_budget = Budget {
        threads: max_threads,
        cpu_percent: 100,
        priority: Priority::Background,
    };
    runner.run_until_shutdown(initial_budget, shutdown).await?;
    Ok(())
}

fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| usize::max(1, parallelism.get() / 2))
        .unwrap_or(1)
}

fn parse_max_threads(threads: usize) -> Result<u16> {
    u16::try_from(threads.max(1)).context("threads exceed u16 range")
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

impl From<CliConsensusStrategy> for ConsensusStrategy {
    fn from(value: CliConsensusStrategy) -> Self {
        match value {
            CliConsensusStrategy::Dummy => ConsensusStrategy::Dummy,
            CliConsensusStrategy::Argon => ConsensusStrategy::Argon,
            CliConsensusStrategy::Keccak => ConsensusStrategy::Keccak,
            CliConsensusStrategy::Cryptonight => ConsensusStrategy::CryptoNight,
        }
    }
}
