//! Worker presentation layer.
//!
//! Everything a worker wants to tell a human goes through [`Ui::send`] as an
//! [`Event`], never straight to stdout. One origin, several sinks: today a plain
//! line logger for headless runs, next a ratatui dashboard for the machines
//! being demoed. Log lines will fan out to the coordinator from the same place,
//! so the screen and the website can never disagree about what happened.

use std::io::IsTerminal;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Everything the worker reports. Renderers decide what to do with each one —
/// the plain logger prints most and drops the noisy ones, the TUI keeps state.
#[derive(Debug, Clone)]
pub enum Event {
    Registered {
        name: String,
        id: String,
        coordinator: String,
        executor: String,
        tags: Vec<String>,
    },
    /// Coordinator unreachable / rejected us; carries the retry delay.
    RegisterRetry {
        reason: String,
        retry_in_secs: u64,
    },
    JobStarted {
        id: i64,
        run_id: i64,
        stage: String,
        name: String,
        command: String,
    },
    /// A chunk of executor output. `text` is the *new* tail, not the whole log.
    JobLog {
        id: i64,
        text: String,
    },
    JobFinished {
        id: i64,
        name: String,
        passed: bool,
        exit_code: Option<i32>,
        elapsed_ms: u128,
    },
    /// Read by the TUI gauges; the plain renderer drops it as journal noise.
    Heartbeat {
        cpu_pct: f32,
        mem_pct: f32,
        mem_used_mb: u64,
        mem_total_mb: u64,
    },
    /// Something went wrong but the worker carries on.
    Warn(String),
}

/// Which renderer drains the channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Timestamped lines on stdout. What the server's worker uses.
    Plain,
    /// Full-screen dashboard. Requires a terminal.
    Tui,
}

impl Mode {
    /// A TUI is only meaningful on a real terminal. `--no-tui` forces plain;
    /// `--tui` forces it on (useful when a wrapper hides the tty); otherwise
    /// the presence of a terminal decides, so the server needs no flag at all.
    pub fn resolve(force_tui: bool, no_tui: bool) -> Mode {
        if no_tui {
            Mode::Plain
        } else if force_tui || std::io::stdout().is_terminal() {
            Mode::Tui
        } else {
            Mode::Plain
        }
    }
}

#[derive(Clone)]
pub struct Ui {
    tx: UnboundedSender<Event>,
}

impl Ui {
    /// Build the channel and hand back the receiver for a renderer to drain.
    pub fn channel() -> (Ui, UnboundedReceiver<Event>) {
        let (tx, rx) = unbounded_channel();
        (Ui { tx }, rx)
    }

    /// Fire and forget: a closed channel means the renderer stopped, which is
    /// never a reason to take the worker down with it.
    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn warn(&self, msg: impl Into<String>) {
        self.send(Event::Warn(msg.into()));
    }
}

/// Timestamped stdout renderer. Deliberately quiet: heartbeats and per-chunk
/// log spam would bury the events that matter in a server's journal.
pub async fn run_plain(mut rx: UnboundedReceiver<Event>) {
    while let Some(event) = rx.recv().await {
        let now = chrono::Local::now().format("%H:%M:%S");
        match event {
            Event::Registered { name, id, coordinator, executor, tags } => {
                let tags = if tags.is_empty() {
                    String::new()
                } else {
                    format!(" tags[{}]", tags.join(","))
                };
                println!("{now}  worker '{name}' up — id {id}{tags}");
                println!("{now}  coordinator {coordinator} · executor {executor}");
            }
            Event::RegisterRetry { reason, retry_in_secs } => {
                println!("{now}  cannot register: {reason} — retrying in {retry_in_secs}s");
            }
            Event::JobStarted { id, run_id, stage, name, command } => {
                println!("{now}  ▶ job {id} run {run_id} [{stage}/{name}] $ {command}");
            }
            Event::JobFinished { id, name, passed, exit_code, elapsed_ms } => {
                let mark = if passed { "✓" } else { "✕" };
                let code = exit_code.map(|c| format!(" exit {c}")).unwrap_or_default();
                println!(
                    "{now}  {mark} job {id} [{name}] {}{code} in {:.1}s",
                    if passed { "passed" } else { "failed" },
                    elapsed_ms as f64 / 1000.0
                );
            }
            Event::Warn(msg) => println!("{now}  ! {msg}"),
            // dropped on purpose: a heartbeat every 5s and a log chunk every 2s
            // would make a server log unreadable. The TUI shows both.
            Event::JobLog { .. } | Event::Heartbeat { .. } => {}
        }
    }
}
