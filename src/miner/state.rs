use super::{Budget, MinerSnapshot, MinerState};
use crate::mining::job::{MiningJob, SolvedShare};
use crate::stratum::client::StratumClient;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);
const SUBMIT_QUEUE_LIMIT: usize = 128;
const TREND_BUCKET_SPAN: Duration = Duration::from_secs(5);
const TREND_WINDOW: Duration = Duration::from_secs(5 * 60);

#[derive(Debug)]
pub(crate) struct RuntimeState {
    pub(crate) state: MinerState,
    pub(crate) accepted: u64,
    pub(crate) rejected: u64,
    pub(crate) consecutive_rejected: u64,
    pub(crate) submitted: u64,
    pub(crate) reconnects: u64,
    pub(crate) submit_queue: VecDeque<QueuedShare>,
    pub(crate) inflight_submit: Option<InflightSubmit>,
    pub(crate) pending_keepalive_deadline: Option<tokio::time::Instant>,
    pub(crate) client: Option<StratumClient>,
    pub(crate) current_job: Option<MiningJob>,
    pub(crate) reconnect_delay: Duration,
    pub(crate) last_status_at: Instant,
    pub(crate) last_status_hashes: u64,
    pub(crate) last_hashrate: f64,
    pub(crate) rolling_stats: RollingStats,
    pub(crate) budget: Budget,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct QueuedShare {
    pub(crate) share: SolvedShare,
    queued_at: Instant,
}

#[derive(Debug)]
pub(crate) struct InflightSubmit {
    pub(crate) share: SolvedShare,
    pub(crate) sent_at: Instant,
}

pub(crate) enum ConnectState {
    Ready,
    Retry,
    Shutdown,
}

pub(crate) enum ConnectFailure {
    Retryable(String),
    Permanent(String),
}

#[derive(Debug, Default)]
pub(crate) struct RollingStats {
    buckets: VecDeque<RollingBucket>,
}

#[derive(Debug)]
struct RollingBucket {
    started_at: Instant,
    hashes: u64,
    submitted: u64,
    accepted: u64,
    rejected: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct RollingSnapshot {
    hashrate: f64,
    submitted: u64,
    accepted: u64,
    rejected: u64,
    reject_rate: f64,
}

impl RuntimeState {
    pub(crate) fn new(started_at: Instant, budget: Budget) -> Self {
        Self {
            state: MinerState::Starting,
            accepted: 0,
            rejected: 0,
            consecutive_rejected: 0,
            submitted: 0,
            reconnects: 0,
            submit_queue: VecDeque::new(),
            inflight_submit: None,
            pending_keepalive_deadline: None,
            client: None,
            current_job: None,
            reconnect_delay: super::runtime::RECONNECT_BASE_DELAY,
            last_status_at: started_at,
            last_status_hashes: 0,
            last_hashrate: 0.0,
            rolling_stats: RollingStats::default(),
            budget,
            last_error: None,
        }
    }

    pub(crate) fn should_stop(&self, accepted_goal: u64) -> bool {
        self.accepted >= accepted_goal
    }

    pub(crate) fn mark_disconnected(&mut self) {
        if let Some(inflight) = self.inflight_submit.take() {
            self.queue_share(inflight.share);
        }
        self.state = MinerState::Reconnecting;
        self.consecutive_rejected = 0;
        self.pending_keepalive_deadline = None;
        self.client = None;
    }

    pub(crate) fn queue_share(&mut self, share: SolvedShare) {
        if self.submit_queue.len() >= SUBMIT_QUEUE_LIMIT {
            self.submit_queue.pop_front();
        }
        self.submit_queue.push_back(QueuedShare {
            share,
            queued_at: Instant::now(),
        });
    }

    pub(crate) fn pop_next_share(&mut self) -> Option<SolvedShare> {
        let current_job = self.current_job.as_ref();
        while let Some(queued) = self.submit_queue.pop_front() {
            if queued.queued_at.elapsed() >= SUBMIT_TIMEOUT {
                continue;
            }
            if let Some(job) = current_job {
                if queued.share.worker_id != job.worker_id || queued.share.job_id != job.job_id {
                    continue;
                }
            }
            return Some(queued.share);
        }
        None
    }

    pub(crate) fn drop_stale_shares(&mut self) {
        let current_job = self.current_job.as_ref();
        self.submit_queue.retain(|queued| {
            if queued.queued_at.elapsed() >= SUBMIT_TIMEOUT {
                return false;
            }
            match current_job {
                Some(job) => {
                    queued.share.worker_id == job.worker_id && queued.share.job_id == job.job_id
                }
                None => false,
            }
        });
        if self.inflight_submit.as_ref().is_some_and(|inflight| {
            current_job.is_none_or(|job| {
                inflight.share.worker_id != job.worker_id || inflight.share.job_id != job.job_id
            })
        }) {
            self.inflight_submit = None;
        }
    }

    pub(crate) fn handle_submit_timeout(&mut self) -> bool {
        if let Some(inflight) = self.inflight_submit.as_ref() {
            if inflight.sent_at.elapsed() >= SUBMIT_TIMEOUT {
                self.last_error = Some(format!(
                    "submit timeout worker={} job_id={}",
                    inflight.share.worker_name, inflight.share.job_id,
                ));
                self.mark_disconnected();
                return true;
            }
        }
        false
    }

