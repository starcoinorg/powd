use super::auto_mode::{AutoRuntime, SystemUsageSnapshot};
use super::config::{build_miner_config, default_max_threads, MintProfile};
use crate::miner::{default_agent_methods, default_miner_capabilities};
use crate::{
    default_budget_for_mode, AgentError, AgentMethods, AutoHoldReason, AutoState, Budget,
    BudgetMode, EventsSinceResponse, MinerCapabilities, MinerConfig, MinerEvent,
    MinerEventEnvelope, MinerHandle, MinerRunner, MinerSnapshot, MinerState,
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, Mutex};

const EVENT_LOG_LIMIT: usize = 256;

#[derive(Clone)]
pub struct SharedState {
    inner: Arc<Mutex<DaemonState>>,
    ops: Arc<Mutex<()>>,
    events_tx: broadcast::Sender<MinerEvent>,
    event_log: Arc<Mutex<EventLog>>,
}

struct DaemonState {
    configured: Option<ConfiguredDaemon>,
    logical_cpus: usize,
    usage: SystemUsageSnapshot,
}

struct ConfiguredDaemon {
    profile: MintProfile,
    config: MinerConfig,
    runner: MinerRunner,
    budget: Budget,
    auto: AutoRuntime,
    handle: Option<MinerHandle>,
}

struct EventLog {
    next_seq: u64,
    events: VecDeque<MinerEventEnvelope>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeIntent {
    Stopped,
    Running,
    Paused,
}

impl SharedState {
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(128);
        Self {
            inner: Arc::new(Mutex::new(DaemonState {
                configured: None,
                logical_cpus: logical_cpus(),
                usage: SystemUsageSnapshot::default(),
            })),
            ops: Arc::new(Mutex::new(())),
            events_tx,
            event_log: Arc::new(Mutex::new(EventLog::default())),
        }
    }

    pub async fn snapshot(&self) -> MinerSnapshot {
        self.inner.lock().await.snapshot()
    }

    pub async fn capabilities(&self) -> MinerCapabilities {
        self.inner.lock().await.capabilities()
    }

    pub async fn methods(&self) -> AgentMethods {
        self.inner.lock().await.methods()
    }

    pub async fn events_since(&self, since_seq: u64) -> EventsSinceResponse {
        self.event_log.lock().await.since(since_seq)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<MinerEvent> {
        self.events_tx.subscribe()
    }

    pub async fn configure(
        &self,
        profile: MintProfile,
    ) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        self.configure_locked(profile).await
    }

    pub async fn start(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        self.start_locked().await
    }

    pub async fn stop(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        let handle = {
            let mut guard = self.inner.lock().await;
            let Some(configured) = guard.configured.as_mut() else {
                return Err(AgentError::NotConfigured);
            };
            if configured.profile.requested_mode == BudgetMode::Auto {
                configured.auto.hold(AutoHoldReason::ManualStop);
            }
            configured.handle.take()
        };
        if let Some(handle) = handle {
            let _ = handle.stop().await;
        }
        Ok(self.snapshot().await)
    }

    pub async fn pause(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        self.pause_locked().await
    }

    pub async fn resume(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        let handle = {
            let guard = self.inner.lock().await;
            let Some(configured) = guard.configured.as_ref() else {
                return Err(AgentError::NotConfigured);
            };
            configured.handle.clone()
        };
        match handle {
            Some(handle) if !is_terminal(handle.snapshot().state) => {
                let snapshot = handle.resume().await?;
                {
                    let mut guard = self.inner.lock().await;
                    if let Some(configured) = guard.configured.as_mut() {
                        if configured.profile.requested_mode == BudgetMode::Auto {
                            configured.auto.activate(snapshot.state);
                        }
                    }
                }
                if self.inner.lock().await.requested_mode() == Some(BudgetMode::Auto) {
                    self.tick_auto_locked().await
                } else {
                    Ok(self.snapshot().await)
                }
            }
            _ => self.start_locked().await,
        }
    }

