use super::render::format_event;
use super::wallet::WalletAgent;
use super::wallet_support::WalletAgentError;
use crate::{BudgetMode, EventsSinceResponse, MinerSnapshot, WalletAddress};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

const DASHBOARD_EVENT_LIMIT: usize = 12;
const DASHBOARD_REFRESH: Duration = Duration::from_millis(500);

pub async fn run_dashboard(agent: WalletAgent) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_dashboard_loop(agent, &mut terminal).await;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

#[derive(Default)]
struct DashboardState {
    snapshot: Option<MinerSnapshot>,
    events: VecDeque<String>,
    since_seq: u64,
    status_message: Option<String>,
    wallet_input: Option<String>,
}

async fn run_dashboard_loop(
    agent: WalletAgent,
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
) -> io::Result<()> {
    let mut state = DashboardState::default();
    let mut last_refresh = Instant::now() - DASHBOARD_REFRESH;
    loop {
        if last_refresh.elapsed() >= DASHBOARD_REFRESH {
            refresh_dashboard(&agent, &mut state).await;
            render_dashboard(terminal, &state)?;
            last_refresh = Instant::now();
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if handle_key(&agent, &mut state, key.code).await? {
                        return Ok(());
                    }
                    render_dashboard(terminal, &state)?;
                }
                Event::Resize(_, _) => {
                    render_dashboard(terminal, &state)?;
                }
                _ => {}
            }
        }
    }
}

async fn handle_key(
    agent: &WalletAgent,
    state: &mut DashboardState,
    code: KeyCode,
) -> io::Result<bool> {
    if let Some(buffer) = state.wallet_input.as_mut() {
        match code {
            KeyCode::Esc => state.wallet_input = None,
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Enter => {
                let wallet = buffer.trim().to_string();
                state.wallet_input = None;
                if wallet.is_empty() {
                    state.status_message = Some("wallet update cancelled".to_string());
                } else {
                    state.status_message = Some(match WalletAddress::parse(wallet) {
                        Ok(wallet_address) => match agent.set_wallet(wallet_address, None).await {
                            Ok(summary) => format!("wallet updated: {}", summary.login),
                            Err(err) => err.to_string(),
                        },
                        Err(err) => err.to_string(),
                    });
                }
            }
            KeyCode::Char(ch) => {
                buffer.push(ch);
            }
            _ => {}
        }
        return Ok(false);
    }

    let outcome = match code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('s') => Some(agent.start().await.map(|_| "miner started".to_string())),
        KeyCode::Char('x') => Some(agent.stop().await.map(|_| "miner stopped".to_string())),
        KeyCode::Char('p') => Some(agent.pause().await.map(|_| "miner paused".to_string())),
        KeyCode::Char('r') => Some(agent.resume().await.map(|_| "miner resumed".to_string())),
        KeyCode::Char('a') => Some(
            agent
                .set_mode(BudgetMode::Auto)
                .await
                .map(|_| "mode -> auto".to_string()),
        ),
        KeyCode::Char('1') => Some(
            agent
                .set_mode(BudgetMode::Conservative)
                .await
                .map(|_| "mode -> conservative".to_string()),
        ),
        KeyCode::Char('2') => Some(
            agent
                .set_mode(BudgetMode::Idle)
                .await
                .map(|_| "mode -> idle".to_string()),
        ),
        KeyCode::Char('3') => Some(
            agent
                .set_mode(BudgetMode::Balanced)
                .await
                .map(|_| "mode -> balanced".to_string()),
        ),
        KeyCode::Char('4') => Some(
            agent
                .set_mode(BudgetMode::Aggressive)
                .await
                .map(|_| "mode -> aggressive".to_string()),
        ),
        KeyCode::Char('w') => {
            state.wallet_input = Some(String::new());
            None
        }
        _ => None,
    };
    if let Some(outcome) = outcome {
        state.status_message = Some(match outcome {
            Ok(message) => message,
            Err(err) => err.to_string(),
        });
    }
    Ok(false)
}

async fn refresh_dashboard(agent: &WalletAgent, state: &mut DashboardState) {
    match agent.status().await {
        Ok(snapshot) => state.snapshot = Some(snapshot),
        Err(WalletAgentError::NotConfigured) => state.snapshot = None,
        Err(err) => state.status_message = Some(err.to_string()),
    }

    match agent.events_since(state.since_seq).await {
        Ok(EventsSinceResponse { next_seq, events }) => {
            state.since_seq = next_seq;
            for event in events {
                if state.events.len() >= DASHBOARD_EVENT_LIMIT {
                    state.events.pop_front();
                }
                state
                    .events
                    .push_back(format!("#{} {}", event.seq, format_event(&event.event)));
            }
        }
        Err(WalletAgentError::NotConfigured) => {}
        Err(err) => state.status_message = Some(err.to_string()),
    }
}

