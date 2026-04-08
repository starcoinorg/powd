use anyhow::Result;
use clap::Parser;
use powd::agent::{run, run_cli, AgentArgs, AgentCliArgs};
use std::ffi::OsStr;
use std::process::ExitCode;

const INTERNAL_DAEMON_MODE: &str = "__daemon";

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
    if args
        .get(1)
        .is_some_and(|arg| arg == OsStr::new(INTERNAL_DAEMON_MODE))
    {
        let daemon_args = std::iter::once(args[0].clone()).chain(args.into_iter().skip(2));
        run(AgentArgs::parse_from(daemon_args)).await?;
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(run_cli(AgentCliArgs::parse_from(args)).await)
    }
}
