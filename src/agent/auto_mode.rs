use crate::{AutoHoldReason, AutoState, Budget, BudgetMode, MinerSnapshot, MinerState, Priority};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

pub(crate) const SYSTEM_USAGE_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
pub(crate) const AUTO_TICK_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const AUTO_RISE_COOLDOWN: Duration = Duration::from_secs(5 * 60);
pub(crate) const AUTO_HEALTHY_CYCLES_REQUIRED: u8 = 3;
const SYSTEM_USAGE_WINDOW_SAMPLES: usize = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SystemUsageSnapshot {
    pub(crate) cpu_percent: f64,
    pub(crate) memory_percent: f64,
    pub(crate) cpu_percent_1m: f64,
    pub(crate) memory_percent_1m: f64,
}

#[derive(Clone, Copy, Debug)]
struct SystemUsageSample {
    cpu_percent: f64,
    memory_percent: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum AutoBudgetTier {
    Floor20,
    Guard30,
    Quarter35,
    Quarter40,
    Balanced40,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AutoSnapshot {
    pub(crate) state: AutoState,
    pub(crate) hold_reason: Option<AutoHoldReason>,
    pub(crate) usage: SystemUsageSnapshot,
}

#[derive(Debug)]
pub(crate) struct AutoRuntime {
    state: AutoState,
    hold_reason: Option<AutoHoldReason>,
    usage: SystemUsageSnapshot,
    last_reconnects: u64,
    healthy_cycles: u8,
    last_raise_at: Option<Instant>,
    last_tier: AutoBudgetTier,
}

pub(crate) struct SystemUsageSampler {
    system: System,
    samples: VecDeque<SystemUsageSample>,
}

pub(crate) struct AutoDecision {
    pub(crate) budget: Budget,
}

impl AutoRuntime {
    pub(crate) fn new(initial_budget: Budget) -> Self {
        Self {
            state: AutoState::Held,
            hold_reason: Some(AutoHoldReason::NotRunning),
            usage: SystemUsageSnapshot::default(),
            last_reconnects: 0,
            healthy_cycles: 0,
            last_raise_at: None,
            last_tier: tier_for_budget(initial_budget),
        }
    }

    pub(crate) fn snapshot(&self, requested_mode: BudgetMode) -> AutoSnapshot {
        AutoSnapshot {
            state: if requested_mode == BudgetMode::Auto {
                self.state
            } else {
                AutoState::Inactive
            },
            hold_reason: if requested_mode == BudgetMode::Auto {
                self.hold_reason
            } else {
                None
            },
            usage: self.usage,
        }
    }

    pub(crate) fn record_usage(&mut self, usage: SystemUsageSnapshot) {
        self.usage = usage;
    }

    pub(crate) fn hold(&mut self, reason: AutoHoldReason) {
        self.state = AutoState::Held;
        self.hold_reason = Some(reason);
        self.healthy_cycles = 0;
    }

    pub(crate) fn deactivate(&mut self, budget: Budget) {
        self.state = AutoState::Inactive;
        self.hold_reason = None;
        self.healthy_cycles = 0;
        self.last_tier = tier_for_budget(budget);
    }

    pub(crate) fn activate(&mut self, state: MinerState) {
        self.healthy_cycles = 0;
        if matches!(state, MinerState::Stopped | MinerState::Paused) {
            self.state = AutoState::Held;
            self.hold_reason = Some(AutoHoldReason::NotRunning);
        } else {
            self.state = AutoState::Active;
            self.hold_reason = None;
        }
    }

    pub(crate) fn evaluate(
        &mut self,
        snapshot: &MinerSnapshot,
        max_threads: u16,
        logical_cpus: usize,
        now: Instant,
    ) -> Option<AutoDecision> {
        if snapshot.requested_mode != BudgetMode::Auto {
            self.state = AutoState::Inactive;
            self.hold_reason = None;
            self.healthy_cycles = 0;
            return None;
        }
        if matches!(snapshot.state, MinerState::Stopped | MinerState::Paused) {
            self.state = AutoState::Held;
            self.hold_reason.get_or_insert(AutoHoldReason::NotRunning);
            self.healthy_cycles = 0;
            return None;
        }
        self.state = AutoState::Active;
        self.hold_reason = None;

        let target_tier = target_tier(
            self.usage,
            snapshot.reconnects > self.last_reconnects,
            snapshot.state,
        );
        self.last_reconnects = snapshot.reconnects;
        let current_tier = tier_for_budget(snapshot.effective_budget);

        if target_tier < current_tier {
            self.healthy_cycles = 0;
            self.last_tier = target_tier;
            return Some(AutoDecision {
                budget: budget_for_tier(target_tier, max_threads, logical_cpus),
            });
        }

        if target_tier == current_tier {
            self.healthy_cycles = 0;
            self.last_tier = current_tier;
            return None;
        }

        self.healthy_cycles = self.healthy_cycles.saturating_add(1);
        if self.healthy_cycles < AUTO_HEALTHY_CYCLES_REQUIRED {
            return None;
        }
        if self
            .last_raise_at
            .is_some_and(|last_raise_at| now.duration_since(last_raise_at) < AUTO_RISE_COOLDOWN)
        {
            return None;
        }

        self.healthy_cycles = 0;
        self.last_raise_at = Some(now);
        let raised_tier = next_higher_tier(current_tier);
        self.last_tier = raised_tier;
        Some(AutoDecision {
            budget: budget_for_tier(raised_tier, max_threads, logical_cpus),
        })
    }

    pub(crate) fn record_applied_budget(&mut self, budget: Budget) {
        self.last_tier = tier_for_budget(budget);
    }
}

impl SystemUsageSampler {
    pub(crate) fn new() -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        system.refresh_cpu_usage();
        system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        Self {
            system,
            samples: VecDeque::with_capacity(SYSTEM_USAGE_WINDOW_SAMPLES),
        }
    }

    pub(crate) fn sample(&mut self) -> SystemUsageSnapshot {
        self.system.refresh_cpu_usage();
        self.system
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());

        let total_memory = self.system.total_memory();
        let used_memory = self.system.used_memory();
        let memory_percent = if total_memory == 0 {
            0.0
        } else {
            (used_memory as f64 / total_memory as f64) * 100.0
        };
        let sample = SystemUsageSample {
            cpu_percent: self.system.global_cpu_usage() as f64,
            memory_percent,
        };
        if self.samples.len() >= SYSTEM_USAGE_WINDOW_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);

