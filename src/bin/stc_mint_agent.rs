use anyhow::Result;
use clap::Parser;
use starcoin_cpu_miner::agent::{run, AgentArgs};

#[tokio::main]
async fn main() -> Result<()> {
    let _logger = starcoin_logger::init();
    run(AgentArgs::parse()).await
}
