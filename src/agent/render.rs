use super::reward::WalletRewardSnapshot;
use super::wallet_support::{DoctorReport, WalletConfigSummary};
use crate::{Budget, MinerEvent, MinerSnapshot};
use serde::Serialize;

pub(crate) fn print_json_or_text<T, F>(value: &T, json_output: bool, printer: F)
where
    T: Serialize,
    F: Fn(&T),
{
    if json_output {
        println!(
            "{}",
            serde_json::to_string(value).expect("encode cli json output")
        );
    } else {
        printer(value);
    }
}

pub(crate) fn print_wallet_summary(summary: &WalletConfigSummary) {
    println!("wallet_address: {}", summary.wallet_address);
    println!("worker_id: {}", summary.worker_id);
    println!("network: {}", serde_name(&summary.network));
    println!("login: {}", summary.login);
    println!("daemon_running: {}", summary.daemon_running);
    println!("socket_path: {}", summary.socket_path);
    println!("state_path: {}", summary.state_path);
}

pub(crate) fn print_wallet_reward(snapshot: &WalletRewardSnapshot) {
    println!("account: {}", snapshot.account);
    println!("network: {}", serde_name(&snapshot.network));
    println!("confirmed_total: {}", snapshot.confirmed_total_display);
    println!(
        "estimated_pending_total: {}",
        snapshot
            .estimated_pending_total_display
            .clone()
            .unwrap_or_else(|| "-".to_string())
    );
    println!("paid_total: {}", snapshot.paid_total_display);
    println!(
        "confirmed_through_height: {}",
        snapshot
            .confirmed_through_height
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!("confirmed_blocks_24h: {}", snapshot.confirmed_blocks_24h);
    println!("orphaned_blocks_24h: {}", snapshot.orphaned_blocks_24h);
    println!("source_base_url: {}", snapshot.source_base_url);
}

pub(crate) fn print_doctor_report(report: &DoctorReport) {
    println!("wallet_configured: {}", report.wallet_configured);
    println!(
        "wallet_address: {}",
        report
            .wallet_address
            .clone()
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "worker_id: {}",
        report.worker_id.clone().unwrap_or_else(|| "-".to_string())
    );
    println!(
        "network: {}",
        report
            .network
            .as_ref()
            .map(serde_name)
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "login: {}",
        report.login.clone().unwrap_or_else(|| "-".to_string())
    );
    println!(
        "requested_mode: {}",
        report
            .requested_mode
            .as_ref()
            .map(serde_name)
            .unwrap_or_else(|| "-".to_string())
    );
    println!("daemon_running: {}", report.daemon_running);
    println!(
        "current_state: {}",
        report
            .current_state
            .map(|value| serde_name(&value))
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "current_pool: {}",
        report
            .current_pool
            .clone()
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "daemon_worker_name: {}",
        report
            .daemon_worker_name
            .clone()
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "last_error: {}",
        report.last_error.clone().unwrap_or_else(|| "-".to_string())
    );
    println!("socket_path: {}", report.socket_path);
    println!("state_path: {}", report.state_path);
}

pub(crate) fn print_status(snapshot: MinerSnapshot, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&snapshot).expect("encode snapshot json")
        );
        return;
    }

    println!("state: {}", serde_name(&snapshot.state));
    println!("connected: {}", snapshot.connected);
    println!("pool: {}", snapshot.pool);
    println!("worker_name: {}", snapshot.worker_name);
    println!("requested_mode: {}", serde_name(&snapshot.requested_mode));
    println!("auto_state: {}", serde_name(&snapshot.auto_state));
    println!(
        "auto_hold_reason: {}",
        snapshot
            .auto_hold_reason
            .as_ref()
            .map(serde_name)
            .unwrap_or_else(|| "-".to_string())
    );
    print_budget("effective_budget", &snapshot.effective_budget);
    println!("hashrate: {:.2} H/s", snapshot.hashrate);
    println!("hashrate_5m: {:.2} H/s", snapshot.hashrate_5m);
    println!("accepted: {}", snapshot.accepted);
    println!("accepted_5m: {}", snapshot.accepted_5m);
    println!("rejected: {}", snapshot.rejected);
    println!("rejected_5m: {}", snapshot.rejected_5m);
    println!("submitted: {}", snapshot.submitted);
    println!("submitted_5m: {}", snapshot.submitted_5m);
    println!("reject_rate_5m: {:.4}", snapshot.reject_rate_5m);
    println!("reconnects: {}", snapshot.reconnects);
    println!("uptime_secs: {}", snapshot.uptime_secs);
    println!("system_cpu_percent: {:.1}", snapshot.system_cpu_percent);
    println!(
        "system_memory_percent: {:.1}",
        snapshot.system_memory_percent
    );
    println!(
        "system_cpu_percent_1m: {:.1}",
        snapshot.system_cpu_percent_1m
    );
    println!(
        "system_memory_percent_1m: {:.1}",
        snapshot.system_memory_percent_1m
    );
    println!(
        "last_error: {}",
        snapshot.last_error.unwrap_or_else(|| "-".to_string())
    );
}

