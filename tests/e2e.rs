mod support;

use anyhow::{Context, Result};
use starcoin_types::genesis_config::ConsensusStrategy;
use std::net::{Ipv4Addr, SocketAddr};
use std::process::{Command, Stdio};
use std::time::Duration;
use support::fake_pool::{DisconnectOncePool, SilentKeepalivePool};
use support::mock_rpc::{build_mint_event, MockMiningRpc};
use support::process::{
    pick_free_port, resolve_cpu_miner_bin, wait_for_child_exit, wait_for_submit_count,
    StratumdProcess, TEST_MUTEX,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cpu_miner_submits_share_to_current_stratumd() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let mock = MockMiningRpc::start(build_mint_event(1, 1, ConsensusStrategy::Dummy))?;
    let listen = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), pick_free_port()?);
    let _stratumd = StratumdProcess::spawn(listen, &mock.ws_url()).await?;

    let miner_bin = resolve_cpu_miner_bin()?;
    let login = "0xd820b199fbaf1bc5e68eb1c511c2c3d1.worker1";
    let status = Command::new(miner_bin)
        .arg("--pool")
        .arg(listen.to_string())
        .arg("--consensus-strategy")
        .arg("dummy")
        .arg("--login")
        .arg(login)
        .arg("--pass")
        .arg("x")
        .arg("--threads")
        .arg("1")
        .arg("--exit-after-accepted")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run cpu miner failed")?;
    assert!(
        status.success(),
        "cpu miner should exit successfully: {}",
        status
    );

    let calls = mock.submit_calls()?;
    assert!(
        !calls.is_empty(),
        "cpu miner should submit at least one candidate upstream"
    );
    let call = &calls[0];
    assert!(!call.extra.is_empty());
    assert!(!call.minting_blob.is_empty());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cpu_miner_submits_keccak_share_to_current_stratumd() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let mock = MockMiningRpc::start(build_mint_event(1, 1, ConsensusStrategy::Keccak))?;
    let listen = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), pick_free_port()?);
    let _stratumd = StratumdProcess::spawn(listen, &mock.ws_url()).await?;

    let miner_bin = resolve_cpu_miner_bin()?;
    let login = "0xd820b199fbaf1bc5e68eb1c511c2c3d1.worker-keccak";
    let status = Command::new(miner_bin)
        .arg("--pool")
        .arg(listen.to_string())
        .arg("--consensus-strategy")
        .arg("keccak")
        .arg("--login")
        .arg(login)
        .arg("--pass")
        .arg("x")
        .arg("--threads")
        .arg("1")
        .arg("--exit-after-accepted")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run cpu miner failed")?;
    assert!(
        status.success(),
        "cpu miner should exit successfully on keccak: {}",
        status
    );

    let calls = mock.submit_calls()?;
    assert!(
        !calls.is_empty(),
        "cpu miner should submit at least one keccak candidate upstream"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cpu_miner_retries_until_stratumd_is_available() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let mock = MockMiningRpc::start(build_mint_event(1, 1, ConsensusStrategy::Dummy))?;
    let listen = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), pick_free_port()?);
    let miner_bin = resolve_cpu_miner_bin()?;
    let login = "0xd820b199fbaf1bc5e68eb1c511c2c3d1.worker-retry";

    let mut miner = Command::new(miner_bin)
        .arg("--pool")
        .arg(listen.to_string())
        .arg("--consensus-strategy")
        .arg("dummy")
        .arg("--login")
        .arg(login)
        .arg("--pass")
        .arg("x")
        .arg("--threads")
        .arg("1")
        .arg("--exit-after-accepted")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn cpu miner failed")?;

    tokio::time::sleep(Duration::from_millis(1200)).await;
    let _stratumd = StratumdProcess::spawn(listen, &mock.ws_url()).await?;

    let status = wait_for_child_exit(&mut miner, Duration::from_secs(20)).await?;
    assert!(
        status.success(),
        "cpu miner should recover and exit successfully: {}",
        status
    );

    wait_for_submit_count(|| Ok(mock.submit_calls()?.len()), 1, Duration::from_secs(5)).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cpu_miner_recovers_after_connected_stratumd_restart() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let mock = MockMiningRpc::start(build_mint_event(1, 1, ConsensusStrategy::Dummy))?;
    let listen = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), pick_free_port()?);
    let mut stratumd = Some(StratumdProcess::spawn(listen, &mock.ws_url()).await?);
    let miner_bin = resolve_cpu_miner_bin()?;
    let login = "0xd820b199fbaf1bc5e68eb1c511c2c3d1.worker-restart";

    let mut miner = Command::new(miner_bin)
        .arg("--pool")
        .arg(listen.to_string())
        .arg("--consensus-strategy")
        .arg("dummy")
        .arg("--login")
        .arg(login)
        .arg("--pass")
        .arg("x")
        .arg("--threads")
        .arg("1")
        .arg("--exit-after-accepted")
        .arg("2")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn cpu miner failed")?;

    wait_for_submit_count(
        || Ok(mock.submit_calls()?.len()),
        1,
        Duration::from_secs(15),
    )
    .await?;
    drop(stratumd.take());

    tokio::time::sleep(Duration::from_millis(1200)).await;
    stratumd = Some(StratumdProcess::spawn(listen, &mock.ws_url()).await?);

    let status = wait_for_child_exit(&mut miner, Duration::from_secs(25)).await?;
    assert!(
        status.success(),
        "cpu miner should recover after stratumd restart: {}",
        status
    );
    drop(stratumd);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cpu_miner_reconnects_after_keepalive_timeout() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = SilentKeepalivePool::start().await?;
    let miner_bin = resolve_cpu_miner_bin()?;
    let login = "0xd820b199fbaf1bc5e68eb1c511c2c3d1.worker-silent";

    let status = Command::new(miner_bin)
        .arg("--pool")
        .arg(pool.pool_addr().to_string())
        .arg("--consensus-strategy")
        .arg("keccak")
        .arg("--login")
        .arg(login)
        .arg("--pass")
        .arg("x")
        .arg("--threads")
        .arg("1")
        .arg("--keepalive-interval-secs")
        .arg("1")
        .arg("--exit-after-accepted")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run cpu miner against silent keepalive pool failed")?;

    assert!(
        status.success(),
        "cpu miner should recover after keepalive timeout and reconnect: {}",
        status
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cpu_miner_recovers_after_connected_pool_disconnect() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;

    let pool = DisconnectOncePool::start().await?;
    let miner_bin = resolve_cpu_miner_bin()?;
    let login = "0xd820b199fbaf1bc5e68eb1c511c2c3d1.worker-disconnect";

    let status = Command::new(miner_bin)
        .arg("--pool")
        .arg(pool.pool_addr().to_string())
        .arg("--consensus-strategy")
        .arg("keccak")
        .arg("--login")
        .arg(login)
        .arg("--pass")
        .arg("x")
        .arg("--threads")
        .arg("1")
        .arg("--keepalive-interval-secs")
        .arg("1")
        .arg("--exit-after-accepted")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run cpu miner against disconnect-once pool failed")?;

    assert!(
        status.success(),
        "cpu miner should reconnect after connected pool disconnect: {}",
        status
    );
    Ok(())
}
