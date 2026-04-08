use super::runtime_support::{
    keepalive_response_timeout, log_status, next_reconnect_delay, publish_event, publish_snapshot,
    wait_reconnect_or_shutdown,
};
use super::solver::{SolverPool, SolverPoolGuard};
use super::state::{ConnectFailure, ConnectState, InflightSubmit, RuntimeState};
use super::{
    AgentError, AutoState, Budget, BudgetMode, MinerCapabilities, MinerConfig, MinerEvent,
    MinerSnapshot, MinerState, RunnerError,
};
use crate::mining::job::{MiningJob, SolvedShare};
use crate::protocol::stratum_rpc::LoginRequest;
use crate::stratum::client::{ClientEvent, LoginError, StratumClient};
use anyhow::Context;
use starcoin_logger::prelude::warn;
use starcoin_types::genesis_config::ConsensusStrategy;
use std::sync::{atomic::AtomicU64, Arc};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);
const SHARE_CHANNEL_LIMIT: usize = 128;
const CONSENSUS_RECHECK_REJECT_THRESHOLD: u64 = 5;
const CONTROL_TRANSITION_TIMEOUT: Duration = Duration::from_secs(2);

struct RuntimeCtx<'a> {
    config: &'a MinerConfig,
    started_at: Instant,
    worker_name: &'a str,
    snapshot_tx: &'a watch::Sender<MinerSnapshot>,
    events_tx: &'a broadcast::Sender<MinerEvent>,
}

pub struct MinerRunner {
    config: MinerConfig,
}

#[derive(Clone)]
pub struct MinerHandle {
    commands: mpsc::UnboundedSender<RuntimeCommand>,
    snapshot_rx: watch::Receiver<MinerSnapshot>,
    events_tx: broadcast::Sender<MinerEvent>,
    finished: watch::Receiver<Option<std::result::Result<(), RunnerError>>>,
    capabilities: MinerCapabilities,
    shutdown: CancellationToken,
}

#[derive(Debug)]
enum EventOutcome {
    Continue,
    RecheckConsensus,
}

#[derive(Debug)]
enum RuntimeCommand {
    Pause,
    Resume,
    SetBudget(Budget),
    Stop,
}

impl RuntimeCtx<'_> {
    fn snapshot(&self, state: &RuntimeState) -> MinerSnapshot {
        state.snapshot(self.started_at, &self.config.pool, self.worker_name)
    }

    fn publish_snapshot(&self, state: &RuntimeState) {
        publish_snapshot(state, self.config, self.started_at, self.snapshot_tx);
    }

    fn publish_event(&self, event: MinerEvent) {
        publish_event(event, self.events_tx);
    }
}

impl MinerRunner {
    pub fn new(config: MinerConfig) -> std::result::Result<Self, RunnerError> {
        config.validate().map_err(RunnerError::InvalidConfig)?;
        Ok(Self { config })
    }

    pub fn capabilities(&self) -> MinerCapabilities {
        self.config.capabilities()
    }

    pub async fn run_until_shutdown(
        &self,
        initial_budget: Budget,
        shutdown: CancellationToken,
    ) -> std::result::Result<(), RunnerError> {
        let handle = self.spawn(initial_budget)?;
        tokio::select! {
            result = handle.wait_for_termination() => result,
            _ = shutdown.cancelled() => {
                let _ = handle.stop().await;
                handle.wait_for_termination().await
            }
        }
    }