    pub(crate) fn handle_keepalive_timeout(&mut self) -> bool {
        if self
            .pending_keepalive_deadline
            .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
        {
            self.reconnects = self.reconnects.saturating_add(1);
            self.last_error = Some("keepalive response timeout".to_string());
            self.mark_disconnected();
            return true;
        }
        false
    }

    pub(crate) fn refresh_hashrate(&mut self, total_hashes: u64) {
        let now = Instant::now();
        let elapsed_window = now
            .duration_since(self.last_status_at)
            .as_secs_f64()
            .max(1e-6);
        let delta_hashes = total_hashes.saturating_sub(self.last_status_hashes);
        self.last_hashrate = delta_hashes as f64 / elapsed_window;
        self.last_status_at = now;
        self.last_status_hashes = total_hashes;
        self.rolling_stats.record_hashes(now, delta_hashes);
    }

    pub(crate) fn snapshot(
        &self,
        started_at: Instant,
        pool: &str,
        worker_name: &str,
    ) -> MinerSnapshot {
        let rolling = self.rolling_stats.snapshot(Instant::now());
        MinerSnapshot {
            state: self.state,
            connected: self.client.is_some(),
            pool: pool.to_string(),
            worker_name: worker_name.to_string(),
            hashrate: self.last_hashrate,
            hashrate_5m: rolling.hashrate,
            accepted: self.accepted,
            accepted_5m: rolling.accepted,
            rejected: self.rejected,
            rejected_5m: rolling.rejected,
            submitted: self.submitted,
            submitted_5m: rolling.submitted,
            reject_rate_5m: rolling.reject_rate,
            reconnects: self.reconnects,
            uptime_secs: started_at.elapsed().as_secs(),
            current_budget: self.budget,
            last_error: self.last_error.clone(),
        }
    }

    pub(crate) fn record_submitted(&mut self) {
        self.submitted = self.submitted.saturating_add(1);
        self.rolling_stats.record_submitted(Instant::now(), 1);
    }

    pub(crate) fn record_accepted(&mut self) {
        self.accepted = self.accepted.saturating_add(1);
        self.rolling_stats.record_accepted(Instant::now(), 1);
    }

    pub(crate) fn record_rejected(&mut self) {
        self.rejected = self.rejected.saturating_add(1);
        self.rolling_stats.record_rejected(Instant::now(), 1);
    }
}

impl RollingStats {
    fn record_hashes(&mut self, now: Instant, hashes: u64) {
        self.prune(now);
        if hashes > 0 {
            let bucket = self.bucket_mut(now);
            bucket.hashes = bucket.hashes.saturating_add(hashes);
        }
    }

    fn record_submitted(&mut self, now: Instant, count: u64) {
        self.prune(now);
        let bucket = self.bucket_mut(now);
        bucket.submitted = bucket.submitted.saturating_add(count);
    }

    fn record_accepted(&mut self, now: Instant, count: u64) {
        self.prune(now);
        let bucket = self.bucket_mut(now);
        bucket.accepted = bucket.accepted.saturating_add(count);
    }

    fn record_rejected(&mut self, now: Instant, count: u64) {
        self.prune(now);
        let bucket = self.bucket_mut(now);
        bucket.rejected = bucket.rejected.saturating_add(count);
    }

    fn snapshot(&self, now: Instant) -> RollingSnapshot {
        let cutoff = now - TREND_WINDOW;
        let mut total_hashes = 0_u64;
        let mut submitted = 0_u64;
        let mut accepted = 0_u64;
        let mut rejected = 0_u64;
        let mut first_bucket_at = None;

        for bucket in self
            .buckets
            .iter()
            .filter(|bucket| bucket.started_at >= cutoff)
        {
            first_bucket_at.get_or_insert(bucket.started_at);
            total_hashes = total_hashes.saturating_add(bucket.hashes);
            submitted = submitted.saturating_add(bucket.submitted);
            accepted = accepted.saturating_add(bucket.accepted);
            rejected = rejected.saturating_add(bucket.rejected);
        }

        let elapsed_secs = first_bucket_at
            .map(|started_at| now.duration_since(started_at).as_secs_f64().max(1.0))
            .unwrap_or(0.0);
        let hashrate = if elapsed_secs > 0.0 {
            total_hashes as f64 / elapsed_secs
        } else {
            0.0
        };
        let reject_rate = if submitted > 0 {
            rejected as f64 / submitted as f64
        } else {
            0.0
        };

        RollingSnapshot {
            hashrate,
            submitted,
            accepted,
            rejected,
            reject_rate,
        }
    }

    fn bucket_mut(&mut self, now: Instant) -> &mut RollingBucket {
        let needs_new_bucket = self
            .buckets
            .back()
            .is_none_or(|bucket| now.duration_since(bucket.started_at) >= TREND_BUCKET_SPAN);
        if needs_new_bucket {
            self.buckets.push_back(RollingBucket {
                started_at: now,
                hashes: 0,
                submitted: 0,
                accepted: 0,
                rejected: 0,
            });
        }
        self.buckets
            .back_mut()
            .expect("rolling stats bucket must exist")
    }

    fn prune(&mut self, now: Instant) {
        let cutoff = now - TREND_WINDOW;
        while self
            .buckets
            .front()
            .is_some_and(|bucket| bucket.started_at < cutoff)
        {
            self.buckets.pop_front();
        }
    }
}
