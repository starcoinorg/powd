use clap::Parser;
use powd::agent::{run_cli, AgentCliArgs};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    run_cli(AgentCliArgs::parse()).await
}