    pub fn spawn(&self, initial_budget: Budget) -> std::result::Result<MinerHandle, RunnerError> {
        let initial_budget = initial_budget
            .validate(self.config.max_threads)
            .map_err(RunnerError::InvalidBudget)?;
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let initial_snapshot = MinerSnapshot {
            state: MinerState::Starting,
            connected: false,
            pool: self.config.pool.clone(),
            worker_name: self.config.login.worker_name().to_string(),
            requested_mode: BudgetMode::Auto,
            effective_budget: initial_budget,
            hashrate: 0.0,
            hashrate_5m: 0.0,
            accepted: 0,
            accepted_5m: 0,
            rejected: 0,
            rejected_5m: 0,
            submitted: 0,
            submitted_5m: 0,
            reject_rate_5m: 0.0,
            reconnects: 0,
            uptime_secs: 0,
            system_cpu_percent: 0.0,
            system_memory_percent: 0.0,
            system_cpu_percent_1m: 0.0,
            system_memory_percent_1m: 0.0,
            auto_state: AutoState::Inactive,
            auto_hold_reason: None,
            last_error: None,
        };
        let (snapshot_tx, snapshot_rx) = watch::channel(initial_snapshot);
        let (events_tx, _) = broadcast::channel(128);
        let (finished_tx, finished_rx) = watch::channel(None);
        let config = self.config.clone();
        let events_tx_task = events_tx.clone();
        let shutdown = CancellationToken::new();
        let runtime_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let result = run_runtime(
                config,
                initial_budget,
                command_rx,
                snapshot_tx,
                events_tx_task,
                runtime_shutdown,
            )
            .await
            .map_err(|err| RunnerError::RuntimeFailed(err.to_string()));
            let _ = finished_tx.send(Some(result));
        });

        Ok(MinerHandle {
            commands: command_tx,
            snapshot_rx,
            events_tx,
            finished: finished_rx,
            capabilities: self.capabilities(),
            shutdown,
        })
    }
}

impl MinerHandle {
    pub fn snapshot(&self) -> MinerSnapshot {
        self.snapshot_rx.borrow().clone()
    }

    pub fn capabilities(&self) -> MinerCapabilities {
        self.capabilities.clone()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<MinerEvent> {
        self.events_tx.subscribe()
    }

    pub async fn pause(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        self.send_command(RuntimeCommand::Pause)?;
        self.wait_for_snapshot(|snapshot| snapshot.state == MinerState::Paused, "pause")
            .await
    }

    pub async fn resume(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        self.send_command(RuntimeCommand::Resume)?;
        self.wait_for_snapshot(
            |snapshot| {
                matches!(
                    snapshot.state,
                    MinerState::Running | MinerState::Reconnecting
                )
            },
            "resume",
        )
        .await
    }

    pub async fn set_budget(
        &self,
        budget: Budget,
    ) -> std::result::Result<MinerSnapshot, AgentError> {
        let budget = budget
            .validate(self.capabilities.max_threads)
            .map_err(AgentError::InvalidBudget)?;
        self.send_command(RuntimeCommand::SetBudget(budget))?;
        self.wait_for_snapshot(
            move |snapshot| snapshot.effective_budget == budget,
            "set_budget",
        )
        .await
    }

    pub async fn set_mode(
        &self,
        mode: BudgetMode,
    ) -> std::result::Result<MinerSnapshot, AgentError> {
        let logical_cpus = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);
        let budget =
            super::default_budget_for_mode(mode, self.capabilities.max_threads, logical_cpus);
        self.set_budget(budget).await
    }

    pub async fn stop(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        self.shutdown.cancel();
        let _ = self.send_command(RuntimeCommand::Stop);
        self.wait_for_termination()
            .await
            .map_err(|err| AgentError::RuntimeFailed(err.to_string()))?;
        Ok(self.snapshot())
    }

    pub async fn wait_for_termination(&self) -> std::result::Result<(), RunnerError> {
        let mut finished = self.finished.clone();
        loop {
            if let Some(result) = finished.borrow().clone() {
                return result;
            }
            if finished.changed().await.is_err() {
                return Err(RunnerError::RuntimeFailed(
                    "miner finished channel closed".to_string(),
                ));
            }
        }
    }

    fn send_command(&self, command: RuntimeCommand) -> std::result::Result<(), AgentError> {
        if self.is_terminal() {
            return Err(AgentError::RuntimeTerminated);
        }
        self.commands
            .send(command)
            .map_err(|_| AgentError::CommandChannelClosed)
    }

