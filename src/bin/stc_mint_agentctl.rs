use clap::Parser;
use starcoin_cpu_miner::agent::{run_cli, AgentCliArgs};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    run_cli(AgentCliArgs::parse()).await
}
