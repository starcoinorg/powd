use anyhow::Result;
use clap::Parser;
use starcoin_cpu_miner::control::{run, ControlPlaneArgs};

#[tokio::main]
async fn main() -> Result<()> {
    let _logger = starcoin_logger::init();
    run(ControlPlaneArgs::parse()).await
}
