use super::wallet::{DoctorReport, WalletConfigSummary};
use crate::{AgentMethods, EventsSinceResponse, MinerCapabilities, MinerEvent, MinerSnapshot};
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
    println!("login: {}", summary.login);
    println!("daemon_running: {}", summary.daemon_running);
    println!("socket_path: {}", summary.socket_path);
    println!("state_path: {}", summary.state_path);
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
        "login: {}",
        report.login.clone().unwrap_or_else(|| "-".to_string())
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
    println!(
        "budget: threads={} cpu_percent={} priority={}",
        snapshot.current_budget.threads,
        snapshot.current_budget.cpu_percent,
        serde_name(&snapshot.current_budget.priority)
    );
    println!(
        "last_error: {}",
        snapshot.last_error.unwrap_or_else(|| "-".to_string())
    );
}

pub(crate) fn print_capabilities(caps: MinerCapabilities, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&caps).expect("encode capabilities json")
        );
        return;
    }
    println!("max_threads: {}", caps.max_threads);
    println!(
        "supported_modes: {}",
        caps.supported_modes
            .iter()
            .map(serde_name)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "supported_priorities: {}",
        caps.supported_priorities
            .iter()
            .map(serde_name)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("supports_cpu_percent: {}", caps.supports_cpu_percent);
    println!("supports_priority: {}", caps.supports_priority);
}

pub(crate) fn print_methods(methods: AgentMethods, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&methods).expect("encode methods json")
        );
        return;
    }
    println!("agent_api_version: {}", methods.agent_api_version);
    println!("agent_version: {}", methods.agent_version);
    for (name, method) in methods.methods {
        println!("{name}:");
        match method.params {
            Some(params) => {
                for (field, schema) in params.fields {
                    let mut line = format!(
                        "  param {}: {}{}",
                        field,
                        schema.type_name,
                        if schema.optional { "?" } else { "" }
                    );
                    if !schema.enum_values.is_empty() {
                        line.push_str(&format!(" enum={:?}", schema.enum_values));
                    }
                    println!("{line}");
                }
            }
            None => println!("  params: none"),
        }
        println!("  result: {}", method.result);
    }
}

pub(crate) fn print_events_since(response: EventsSinceResponse, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&response).expect("encode events since json")
        );
        return;
    }
    println!("next_seq: {}", response.next_seq);
    for envelope in response.events {
        println!("#{} {}", envelope.seq, format_event(&envelope.event));
    }
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
            "{} state={} connected={} accepted={} rejected={} hashrate={:.2}",
            event_type(event),
            serde_name(&snapshot.state),
            snapshot.connected,
            snapshot.accepted,
            snapshot.rejected,
            snapshot.hashrate
        ),
        MinerEvent::ShareRejected { snapshot, reason } => format!(
            "{} state={} accepted={} rejected={} reason={}",
            event_type(event),
            serde_name(&snapshot.state),
            snapshot.accepted,
            snapshot.rejected,
            reason
        ),
        MinerEvent::Error { snapshot, message } => format!(
            "{} state={} message={}",
            event_type(event),
            serde_name(&snapshot.state),
            message
        ),
    }
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

pub(crate) fn serde_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .expect("encode serde name")
        .trim_matches('"')
        .to_string()
}