    async fn wait_for_snapshot<P>(
        &self,
        predicate: P,
        transition: &'static str,
    ) -> std::result::Result<MinerSnapshot, AgentError>
    where
        P: Fn(&MinerSnapshot) -> bool,
    {
        let mut snapshots = self.snapshot_rx.clone();
        if predicate(&snapshots.borrow()) {
            return Ok(snapshots.borrow().clone());
        }

        let deadline = tokio::time::Instant::now() + CONTROL_TRANSITION_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AgentError::TransitionTimeout(transition));
            }
            tokio::time::timeout(remaining, snapshots.changed())
                .await
                .map_err(|_| AgentError::TransitionTimeout(transition))?
                .map_err(|_| AgentError::RuntimeTerminated)?;
            if predicate(&snapshots.borrow()) {
                return Ok(snapshots.borrow().clone());
            }
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.snapshot_rx.borrow().state,
            MinerState::Stopped | MinerState::Error
        )
    }
}

async fn run_runtime(
    config: MinerConfig,
    initial_budget: Budget,
    mut command_rx: mpsc::UnboundedReceiver<RuntimeCommand>,
    snapshot_tx: watch::Sender<MinerSnapshot>,
    events_tx: broadcast::Sender<MinerEvent>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let (share_tx, mut share_rx) = mpsc::channel::<SolvedShare>(SHARE_CHANNEL_LIMIT);
    let hashes = Arc::new(AtomicU64::new(0));
    let started_at = Instant::now();
    let worker_name = config.login.worker_name().to_string();
    let accepted_goal = config.exit_after_accepted.unwrap_or(u64::MAX);
    let mut keepalive_tick = tokio::time::interval(config.keepalive_interval);
    keepalive_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_tick = tokio::time::interval(config.status_interval);
    status_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut solver = SolverPoolGuard::new(SolverPool::start(
        config.max_threads,
        initial_budget,
        Arc::clone(&hashes),
        share_tx,
    ));
    solver.pool().set_priority(initial_budget.priority);
    let mut state = RuntimeState::new(started_at, initial_budget);
    let ctx = RuntimeCtx {
        config: &config,
        started_at,
        worker_name: &worker_name,
        snapshot_tx: &snapshot_tx,
        events_tx: &events_tx,
    };
    let result = run_loop(
        &ctx,
        solver.pool(),
        &mut state,
        &mut share_rx,
        &mut keepalive_tick,
        &mut status_tick,
        &hashes,
        accepted_goal,
        &shutdown,
        &mut command_rx,
    )
    .await;

    solver.shutdown();
    match result {
        Ok(()) => {
            state.refresh_hashrate(hashes.load(std::sync::atomic::Ordering::Relaxed));
            state.state = MinerState::Stopped;
            ctx.publish_snapshot(&state);
            ctx.publish_event(MinerEvent::Stopped {
                snapshot: ctx.snapshot(&state),
            });
            Ok(())
        }
        Err(err) => {
            state.refresh_hashrate(hashes.load(std::sync::atomic::Ordering::Relaxed));
            state.state = MinerState::Error;
            state.last_error = Some(err.to_string());
            ctx.publish_snapshot(&state);
            ctx.publish_event(MinerEvent::Error {
                snapshot: ctx.snapshot(&state),
                message: err.to_string(),
            });
            Err(err)
        }
    }
}

