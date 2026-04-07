use crate::{
    default_budget_for_mode, AgentError, AgentMethods, Budget, BudgetMode, EventsSinceResponse,
    MinerCapabilities, MinerConfig, MinerEvent, MinerEventEnvelope, MinerHandle, MinerRunner,
    MinerSnapshot, MinerState, Priority,
};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

const EVENT_LOG_LIMIT: usize = 256;

#[derive(Clone)]
pub struct SharedState {
    inner: Arc<Mutex<DaemonState>>,
    ops: Arc<Mutex<()>>,
    events_tx: broadcast::Sender<MinerEvent>,
    event_log: Arc<Mutex<EventLog>>,
}

pub struct BudgetUpdate {
    pub threads: Option<u16>,
    pub cpu_percent: Option<u8>,
    pub priority: Option<Priority>,
}

struct DaemonState {
    config: MinerConfig,
    runner: MinerRunner,
    budget: Budget,
    handle: Option<MinerHandle>,
}

struct EventLog {
    next_seq: u64,
    events: VecDeque<MinerEventEnvelope>,
}

impl SharedState {
    pub fn new(config: MinerConfig, runner: MinerRunner, budget: Budget) -> Self {
        let (events_tx, _) = broadcast::channel(128);
        Self {
            inner: Arc::new(Mutex::new(DaemonState {
                config,
                runner,
                budget,
                handle: None,
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

    pub async fn start(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        let mut guard = self.inner.lock().await;
        if let Some(handle) = guard.handle.as_ref() {
            let snapshot = handle.snapshot();
            if !is_terminal(snapshot.state) {
                return Ok(snapshot);
            }
        }
        let handle = guard.runner.spawn(guard.budget).map_err(AgentError::from)?;
        let snapshot = handle.snapshot();
        let events = handle.subscribe_events();
        guard.handle = Some(handle);
        drop(guard);
        self.spawn_event_forwarder(events);
        Ok(snapshot)
    }

    pub async fn stop(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        let handle = self.inner.lock().await.handle.take();
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
        let handle = self.inner.lock().await.handle.clone();
        match handle {
            Some(handle) if !is_terminal(handle.snapshot().state) => handle.resume().await,
            _ => Ok(self.snapshot().await),
        }
    }

    pub async fn set_mode(
        &self,
        mode: BudgetMode,
    ) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        let capabilities = self.capabilities().await;
        let budget = default_budget_for_mode(mode, capabilities.max_threads, logical_cpus());
        self.apply_budget_locked(budget).await
    }

    pub async fn set_budget(
        &self,
        update: BudgetUpdate,
    ) -> std::result::Result<MinerSnapshot, AgentError> {
        let _op = self.ops.lock().await;
        let budget = {
            let guard = self.inner.lock().await;
            let current = guard
                .handle
                .as_ref()
                .map(|handle| handle.snapshot().current_budget)
                .unwrap_or(guard.budget);
            Budget {
                threads: update.threads.unwrap_or(current.threads),
                cpu_percent: update.cpu_percent.unwrap_or(current.cpu_percent),
                priority: update.priority.unwrap_or(current.priority),
            }
            .validate(guard.capabilities().max_threads)
            .map_err(AgentError::InvalidBudget)?
        };
        self.apply_budget_locked(budget).await
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<MinerEvent> {
        self.events_tx.subscribe()
    }

    pub async fn stop_on_shutdown(&self) {
        let _op = self.ops.lock().await;
        let handle = self.inner.lock().await.handle.take();
        if let Some(handle) = handle {
            let _ = handle.stop().await;
        }
    }

    async fn pause_locked(&self) -> std::result::Result<MinerSnapshot, AgentError> {
        let handle = self.inner.lock().await.handle.clone();
        match handle {
            Some(handle) if !is_terminal(handle.snapshot().state) => handle.pause().await,
            _ => Ok(self.snapshot().await),
        }
    }

    async fn apply_budget_locked(
        &self,
        budget: Budget,
    ) -> std::result::Result<MinerSnapshot, AgentError> {
        let handle = {
            let mut guard = self.inner.lock().await;
            guard.budget = budget;
            guard.handle.clone()
        };
        if let Some(handle) = handle {
            if !is_terminal(handle.snapshot().state) {
                return handle.set_budget(budget).await;
            }
        }
        Ok(self.snapshot().await)
    }

    fn spawn_event_forwarder(&self, mut events: broadcast::Receiver<MinerEvent>) {
        let events_tx = self.events_tx.clone();
        let event_log = Arc::clone(&self.event_log);
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
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
        self.handle
            .as_ref()
            .map(MinerHandle::snapshot)
            .unwrap_or_else(|| MinerSnapshot {
                state: MinerState::Stopped,
                connected: false,
                pool: self.config.pool.clone(),
                worker_name: self.config.login.worker_name().to_string(),
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
                current_budget: self.budget,
                last_error: None,
            })
    }

    fn capabilities(&self) -> MinerCapabilities {
        self.runner.capabilities()
    }

    fn methods(&self) -> AgentMethods {
        self.config.methods()
    }
}

fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
}

fn is_terminal(state: MinerState) -> bool {
    matches!(state, MinerState::Stopped | MinerState::Error)
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
