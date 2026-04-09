use super::{MinerConfig, MinerEvent, MinerSnapshot, Priority};
use crate::miner::state::RuntimeState;
use starcoin_logger::prelude::*;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "linux")]
use anyhow::Context;

const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(15);
const MIN_KEEPALIVE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(target_os = "linux")]
const DEFAULT_LINUX_BACKGROUND_NICE: i32 = 10;

pub(super) fn next_reconnect_delay(current: Duration) -> Duration {
    std::cmp::min(current.saturating_mul(2), RECONNECT_MAX_DELAY)
}

pub(super) fn keepalive_response_timeout(interval: Duration) -> Duration {
    std::cmp::max(interval.saturating_mul(2), MIN_KEEPALIVE_RESPONSE_TIMEOUT)
}

pub(super) async fn wait_reconnect_or_shutdown(
    shutdown: &CancellationToken,
    delay: Duration,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = shutdown.cancelled() => true,
    }
}

pub(super) fn publish_snapshot(
    state: &RuntimeState,
    config: &MinerConfig,
    started_at: Instant,
    snapshot_tx: &watch::Sender<MinerSnapshot>,
) {
    let snapshot = state.snapshot(
        started_at,
        &config.pool,
        &config.login.worker_name().to_string(),
    );
    let _ = snapshot_tx.send(snapshot);
}

pub(super) fn publish_event(event: MinerEvent, events_tx: &broadcast::Sender<MinerEvent>) {
    let _ = events_tx.send(event);
}

pub(super) fn log_status(state: &RuntimeState, config: &MinerConfig, started_at: Instant) {
    let snapshot = state.snapshot(
        started_at,
        &config.pool,
        &config.login.worker_name().to_string(),
    );
    info!(
        target: "cpu_miner",
        "status state={:?} connected={} accepted={} rejected={} submitted={} reconnects={} uptime_secs={} hashrate={:.2}H/s",
        snapshot.state,
        snapshot.connected,
        snapshot.accepted,
        snapshot.rejected,
        snapshot.submitted,
        snapshot.reconnects,
        snapshot.uptime_secs,
        snapshot.hashrate,
    );
}

#[cfg(target_os = "linux")]
pub(super) fn apply_runtime_priority(priority: Priority) -> anyhow::Result<()> {
    let _ = priority;
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, DEFAULT_LINUX_BACKGROUND_NICE) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()).context("setpriority failed");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn apply_runtime_priority(_priority: Priority) -> anyhow::Result<()> {
    Ok(())
}