pub(crate) fn format_event(event: &MinerEvent) -> String {
    match event {
        MinerEvent::Started { snapshot }
        | MinerEvent::Paused { snapshot }
        | MinerEvent::Resumed { snapshot }
        | MinerEvent::Stopped { snapshot }
        | MinerEvent::Reconnecting { snapshot }
        | MinerEvent::BudgetChanged { snapshot }
        | MinerEvent::ShareAccepted { snapshot } => format!(
            "{} state={} requested_mode={} auto_state={} budget={} accepted={} rejected={} hashrate={:.2}",
            event_type(event),
            serde_name(&snapshot.state),
            serde_name(&snapshot.requested_mode),
            serde_name(&snapshot.auto_state),
            budget_summary(&snapshot.effective_budget),
            snapshot.accepted,
            snapshot.rejected,
            snapshot.hashrate,
        ),
        MinerEvent::ShareRejected { snapshot, reason } => format!(
            "share_rejected state={} requested_mode={} auto_state={} budget={} reason={}",
            serde_name(&snapshot.state),
            serde_name(&snapshot.requested_mode),
            serde_name(&snapshot.auto_state),
            budget_summary(&snapshot.effective_budget),
            reason,
        ),
        MinerEvent::Error { snapshot, message } => format!(
            "error state={} requested_mode={} auto_state={} budget={} message={}",
            serde_name(&snapshot.state),
            serde_name(&snapshot.requested_mode),
            serde_name(&snapshot.auto_state),
            budget_summary(&snapshot.effective_budget),
            message,
        ),
    }
}

fn print_budget(label: &str, budget: &Budget) {
    println!(
        "{}: threads={} cpu_percent={} priority={}",
        label,
        budget.threads,
        budget.cpu_percent,
        serde_name(&budget.priority)
    );
}

fn budget_summary(budget: &Budget) -> String {
    format!(
        "threads={} cpu_percent={} priority={}",
        budget.threads,
        budget.cpu_percent,
        serde_name(&budget.priority)
    )
}

fn event_type(event: &MinerEvent) -> &'static str {
    match event {
        MinerEvent::Started { .. } => "started",
        MinerEvent::Paused { .. } => "paused",
        MinerEvent::Resumed { .. } => "resumed",
        MinerEvent::Stopped { .. } => "stopped",
        MinerEvent::Reconnecting { .. } => "reconnecting",
        MinerEvent::BudgetChanged { .. } => "budget_changed",
        MinerEvent::ShareAccepted { .. } => "share_accepted",
        MinerEvent::ShareRejected { .. } => "share_rejected",
        MinerEvent::Error { .. } => "error",
    }
}

fn serde_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .expect("encode serde name")
        .trim_matches('"')
        .to_string()
}