        let (cpu_sum, memory_sum, len) =
            self.samples
                .iter()
                .fold((0.0, 0.0, 0usize), |(cpu_sum, memory_sum, len), sample| {
                    (
                        cpu_sum + sample.cpu_percent,
                        memory_sum + sample.memory_percent,
                        len + 1,
                    )
                });
        let avg_divisor = len.max(1) as f64;
        SystemUsageSnapshot {
            cpu_percent: sample.cpu_percent,
            memory_percent: sample.memory_percent,
            cpu_percent_1m: cpu_sum / avg_divisor,
            memory_percent_1m: memory_sum / avg_divisor,
        }
    }
}

fn target_tier(
    usage: SystemUsageSnapshot,
    reconnects_increased: bool,
    miner_state: MinerState,
) -> AutoBudgetTier {
    if matches!(miner_state, MinerState::Reconnecting) || reconnects_increased {
        return AutoBudgetTier::Floor20;
    }
    if usage.cpu_percent_1m >= 80.0 || usage.memory_percent_1m >= 90.0 {
        return AutoBudgetTier::Floor20;
    }
    if usage.cpu_percent_1m >= 65.0 || usage.memory_percent_1m >= 85.0 {
        return AutoBudgetTier::Guard30;
    }
    if usage.cpu_percent_1m >= 50.0 || usage.memory_percent_1m >= 80.0 {
        return AutoBudgetTier::Quarter35;
    }
    if usage.cpu_percent_1m >= 35.0 || usage.memory_percent_1m >= 75.0 {
        return AutoBudgetTier::Quarter40;
    }
    AutoBudgetTier::Balanced40
}

fn budget_for_tier(tier: AutoBudgetTier, max_threads: u16, logical_cpus: usize) -> Budget {
    let limit = usize::from(max_threads.max(1));
    let logical_cpus = logical_cpus.max(1);
    let quarter_threads = logical_cpus.div_ceil(4).max(1).min(limit) as u16;
    let balanced_threads = logical_cpus.div_ceil(2).max(1).min(limit) as u16;
    match tier {
        AutoBudgetTier::Floor20 => Budget {
            threads: 1,
            cpu_percent: 20,
            priority: Priority::Background,
        },
        AutoBudgetTier::Guard30 => Budget {
            threads: 1,
            cpu_percent: 30,
            priority: Priority::Background,
        },
        AutoBudgetTier::Quarter35 => Budget {
            threads: quarter_threads,
            cpu_percent: 35,
            priority: Priority::Background,
        },
        AutoBudgetTier::Quarter40 => Budget {
            threads: quarter_threads,
            cpu_percent: 40,
            priority: Priority::Background,
        },
        AutoBudgetTier::Balanced40 => Budget {
            threads: balanced_threads,
            cpu_percent: 40,
            priority: Priority::Background,
        },
    }
}

