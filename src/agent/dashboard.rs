use super::wallet::{WalletAgent, WalletAgentError};
use crate::{BudgetMode, EventsSinceResponse, MinerSnapshot};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::style::Print;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use std::collections::VecDeque;
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

const DASHBOARD_EVENT_LIMIT: usize = 12;
const DASHBOARD_REFRESH: Duration = Duration::from_millis(500);

pub async fn run_dashboard(agent: WalletAgent) -> io::Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let mut terminal = TerminalGuard;
    let result = run_dashboard_loop(agent, &mut stdout).await;
    terminal.restore(&mut stdout)?;
    result
}

struct TerminalGuard;

impl TerminalGuard {
    fn restore(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(stdout, Show, LeaveAlternateScreen)?;
        stdout.flush()
    }
}

#[derive(Default)]
struct DashboardState {
    snapshot: Option<MinerSnapshot>,
    events: VecDeque<String>,
    since_seq: u64,
    status_message: Option<String>,
    wallet_input: Option<String>,
}

async fn run_dashboard_loop(agent: WalletAgent, stdout: &mut Stdout) -> io::Result<()> {
    let mut state = DashboardState::default();
    let mut last_refresh = Instant::now() - DASHBOARD_REFRESH;
    loop {
        if last_refresh.elapsed() >= DASHBOARD_REFRESH {
            refresh_dashboard(&agent, &mut state).await;
            render_dashboard(stdout, &state)?;
            last_refresh = Instant::now();
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if handle_key(&agent, &mut state, key.code).await? {
                        return Ok(());
                    }
                    render_dashboard(stdout, &state)?;
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
                    state.status_message = Some(match agent.update_wallet(&wallet).await {
                        Ok(summary) => format!("wallet updated: {}", summary.login),
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
                state.events.push_back(format!(
                    "#{} {}",
                    event.seq,
                    serde_json::to_string(&event.event).unwrap_or_else(|_| "event".to_string())
                ));
            }
        }
        Err(WalletAgentError::NotConfigured) => {}
        Err(err) => state.status_message = Some(err.to_string()),
    }
}

fn render_dashboard(stdout: &mut Stdout, state: &DashboardState) -> io::Result<()> {
    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    queue!(
        stdout,
        Print("stc-mint-agent dashboard\n"),
        Print("q quit | s start | x stop | p pause | r resume | 1 conservative | 2 idle | 3 balanced | 4 aggressive | w update wallet\n\n"),
    )?;

    match &state.snapshot {
        Some(snapshot) => {
            queue!(
                stdout,
                Print(format!("state:          {:?}\n", snapshot.state)),
                Print(format!("connected:      {}\n", snapshot.connected)),
                Print(format!("pool:           {}\n", snapshot.pool)),
                Print(format!("worker_name:    {}\n", snapshot.worker_name)),
                Print(format!(
                    "mode:           {}\n",
                    snapshot
                        .current_mode
                        .as_ref()
                        .map(|value| format!("{value:?}").to_lowercase())
                        .unwrap_or_else(|| "custom_budget".to_string())
                )),
                Print(format!("hashrate:       {:.2} H/s\n", snapshot.hashrate)),
                Print(format!("hashrate_5m:    {:.2} H/s\n", snapshot.hashrate_5m)),
                Print(format!(
                    "accepted:       {} (5m {})\n",
                    snapshot.accepted, snapshot.accepted_5m
                )),
                Print(format!(
                    "rejected:       {} (5m {})\n",
                    snapshot.rejected, snapshot.rejected_5m
                )),
                Print(format!(
                    "submitted:      {} (5m {})\n",
                    snapshot.submitted, snapshot.submitted_5m
                )),
                Print(format!("reject_rate_5m: {:.3}\n", snapshot.reject_rate_5m)),
                Print(format!("reconnects:     {}\n", snapshot.reconnects)),
                Print(format!(
                    "budget:         threads={} cpu_percent={} priority={:?}\n",
                    snapshot.current_budget.threads,
                    snapshot.current_budget.cpu_percent,
                    snapshot.current_budget.priority
                )),
            )?;
            if let Some(last_error) = &snapshot.last_error {
                queue!(stdout, Print(format!("last_error:     {}\n", last_error)))?;
            }
        }
        None => {
            queue!(
                stdout,
                Print("wallet not configured yet. Press w to set a payout wallet.\n")
            )?;
        }
    }

    queue!(stdout, Print("\nrecent events:\n"))?;
    if state.events.is_empty() {
        queue!(stdout, Print("  (no events)\n"))?;
    } else {
        for event in &state.events {
            queue!(stdout, Print(format!("  {}\n", event)))?;
        }
    }

    queue!(stdout, Print("\n"))?;
    if let Some(buffer) = &state.wallet_input {
        queue!(stdout, Print(format!("wallet> {}_", buffer)),)?;
    } else if let Some(message) = &state.status_message {
        queue!(stdout, Print(format!("status: {}\n", message)))?;
    }
    stdout.flush()
}