    pub async fn set_mode(
        &self,
        mode: BudgetMode,
    ) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        let handle = {
            let mut guard = self.inner.lock().await;
            let logical_cpus = guard.logical_cpus;
            let Some(configured) = guard.configured.as_mut() else {
                return Err(AgentError::NotConfigured);
            };
            let budget = default_budget_for_mode(mode, configured.config.max_threads, logical_cpus);
            configured.profile.requested_mode = mode;
            configured.budget = budget;
            match mode {
                BudgetMode::Auto => {
                    let state = configured
                        .handle
                        .as_ref()
                        .map(|handle| handle.snapshot().state)
                        .unwrap_or(MinerState::Stopped);
                    configured.auto.activate(state);
                }
                _ => configured.auto.deactivate(budget),
            }
            configured.handle.clone()
        };
        if let Some(handle) = handle {
            if !is_terminal(handle.snapshot().state) {
                let budget = self
                    .inner
                    .lock()
                    .await
                    .current_budget()
                    .expect("configured budget");
                let snapshot = handle.set_budget(budget).await?;
                let mut guard = self.inner.lock().await;
                if let Some(configured) = guard.configured.as_mut() {
                    if configured.profile.requested_mode == BudgetMode::Auto {
                        configured
                            .auto
                            .record_applied_budget(snapshot.effective_budget);
                    }
                }
                return Ok(guard.apply_snapshot(snapshot));
            }
        }
        Ok(self.snapshot().await)
    }

    pub async fn record_system_usage(&self, usage: SystemUsageSnapshot) {
        let mut guard = self.inner.lock().await;
        guard.usage = usage;
        if let Some(configured) = guard.configured.as_mut() {
            configured.auto.record_usage(usage);
        }
    }

    pub async fn tick_auto(&self) -> std::result::Result<(), AgentError> {
        let _op = self.ops.lock().await;
        let _ = self.tick_auto_locked().await?;
        Ok(())
    }

    pub async fn stop_on_shutdown(&self) {
        let _op = self.ops.lock().await;
        let handle = {
            let mut guard = self.inner.lock().await;
            guard
                .configured
                .as_mut()
                .and_then(|configured| configured.handle.take())
        };
        if let Some(handle) = handle {
            let _ = handle.stop().await;
        }
    }

    async fn configure_locked(
        &self,
        profile: MintProfile,
    ) -> std::result::Result<MinerSnapshot, AgentError> {
        let (resume_intent, handle_to_stop) = {
            let mut guard = self.inner.lock().await;
            if guard
                .configured
                .as_ref()
                .is_some_and(|configured| configured.profile == profile)
            {
                return Ok(guard.snapshot());
            }
            let intent = guard.runtime_intent();
            let handle = guard
                .configured
                .as_mut()
                .and_then(|configured| configured.handle.take());
            (intent, handle)
        };

        if let Some(handle) = handle_to_stop {
            let _ = handle.stop().await;
        }

        {
            let mut guard = self.inner.lock().await;
            let derived = build_miner_config(&profile).map_err(AgentError::InvalidConfig)?;
            let runner =
                MinerRunner::new(derived.miner_config.clone()).map_err(AgentError::from)?;
            let mut auto = AutoRuntime::new(derived.initial_budget);
            auto.record_usage(guard.usage);
            match profile.requested_mode {
                BudgetMode::Auto => match resume_intent {
                    RuntimeIntent::Running => auto.activate(MinerState::Running),
                    RuntimeIntent::Paused | RuntimeIntent::Stopped => {
                        auto.hold(AutoHoldReason::NotRunning)
                    }
                },
                _ => auto.deactivate(derived.initial_budget),
            }
            guard.configured = Some(ConfiguredDaemon {
                profile,
                config: derived.miner_config,
                runner,
                budget: derived.initial_budget,
                auto,
                handle: None,
            });
        }

        match resume_intent {
            RuntimeIntent::Stopped => Ok(self.snapshot().await),
            RuntimeIntent::Running => self.start_locked().await,
            RuntimeIntent::Paused => {
                let _ = self.start_locked().await?;
                self.pause_locked().await
            }
        }
    }

    async fn start_locked(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let maybe_events = {
            let mut guard = self.inner.lock().await;
            if guard.configured.is_none() {
                return Err(AgentError::NotConfigured);
            }

            let existing_snapshot = guard
                .configured
                .as_ref()
                .and_then(|configured| configured.handle.as_ref().map(|handle| handle.snapshot()))
                .filter(|snapshot| !is_terminal(snapshot.state));
            if let Some(snapshot) = existing_snapshot {
                if guard
                    .configured
                    .as_ref()
                    .expect("configured daemon should exist")
                    .profile
                    .requested_mode
                    == BudgetMode::Auto
                {
                    guard
                        .configured
                        .as_mut()
                        .expect("configured daemon should exist")
                        .auto
                        .activate(snapshot.state);
                }
                return Ok(guard.snapshot());
            }

            let configured = guard
                .configured
                .as_mut()
                .expect("configured daemon should exist");
            let handle = configured
                .runner
                .spawn(configured.budget)
                .map_err(AgentError::from)?;
            let snapshot = handle.snapshot();
            if configured.profile.requested_mode == BudgetMode::Auto {
                configured.auto.activate(snapshot.state);
            }
            let events = handle.subscribe_events();
            configured.handle = Some(handle);
            Some(events)
        };
        if let Some(events) = maybe_events {
            self.spawn_event_forwarder(events);
        }
        if self.inner.lock().await.requested_mode() == Some(BudgetMode::Auto) {
            self.tick_auto_locked().await
        } else {
            Ok(self.snapshot().await)
        }
    }

    async fn pause_locked(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let handle = {
            let mut guard = self.inner.lock().await;
            let Some(configured) = guard.configured.as_mut() else {
                return Err(AgentError::NotConfigured);
            };
            if configured.profile.requested_mode == BudgetMode::Auto {
                configured.auto.hold(AutoHoldReason::ManualPause);
            }
            configured.handle.clone()
        };
        match handle {
            Some(handle) if !is_terminal(handle.snapshot().state) => {
                let snapshot = handle.pause().await?;
                Ok(self.inner.lock().await.apply_snapshot(snapshot))
            }
            _ => Ok(self.snapshot().await),
        }
    }

    async fn tick_auto_locked(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let (handle, budget) = {
            let mut guard = self.inner.lock().await;
            if guard.configured.is_none() {
                return Ok(guard.snapshot());
            }
            let snapshot = guard.snapshot();
            let logical_cpus = guard.logical_cpus;
            let (handle, budget, same_budget) = {
                let configured = guard
                    .configured
                    .as_mut()
                    .expect("configured daemon should exist");
                let max_threads = configured.config.max_threads;
                let decision =
                    configured
                        .auto
                        .evaluate(&snapshot, max_threads, logical_cpus, Instant::now());
                let Some(decision) = decision else {
                    return Ok(snapshot);
                };
                if decision.budget == configured.budget {
                    configured.auto.record_applied_budget(decision.budget);
                    (None, configured.budget, true)
                } else {
                    configured.budget = decision.budget;
                    (configured.handle.clone(), decision.budget, false)
                }
            };
            if same_budget {
                return Ok(guard.snapshot());
            }
            (handle, budget)
        };

        if let Some(handle) = handle {
            if !is_terminal(handle.snapshot().state) {
                let snapshot = handle.set_budget(budget).await?;
                let mut guard = self.inner.lock().await;
                if let Some(configured) = guard.configured.as_mut() {
                    configured
                        .auto
                        .record_applied_budget(snapshot.effective_budget);
                }
                return Ok(guard.apply_snapshot(snapshot));
            }
        }

        Ok(self.snapshot().await)
    }

    fn spawn_event_forwarder(&self, mut events: broadcast::Receiver<MinerEvent>) {
        let events_tx = self.events_tx.clone();
        let event_log = Arc::clone(&self.event_log);
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let event = inner.lock().await.apply_event(event);
                        event_log.lock().await.push(event.clone());
                        let _ = events_tx.send(event);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}