fn next_higher_tier(current: AutoBudgetTier) -> AutoBudgetTier {
    match current {
        AutoBudgetTier::Floor20 => AutoBudgetTier::Guard30,
        AutoBudgetTier::Guard30 => AutoBudgetTier::Quarter35,
        AutoBudgetTier::Quarter35 => AutoBudgetTier::Quarter40,
        AutoBudgetTier::Quarter40 | AutoBudgetTier::Balanced40 => AutoBudgetTier::Balanced40,
    }
}

fn tier_for_budget(budget: Budget) -> AutoBudgetTier {
    if budget.threads <= 1 {
        if budget.cpu_percent <= 20 {
            AutoBudgetTier::Floor20
        } else {
            AutoBudgetTier::Guard30
        }
    } else if budget.cpu_percent <= 35 {
        AutoBudgetTier::Quarter35
    } else if budget.cpu_percent <= 40 {
        AutoBudgetTier::Quarter40
    } else {
        AutoBudgetTier::Balanced40
    }
}

#[cfg(test)]
mod tests {
    use super::{
        budget_for_tier, target_tier, AutoBudgetTier, AutoRuntime, SystemUsageSnapshot,
        AUTO_HEALTHY_CYCLES_REQUIRED, AUTO_RISE_COOLDOWN,
    };
    use crate::{AutoState, BudgetMode, MinerSnapshot, MinerState, Priority};
    use std::time::{Duration, Instant};

    fn snapshot(state: MinerState, reconnects: u64, budget: crate::Budget) -> MinerSnapshot {
        MinerSnapshot {
            state,
            connected: matches!(state, MinerState::Running | MinerState::Reconnecting),
            pool: "pool".to_string(),
            worker_name: "worker".to_string(),
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
            reconnects,
            uptime_secs: 0,
            system_cpu_percent: 0.0,
            system_memory_percent: 0.0,
            system_cpu_percent_1m: 0.0,
            system_memory_percent_1m: 0.0,
            auto_state: AutoState::Active,
            auto_hold_reason: None,
            last_error: None,
        }
    }

    #[test]
    fn target_tier_prefers_reconnect_and_high_usage() {
        assert_eq!(
            target_tier(SystemUsageSnapshot::default(), true, MinerState::Running),
            AutoBudgetTier::Floor20
        );
        assert_eq!(
            target_tier(
                SystemUsageSnapshot {
                    cpu_percent_1m: 81.0,
                    ..SystemUsageSnapshot::default()
                },
                false,
                MinerState::Running,
            ),
            AutoBudgetTier::Floor20
        );
        assert_eq!(
            target_tier(
                SystemUsageSnapshot {
                    cpu_percent_1m: 40.0,
                    memory_percent_1m: 76.0,
                    ..SystemUsageSnapshot::default()
                },
                false,
                MinerState::Running,
            ),
            AutoBudgetTier::Quarter40
        );
    }

    #[test]
    fn auto_runtime_holds_when_not_running() {
        let mut runtime = AutoRuntime::new(budget_for_tier(AutoBudgetTier::Guard30, 8, 8));
        runtime.activate(MinerState::Stopped);
        let status = runtime.snapshot(BudgetMode::Auto);
        assert_eq!(status.state, AutoState::Held);
    }

    #[test]
    fn auto_runtime_rises_only_after_healthy_cycles_and_cooldown() {
        let initial_budget = budget_for_tier(AutoBudgetTier::Guard30, 8, 8);
        let mut runtime = AutoRuntime::new(initial_budget);
        runtime.activate(MinerState::Running);
        runtime.record_usage(SystemUsageSnapshot {
            cpu_percent: 10.0,
            memory_percent: 10.0,
            cpu_percent_1m: 10.0,
            memory_percent_1m: 10.0,
        });
        let snap = snapshot(MinerState::Running, 0, initial_budget);
        let now = Instant::now();
        for _ in 0..(AUTO_HEALTHY_CYCLES_REQUIRED - 1) {
            assert!(runtime.evaluate(&snap, 8, 8, now).is_none());
        }
        let decision = runtime
            .evaluate(&snap, 8, 8, now)
            .expect("should raise after healthy cycles");
        assert_eq!(
            decision.budget,
            budget_for_tier(AutoBudgetTier::Quarter35, 8, 8)
        );
        assert!(runtime
            .evaluate(&snap, 8, 8, now + Duration::from_secs(1))
            .is_none());
        assert!(runtime
            .evaluate(&snap, 8, 8, now + AUTO_RISE_COOLDOWN)
            .is_none());
    }

    #[test]
    fn budget_tiers_use_background_priority() {
        let budget = budget_for_tier(AutoBudgetTier::Balanced40, 8, 8);
        assert_eq!(budget.priority, Priority::Background);
        assert_eq!(budget.cpu_percent, 40);
        assert_eq!(budget.threads, 4);
    }
}
