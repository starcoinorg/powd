use super::{Budget, Priority};
use crate::mining::job::{MiningJob, SolvedShare};
use crate::mining::pow;
use std::cmp;
use std::sync::{
    atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const HASH_BATCH: u32 = 512;
const THROTTLE_SLICE: Duration = Duration::from_millis(20);
const THROTTLE_WAIT_SLICE: Duration = Duration::from_millis(10);

#[derive(Clone)]
struct SolverShared {
    job: Arc<Mutex<Option<MiningJob>>>,
    generation: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    wake: Arc<Condvar>,
    sleep_lock: Arc<Mutex<()>>,
    sleep_wake: Arc<Condvar>,
    active_threads: Arc<AtomicU16>,
    cpu_percent: Arc<AtomicU8>,
    paused: Arc<AtomicBool>,
}

pub(super) struct SolverPool {
    shared: SolverShared,
    handles: Vec<thread::JoinHandle<()>>,
}

pub(super) struct SolverPoolGuard(Option<SolverPool>);

impl SolverPoolGuard {
    pub(super) fn new(pool: SolverPool) -> Self {
        Self(Some(pool))
    }

    pub(super) fn pool(&self) -> &SolverPool {
        self.0.as_ref().expect("solver pool guard must own pool")
    }

    pub(super) fn shutdown(&mut self) {
        if let Some(pool) = self.0.take() {
            pool.shutdown();
        }
    }
}

impl Drop for SolverPoolGuard {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl SolverPool {
    pub(super) fn start(
        max_threads: u16,
        initial_budget: Budget,
        hashes: Arc<AtomicU64>,
        share_tx: mpsc::Sender<SolvedShare>,
    ) -> Self {
        let thread_count = max_threads.max(1);
        let shared = SolverShared {
            job: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            shutdown: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(Condvar::new()),
            sleep_lock: Arc::new(Mutex::new(())),
            sleep_wake: Arc::new(Condvar::new()),
            active_threads: Arc::new(AtomicU16::new(initial_budget.threads)),
            cpu_percent: Arc::new(AtomicU8::new(initial_budget.cpu_percent)),
            paused: Arc::new(AtomicBool::new(false)),
        };
        let mut handles = Vec::with_capacity(thread_count as usize);
        for idx in 0..u32::from(thread_count) {
            let hashes = Arc::clone(&hashes);
            let share_tx = share_tx.clone();
            let shared = shared.clone();
            let handle = thread::Builder::new()
                .name(format!("cpu-miner-{idx}"))
                .spawn(move || run_worker(shared, hashes, share_tx, idx, thread_count))
                .expect("spawn cpu miner worker");
            handles.push(handle);
        }
        Self { shared, handles }
    }

    pub(super) fn set_job(&self, job: MiningJob) {
        let mut job_state = self.shared.job.lock().expect("lock solver job");
        *job_state = Some(job);
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.wake.notify_all();
        self.shared.sleep_wake.notify_all();
    }

    pub(super) fn clear_job(&self) {
        let mut job_state = self.shared.job.lock().expect("lock solver job");
        *job_state = None;
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.wake.notify_all();
        self.shared.sleep_wake.notify_all();
    }

    pub(super) fn apply_budget(&self, budget: Budget) {
        self.shared
            .active_threads
            .store(budget.threads, Ordering::Release);
        self.shared
            .cpu_percent
            .store(budget.cpu_percent, Ordering::Release);
        self.shared.wake.notify_all();
        self.shared.sleep_wake.notify_all();
    }

    pub(super) fn set_priority(&self, priority: Priority) {
        if let Err(err) = super::runtime_support::apply_runtime_priority(priority) {
            starcoin_logger::prelude::warn!(target: "cpu_miner", "set priority failed: {err}");
        }
    }

    pub(super) fn pause(&self) {
        self.shared.paused.store(true, Ordering::Release);
        self.shared.wake.notify_all();
        self.shared.sleep_wake.notify_all();
    }

    pub(super) fn resume(&self) {
        self.shared.paused.store(false, Ordering::Release);
        self.shared.wake.notify_all();
        self.shared.sleep_wake.notify_all();
    }

    fn shutdown(self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        if let Ok(mut job_state) = self.shared.job.lock() {
            *job_state = None;
        }
        self.shared.wake.notify_all();
        self.shared.sleep_wake.notify_all();
        for handle in self.handles {
            let _ = handle.join();
        }
    }
}

fn run_worker(
    shared: SolverShared,
    hashes: Arc<AtomicU64>,
    share_tx: mpsc::Sender<SolvedShare>,
    worker_index: u32,
    nonce_stride: u16,
) {
    let mut nonce = worker_index;
    let mut observed_generation = 0;

    loop {
        let (job, generation) = wait_for_work(&shared, &mut observed_generation, worker_index);
        let Some(job) = job else {
            return;
        };

        let mut batch_start = Instant::now();
        let mut batch_hashes = 0u32;
        loop {
            if should_yield(&shared, generation, worker_index) {
                break;
            }

            if let Ok(hash) = pow::calculate_pow_hash(job.strategy, &job.blob, nonce, &job.extra) {
                hashes.fetch_add(1, Ordering::Relaxed);
                batch_hashes = batch_hashes.saturating_add(1);
                if pow::hash_meets_target(&hash, job.share_target) {
                    let _ = share_tx.try_send(SolvedShare {
                        worker_id: job.worker_id.clone(),
                        worker_name: job.worker_name.clone(),
                        job_id: job.job_id.clone(),
                        nonce,
                        hash,
                    });
                }
            }
            nonce = nonce.wrapping_add(u32::from(nonce_stride));

            if should_apply_throttle(batch_hashes, batch_start.elapsed()) {
                apply_throttle(&shared, generation, worker_index, &mut batch_start);
                batch_hashes = 0;
            }
        }
    }
}

fn wait_for_work(
    shared: &SolverShared,
    observed_generation: &mut u64,
    worker_index: u32,
) -> (Option<MiningJob>, u64) {
    let mut job_guard = shared.job.lock().expect("lock solver job");
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            return (None, 0);
        }
        let generation = shared.generation.load(Ordering::Acquire);
        let paused = shared.paused.load(Ordering::Acquire);
        let active_threads = u32::from(shared.active_threads.load(Ordering::Acquire));
        if generation != *observed_generation {
            *observed_generation = generation;
        }
        if !paused && worker_index < active_threads {
            if let Some(job) = job_guard.clone() {
                return (Some(job), generation);
            }
        }
        job_guard = shared.wake.wait(job_guard).expect("wait solver wake");
    }
}

