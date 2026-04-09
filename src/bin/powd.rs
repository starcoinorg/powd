use anyhow::Result;
use clap::Parser;
use powd::agent::{run, run_cli, AgentArgs, AgentCliArgs};
use std::process::ExitCode;

const INTERNAL_DAEMON_ENV: &str = "POWD_INTERNAL_DAEMON";

#[tokio::main]
async fn main() -> ExitCode {
    let _logger = starcoin_logger::init();
    match dispatch().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::from(1)
        }
    }
}

async fn dispatch() -> Result<ExitCode> {
    let args = std::env::args_os().collect::<Vec<_>>();
    if std::env::var_os(INTERNAL_DAEMON_ENV).is_some() {
        run(AgentArgs::parse_from(args)).await?;
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(run_cli(AgentCliArgs::parse_from(args)).await)
    }
}