impl DaemonState {
    fn snapshot(&self) -> MinerSnapshot {
        let Some(configured) = self.configured.as_ref() else {
            return unconfigured_snapshot(self.logical_cpus, self.usage);
        };
        let base = configured
            .handle
            .as_ref()
            .map(MinerHandle::snapshot)
            .unwrap_or_else(|| MinerSnapshot {
                state: MinerState::Stopped,
                connected: false,
                pool: configured.config.pool.clone(),
                worker_name: configured.config.login.worker_name().to_string(),
                requested_mode: configured.profile.requested_mode,
                effective_budget: configured.budget,
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
            });
        self.apply_snapshot(base)
    }

    fn capabilities(&self) -> MinerCapabilities {
        self.configured
            .as_ref()
            .map(|configured| configured.runner.capabilities())
            .unwrap_or_else(|| default_miner_capabilities(default_max_threads()))
    }

    fn methods(&self) -> AgentMethods {
        build_methods(self.capabilities())
    }

    fn apply_snapshot(&self, mut snapshot: MinerSnapshot) -> MinerSnapshot {
        let Some(configured) = self.configured.as_ref() else {
            return unconfigured_snapshot(self.logical_cpus, self.usage);
        };
        let auto = configured.auto.snapshot(configured.profile.requested_mode);
        snapshot.requested_mode = configured.profile.requested_mode;
        snapshot.effective_budget = configured.budget;
        snapshot.system_cpu_percent = auto.usage.cpu_percent;
        snapshot.system_memory_percent = auto.usage.memory_percent;
        snapshot.system_cpu_percent_1m = auto.usage.cpu_percent_1m;
        snapshot.system_memory_percent_1m = auto.usage.memory_percent_1m;
        snapshot.auto_state = auto.state;
        snapshot.auto_hold_reason = auto.hold_reason;
        snapshot
    }