async fn run_loop(
    ctx: &RuntimeCtx<'_>,
    solver: &SolverPool,
    state: &mut RuntimeState,
    share_rx: &mut mpsc::Receiver<SolvedShare>,
    keepalive_tick: &mut tokio::time::Interval,
    status_tick: &mut tokio::time::Interval,
    hashes: &Arc<AtomicU64>,
    accepted_goal: u64,
    shutdown: &CancellationToken,
    command_rx: &mut mpsc::UnboundedReceiver<RuntimeCommand>,
) -> anyhow::Result<()> {
    ctx.publish_snapshot(state);
    loop {
        if shutdown.is_cancelled() || state.should_stop(accepted_goal) {
            break;
        }

        match ensure_connected(ctx, shutdown, solver, state).await? {
            ConnectState::Ready => {}
            ConnectState::Retry => continue,
            ConnectState::Shutdown => break,
        }
        if state.handle_submit_timeout() || state.handle_keepalive_timeout() {
            ctx.publish_snapshot(state);
            ctx.publish_event(MinerEvent::Reconnecting {
                snapshot: ctx.snapshot(state),
            });
            continue;
        }
        if try_submit_next(state).await {
            ctx.publish_snapshot(state);
            continue;
        }

        tokio::select! {
            _ = shutdown.cancelled() => {
                break;
            }
            Some(command) = command_rx.recv() => {
                if handle_command(command, solver, state, ctx)? {
                    shutdown.cancel();
                }
            }
            event = async {
                let client = state.client.as_mut().expect("client must exist while receiving events");
                client.next_event().await
            }, if state.client.is_some() => {
                match event {
                    Ok(event) => {
                        let outcome = handle_client_event(state, solver, event, ctx)?;
                        if matches!(outcome, EventOutcome::RecheckConsensus) {
                            maybe_abort_on_consensus_switch(ctx.config, state)?;
                        }
                    }
                    Err(err) => {
                        state.reconnects = state.reconnects.saturating_add(1);
                        state.last_error = Some(format!("connection lost: {err}"));
                        state.mark_disconnected();
                        ctx.publish_snapshot(state);
                        ctx.publish_event(MinerEvent::Reconnecting {
                            snapshot: ctx.snapshot(state),
                        });
                        continue;
                    }
                }
            }
            maybe_share = share_rx.recv() => {
                let Some(share) = maybe_share else {
                    break;
                };
                state.queue_share(share);
                ctx.publish_snapshot(state);
            }
            _ = keepalive_tick.tick(), if state.client.is_some() && state.current_job.is_some() => {
                if try_send_keepalive(ctx.config, state).await {
                    ctx.publish_snapshot(state);
                    ctx.publish_event(MinerEvent::Reconnecting {
                        snapshot: ctx.snapshot(state),
                    });
                    continue;
                }
            }
            _ = async {
                if let Some(deadline) = state.pending_keepalive_deadline {
                    tokio::time::sleep_until(deadline).await;
                }
            }, if state.pending_keepalive_deadline.is_some() => {
                state.reconnects = state.reconnects.saturating_add(1);
                state.last_error = Some("keepalive response timeout".to_string());
                state.mark_disconnected();
                ctx.publish_snapshot(state);
                ctx.publish_event(MinerEvent::Reconnecting {
                    snapshot: ctx.snapshot(state),
                });
                continue;
            }
            _ = status_tick.tick() => {
                state.refresh_hashrate(hashes.load(std::sync::atomic::Ordering::Relaxed));
                ctx.publish_snapshot(state);
                log_status(state, ctx.config, ctx.started_at);
            }
        }
    }
    Ok(())
}

fn handle_command(
    command: RuntimeCommand,
    solver: &SolverPool,
    state: &mut RuntimeState,
    ctx: &RuntimeCtx<'_>,
) -> std::result::Result<bool, AgentError> {
    match command {
        RuntimeCommand::Pause => {
            solver.pause();
            state.state = MinerState::Paused;
            ctx.publish_snapshot(state);
            ctx.publish_event(MinerEvent::Paused {
                snapshot: ctx.snapshot(state),
            });
            Ok(false)
        }
        RuntimeCommand::Resume => {
            solver.resume();
            state.state = if state.client.is_some() {
                MinerState::Running
            } else {
                MinerState::Reconnecting
            };
            ctx.publish_snapshot(state);
            ctx.publish_event(MinerEvent::Resumed {
                snapshot: ctx.snapshot(state),
            });
            Ok(false)
        }
        RuntimeCommand::SetBudget(budget) => {
            state.budget = budget;
            solver.apply_budget(budget);
            solver.set_priority(budget.priority);
            ctx.publish_snapshot(state);
            ctx.publish_event(MinerEvent::BudgetChanged {
                snapshot: ctx.snapshot(state),
            });
            Ok(false)
        }
        RuntimeCommand::Stop => Ok(true),
    }
}