fn should_yield(shared: &SolverShared, generation: u64, worker_index: u32) -> bool {
    shared.shutdown.load(Ordering::Relaxed)
        || shared.paused.load(Ordering::Relaxed)
        || shared.generation.load(Ordering::Relaxed) != generation
        || worker_index >= u32::from(shared.active_threads.load(Ordering::Relaxed))
}

fn apply_throttle(
    shared: &SolverShared,
    generation: u64,
    worker_index: u32,
    batch_start: &mut Instant,
) {
    let cpu_percent = shared.cpu_percent.load(Ordering::Relaxed);
    if cpu_percent >= 100 {
        *batch_start = Instant::now();
        return;
    }
    let elapsed = batch_start.elapsed();
    let active = cmp::max(elapsed, THROTTLE_SLICE);
    let target_total = active.mul_f64(100.0 / f64::from(cpu_percent));
    if target_total > elapsed {
        interruptible_sleep(shared, generation, worker_index, target_total - elapsed);
    }
    *batch_start = Instant::now();
}

fn should_apply_throttle(batch_hashes: u32, elapsed: Duration) -> bool {
    batch_hashes >= HASH_BATCH || elapsed >= THROTTLE_SLICE
}

fn interruptible_sleep(
    shared: &SolverShared,
    generation: u64,
    worker_index: u32,
    mut remaining: Duration,
) {
    while !remaining.is_zero() {
        if should_yield(shared, generation, worker_index) {
            return;
        }
        let step = remaining.min(THROTTLE_WAIT_SLICE);
        let sleep_guard = shared
            .sleep_lock
            .lock()
            .expect("lock solver sleep gate for throttle");
        let (_guard, timeout) = shared
            .sleep_wake
            .wait_timeout(sleep_guard, step)
            .expect("wait throttle wake");
        if timeout.timed_out() {
            remaining = remaining.saturating_sub(step);
        } else {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{should_apply_throttle, HASH_BATCH, THROTTLE_SLICE};
    use std::time::Duration;

    #[test]
    fn throttle_triggers_when_batch_is_full() {
        assert!(should_apply_throttle(HASH_BATCH, Duration::from_millis(1)));
    }

    #[test]
    fn throttle_triggers_when_elapsed_reaches_slice() {
        assert!(should_apply_throttle(1, THROTTLE_SLICE));
    }

    #[test]
    fn throttle_waits_for_more_work_when_batch_and_time_are_small() {
        assert!(!should_apply_throttle(
            HASH_BATCH.saturating_sub(1),
            THROTTLE_SLICE.saturating_sub(Duration::from_millis(1))
        ));
    }
}