fn render_dashboard(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    state: &DashboardState,
) -> io::Result<()> {
    terminal
        .draw(|frame| {
            let area = frame.size();
            let root = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(12),
                    Constraint::Length(10),
                    Constraint::Length(3),
                ])
                .split(area);

            frame.render_widget(render_header(), root[0]);

            let middle = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
                .split(root[1]);
            frame.render_widget(render_overview(state), middle[0]);
            frame.render_widget(render_metrics(state), middle[1]);
            frame.render_widget(render_events(state), root[2]);
            frame.render_widget(render_footer(state), root[3]);

            if state.wallet_input.is_some() {
                let popup = centered_rect(70, 5, area);
                frame.render_widget(Clear, popup);
                frame.render_widget(render_wallet_popup(state), popup);
            }
        })
        .map(|_| ())
}

fn render_header() -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from("stc-mint-agent dashboard"),
        Line::from("q quit | s start | x stop | p pause | r resume | w wallet"),
        Line::from("a auto | 1 conservative | 2 idle | 3 balanced | 4 aggressive"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Keys"))
    .wrap(Wrap { trim: true })
}

fn render_overview(state: &DashboardState) -> Paragraph<'static> {
    let lines = match &state.snapshot {
        Some(snapshot) => vec![
            line_kv("state", serde_name(&snapshot.state)),
            line_kv("connected", yes_no(snapshot.connected)),
            line_kv("pool", snapshot.pool.clone()),
            line_kv("worker", snapshot.worker_name.clone()),
            line_kv("requested_mode", serde_name(&snapshot.requested_mode)),
            line_kv("auto_state", serde_name(&snapshot.auto_state)),
            line_kv(
                "auto_hold_reason",
                snapshot
                    .auto_hold_reason
                    .as_ref()
                    .map(serde_name)
                    .unwrap_or_else(|| "-".to_string()),
            ),
            line_kv(
                "budget",
                format!(
                    "threads={} cpu={} priority={}",
                    snapshot.effective_budget.threads,
                    snapshot.effective_budget.cpu_percent,
                    serde_name(&snapshot.effective_budget.priority)
                ),
            ),
            line_kv(
                "last_error",
                snapshot
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "-".to_string()),
            ),
        ],
        None => vec![
            Line::from("wallet not configured yet"),
            Line::from("press w to set a payout wallet"),
        ],
    };
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Overview"))
        .wrap(Wrap { trim: true })
}

fn render_metrics(state: &DashboardState) -> Paragraph<'static> {
    let lines = match &state.snapshot {
        Some(snapshot) => vec![
            line_kv("hashrate", format!("{:.2} H/s", snapshot.hashrate)),
            line_kv("hashrate_5m", format!("{:.2} H/s", snapshot.hashrate_5m)),
            line_kv(
                "accepted",
                format!("{} (5m {})", snapshot.accepted, snapshot.accepted_5m),
            ),
            line_kv(
                "rejected",
                format!("{} (5m {})", snapshot.rejected, snapshot.rejected_5m),
            ),
            line_kv(
                "submitted",
                format!("{} (5m {})", snapshot.submitted, snapshot.submitted_5m),
            ),
            line_kv("reject_rate_5m", format!("{:.3}", snapshot.reject_rate_5m)),
            line_kv("reconnects", snapshot.reconnects.to_string()),
            line_kv("uptime_secs", snapshot.uptime_secs.to_string()),
            line_kv("system_cpu", format!("{:.1}%", snapshot.system_cpu_percent)),
            line_kv(
                "system_memory",
                format!("{:.1}%", snapshot.system_memory_percent),
            ),
            line_kv(
                "system_cpu_1m",
                format!("{:.1}%", snapshot.system_cpu_percent_1m),
            ),
            line_kv(
                "system_memory_1m",
                format!("{:.1}%", snapshot.system_memory_percent_1m),
            ),
        ],
        None => vec![Line::from("no miner metrics yet")],
    };
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Metrics"))
        .wrap(Wrap { trim: true })
}

fn render_events(state: &DashboardState) -> List<'static> {
    let items = if state.events.is_empty() {
        vec![ListItem::new("(no events)")]
    } else {
        state
            .events
            .iter()
            .cloned()
            .map(ListItem::new)
            .collect::<Vec<_>>()
    };
    List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Recent Events"),
    )
}

fn render_footer(state: &DashboardState) -> Paragraph<'static> {
    let message = state
        .status_message
        .clone()
        .unwrap_or_else(|| "ready".to_string());
    Paragraph::new(message)
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .wrap(Wrap { trim: true })
}

fn render_wallet_popup(state: &DashboardState) -> Paragraph<'static> {
    let input = state.wallet_input.clone().unwrap_or_default();
    Paragraph::new(vec![
        Line::from("Update wallet"),
        Line::from("Enter payout wallet address and press Enter. Esc cancels."),
        Line::from(format!("wallet> {}", input)),
    ])
    .block(Block::default().borders(Borders::ALL).title("Wallet"))
    .wrap(Wrap { trim: true })
}

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(height),
            Constraint::Percentage(50),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn line_kv(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(format!("{label}: {}", value.into()))
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn serde_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .expect("encode serde name")
        .trim_matches('"')
        .to_string()
}