    fn apply_event(&self, event: MinerEvent) -> MinerEvent {
        match event {
            MinerEvent::Started { snapshot } => MinerEvent::Started {
                snapshot: self.apply_snapshot(snapshot),
            },
            MinerEvent::Paused { snapshot } => MinerEvent::Paused {
                snapshot: self.apply_snapshot(snapshot),
            },
            MinerEvent::Resumed { snapshot } => MinerEvent::Resumed {
                snapshot: self.apply_snapshot(snapshot),
            },
            MinerEvent::Stopped { snapshot } => MinerEvent::Stopped {
                snapshot: self.apply_snapshot(snapshot),
            },
            MinerEvent::Reconnecting { snapshot } => MinerEvent::Reconnecting {
                snapshot: self.apply_snapshot(snapshot),
            },
            MinerEvent::BudgetChanged { snapshot } => MinerEvent::BudgetChanged {
                snapshot: self.apply_snapshot(snapshot),
            },
            MinerEvent::ShareAccepted { snapshot } => MinerEvent::ShareAccepted {
                snapshot: self.apply_snapshot(snapshot),
            },
            MinerEvent::ShareRejected { snapshot, reason } => MinerEvent::ShareRejected {
                snapshot: self.apply_snapshot(snapshot),
                reason,
            },
            MinerEvent::Error { snapshot, message } => MinerEvent::Error {
                snapshot: self.apply_snapshot(snapshot),
                message,
            },
        }
    }

    fn runtime_intent(&self) -> RuntimeIntent {
        let Some(configured) = self.configured.as_ref() else {
            return RuntimeIntent::Stopped;
        };
        let Some(handle) = configured.handle.as_ref() else {
            return RuntimeIntent::Stopped;
        };
        match handle.snapshot().state {
            MinerState::Paused => RuntimeIntent::Paused,
            MinerState::Starting | MinerState::Running | MinerState::Reconnecting => {
                RuntimeIntent::Running
            }
            MinerState::Stopped | MinerState::Error => RuntimeIntent::Stopped,
        }
    }

    fn requested_mode(&self) -> Option<BudgetMode> {
        self.configured
            .as_ref()
            .map(|configured| configured.profile.requested_mode)
    }

    fn current_budget(&self) -> Option<Budget> {
        self.configured.as_ref().map(|configured| configured.budget)
    }
}

impl EventLog {
    fn push(&mut self, event: MinerEvent) {
        let envelope = MinerEventEnvelope {
            seq: self.next_seq,
            event,
        };
        self.next_seq = self.next_seq.saturating_add(1);
        if self.events.len() >= EVENT_LOG_LIMIT {
            self.events.pop_front();
        }
        self.events.push_back(envelope);
    }

    fn since(&self, since_seq: u64) -> EventsSinceResponse {
        let events = self
            .events
            .iter()
            .filter(|event| event.seq > since_seq)
            .cloned()
            .collect();
        EventsSinceResponse {
            next_seq: self.next_seq,
            events,
        }
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self {
            next_seq: 1,
            events: VecDeque::new(),
        }
    }
}

fn build_methods(capabilities: MinerCapabilities) -> AgentMethods {
    default_agent_methods(capabilities.max_threads)
}

fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
}

fn is_terminal(state: MinerState) -> bool {
    matches!(state, MinerState::Stopped | MinerState::Error)
}

fn unconfigured_snapshot(logical_cpus: usize, usage: SystemUsageSnapshot) -> MinerSnapshot {
    let budget = default_budget_for_mode(BudgetMode::Auto, default_max_threads(), logical_cpus);
    MinerSnapshot {
        state: MinerState::Stopped,
        connected: false,
        pool: String::new(),
        worker_name: String::new(),
        requested_mode: BudgetMode::Auto,
        effective_budget: budget,
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
        system_cpu_percent: usage.cpu_percent,
        system_memory_percent: usage.memory_percent,
        system_cpu_percent_1m: usage.cpu_percent_1m,
        system_memory_percent_1m: usage.memory_percent_1m,
        auto_state: AutoState::Held,
        auto_hold_reason: Some(AutoHoldReason::NotRunning),
        last_error: None,
    }
}