async fn ensure_connected(
    ctx: &RuntimeCtx<'_>,
    shutdown: &CancellationToken,
    solver: &SolverPool,
    state: &mut RuntimeState,
) -> anyhow::Result<ConnectState> {
    if state.client.is_some() {
        return Ok(ConnectState::Ready);
    }

    solver.clear_job();
    state.pending_keepalive_deadline = None;
    let strategy = ctx.config.strategy;

    match tokio::select! {
        result = connect_and_login(ctx.config, strategy) => result,
        _ = shutdown.cancelled() => {
            return Ok(ConnectState::Shutdown);
        }
    } {
        Ok((client, first_job)) => {
            state.client = Some(client);
            state.current_job = Some(first_job);
            state.consecutive_rejected = 0;
            state.drop_stale_shares();
            if let Some(job) = state.current_job.as_ref() {
                solver.set_job(job.clone());
            }
            state.reconnect_delay = RECONNECT_BASE_DELAY;
            if matches!(state.state, MinerState::Paused) {
                solver.pause();
            } else {
                solver.resume();
                state.state = MinerState::Running;
            }
            ctx.publish_snapshot(state);
            ctx.publish_event(MinerEvent::Started {
                snapshot: ctx.snapshot(state),
            });
            Ok(ConnectState::Ready)
        }
        Err(ConnectFailure::Retryable(err)) => {
            state.reconnects = state.reconnects.saturating_add(1);
            state.state = MinerState::Reconnecting;
            state.last_error = Some(err.clone());
            ctx.publish_snapshot(state);
            ctx.publish_event(MinerEvent::Reconnecting {
                snapshot: ctx.snapshot(state),
            });
            Ok(wait_or_retry(shutdown, state).await)
        }
        Err(ConnectFailure::Permanent(err)) => Err(anyhow::anyhow!(err)),
    }
}

async fn connect_and_login(
    config: &MinerConfig,
    strategy: ConsensusStrategy,
) -> std::result::Result<(StratumClient, MiningJob), ConnectFailure> {
    let login = LoginRequest {
        login: config.login.to_string(),
        pass: config.pass.clone(),
        agent: config.agent.clone(),
        algo: None,
    };
    let mut client = tokio::time::timeout(CONNECT_TIMEOUT, StratumClient::connect(&config.pool))
        .await
        .map_err(|_| ConnectFailure::Retryable("connect timeout".to_string()))?
        .map_err(|err| ConnectFailure::Retryable(err.to_string()))?;
    let first_job = tokio::time::timeout(LOGIN_TIMEOUT, client.login(login))
        .await
        .map_err(|_| ConnectFailure::Retryable("login timeout".to_string()))?
        .map_err(|err| match err {
            LoginError::Retryable(err) => ConnectFailure::Retryable(err.to_string()),
            LoginError::Permanent(err) => ConnectFailure::Permanent(err.to_string()),
        })?;
    let job = MiningJob::from_response(&first_job, config.login.worker_name(), strategy)
        .map_err(|err| ConnectFailure::Permanent(err.to_string()))?;
    Ok((client, job))
}

async fn try_submit_next(state: &mut RuntimeState) -> bool {
    if state.inflight_submit.is_some() || state.client.is_none() {
        return false;
    }
    let Some(share) = state.pop_next_share() else {
        return false;
    };
    let client = state.client.as_mut().expect("client checked above");
    match client.submit_share(share.clone().into_request()).await {
        Ok(()) => {
            state.record_submitted();
            state.inflight_submit = Some(InflightSubmit {
                share,
                sent_at: Instant::now(),
            });
            false
        }
        Err(err) => {
            warn!(target: "cpu_miner", "send submit failed: {}", err);
            state.last_error = Some(format!("submit send failed: {err}"));
            state.queue_share(share);
            state.mark_disconnected();
            true
        }
    }
}

