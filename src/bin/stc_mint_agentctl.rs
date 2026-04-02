use clap::Parser;
use starcoin_cpu_miner::control::{run_cli, ControlCliArgs};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    run_cli(ControlCliArgs::parse()).await
}