async fn try_send_keepalive(config: &MinerConfig, state: &mut RuntimeState) -> bool {
    let (Some(client), Some(job)) = (state.client.as_mut(), state.current_job.as_ref()) else {
        return false;
    };
    if state.pending_keepalive_deadline.is_some() {
        return false;
    }
    if let Err(err) = client.send_keepalive(&job.worker_id).await {
        state.reconnects = state.reconnects.saturating_add(1);
        state.last_error = Some(format!("keepalive failed: {err}"));
        state.mark_disconnected();
        return true;
    }
    state.pending_keepalive_deadline =
        Some(tokio::time::Instant::now() + keepalive_response_timeout(config.keepalive_interval));
    false
}

fn handle_client_event(
    state: &mut RuntimeState,
    solver: &SolverPool,
    event: ClientEvent,
    ctx: &RuntimeCtx<'_>,
) -> anyhow::Result<EventOutcome> {
    match event {
        ClientEvent::Job(job) => {
            let strategy = state
                .current_job
                .as_ref()
                .map(|job| job.strategy)
                .context("missing strategy for refreshed job")?;
            let worker_name = state
                .current_job
                .as_ref()
                .map(|job| job.worker_name.clone())
                .context("missing worker_name for refreshed job")?;
            state.current_job = Some(MiningJob::from_response(&job, &worker_name, strategy)?);
            if let Some(job) = state.current_job.clone() {
                state.drop_stale_shares();
                solver.set_job(job);
            }
            ctx.publish_snapshot(state);
            Ok(EventOutcome::Continue)
        }
        ClientEvent::SubmitAccepted => {
            state.inflight_submit = None;
            state.record_accepted();
            state.consecutive_rejected = 0;
            ctx.publish_snapshot(state);
            ctx.publish_event(MinerEvent::ShareAccepted {
                snapshot: ctx.snapshot(state),
            });
            Ok(EventOutcome::Continue)
        }
        ClientEvent::SubmitRejected(message) => {
            state.inflight_submit = None;
            state.record_rejected();
            state.consecutive_rejected = state.consecutive_rejected.saturating_add(1);
            state.last_error = Some(message.clone());
            ctx.publish_snapshot(state);
            ctx.publish_event(MinerEvent::ShareRejected {
                snapshot: ctx.snapshot(state),
                reason: message,
            });
            Ok(EventOutcome::RecheckConsensus)
        }
        ClientEvent::KeepaliveOk => {
            state.pending_keepalive_deadline = None;
            ctx.publish_snapshot(state);
            Ok(EventOutcome::Continue)
        }
    }
}

fn maybe_abort_on_consensus_switch(
    config: &MinerConfig,
    state: &mut RuntimeState,
) -> anyhow::Result<()> {
    if state.consecutive_rejected < CONSENSUS_RECHECK_REJECT_THRESHOLD {
        return Ok(());
    }
    let current_strategy = state
        .current_job
        .as_ref()
        .map(|job| job.strategy)
        .context("missing current strategy for reject threshold")?;
    panic!(
        "too many consecutive rejected shares with configured consensus strategy {} (job strategy {}), restart with the correct strategy",
        config.strategy,
        current_strategy,
    );
}

async fn wait_or_retry(shutdown: &CancellationToken, state: &mut RuntimeState) -> ConnectState {
    if wait_reconnect_or_shutdown(shutdown, state.reconnect_delay).await {
        ConnectState::Shutdown
    } else {
        state.reconnect_delay = next_reconnect_delay(state.reconnect_delay);
        ConnectState::Retry
    }
}
