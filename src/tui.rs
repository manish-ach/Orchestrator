//! Full-screen worker dashboard.
//!
//! A second renderer over the same [`crate::ui::Event`] stream the plain logger
//! drains, so the terminal and the website are fed by one origin. Four regions
//! in one window: identity and link state on top, the job in flight and what
//! finished recently on the left, live executor output on the right, machine
//! load along the bottom.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::{
    crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::ui::Event;

/// Keep the tail bounded — a `cargo build` can emit tens of thousands of lines
/// and only the recent ones are worth a screen.
const MAX_LOG_LINES: usize = 4000;
const MAX_RECENT: usize = 12;

const LIME: Color = Color::Rgb(0xc3, 0xe7, 0x5f);
const DIM: Color = Color::Rgb(0x8a, 0x8f, 0x82);
const OK: Color = Color::Rgb(0x7c, 0xc4, 0x6b);
const BAD: Color = Color::Rgb(0xe0, 0x6c, 0x5c);
const WARN: Color = Color::Rgb(0xd9, 0xa0, 0x4e);

struct Finished {
    name: String,
    passed: bool,
    exit_code: Option<i32>,
    elapsed_ms: u128,
}

struct Current {
    id: i64,
    run_id: i64,
    stage: String,
    name: String,
    command: String,
    started: Instant,
}

#[derive(Default)]
struct App {
    name: String,
    id: String,
    coordinator: String,
    executor: String,
    tags: Vec<String>,
    registered: bool,
    last_issue: Option<String>,
    last_beat: Option<Instant>,
    cpu: f32,
    mem_pct: f32,
    mem_used_mb: u64,
    mem_total_mb: u64,
    current: Option<Current>,
    recent: VecDeque<Finished>,
    log: VecDeque<String>,
    /// trailing bytes of the last chunk that had no newline yet
    partial: String,
}

impl App {
    fn apply(&mut self, ev: Event) {
        match ev {
            Event::Registered { name, id, coordinator, executor, tags } => {
                self.name = name;
                self.id = id;
                self.coordinator = coordinator;
                self.executor = executor;
                self.tags = tags;
                self.registered = true;
                self.last_issue = None;
            }
            Event::RegisterRetry { reason, .. } => {
                self.registered = false;
                self.last_issue = Some(reason);
            }
            Event::JobStarted { id, run_id, stage, name, command } => {
                // a new job owns the pane; the previous job's tail has already
                // been shipped to the coordinator and is readable on the site
                self.log.clear();
                self.partial.clear();
                self.current = Some(Current { id, run_id, stage, name, command, started: Instant::now() });
            }
            // A job's last progress POST can land after the next job has
            // started; without this guard that tail would be appended to the
            // wrong pane.
            Event::JobLog { id, text } => {
                if self.current.as_ref().map(|c| c.id) == Some(id) {
                    self.push_log(&text);
                }
            }
            Event::JobFinished { name, passed, exit_code, elapsed_ms, .. } => {
                self.current = None;
                self.recent.push_front(Finished { name, passed, exit_code, elapsed_ms });
                self.recent.truncate(MAX_RECENT);
            }
            Event::Heartbeat { cpu_pct, mem_pct, mem_used_mb, mem_total_mb } => {
                self.last_beat = Some(Instant::now());
                self.cpu = cpu_pct;
                self.mem_pct = mem_pct;
                self.mem_used_mb = mem_used_mb;
                self.mem_total_mb = mem_total_mb;
            }
            Event::Warn(msg) => self.last_issue = Some(msg),
        }
    }

    /// Chunks arrive mid-line, so hold the remainder until its newline lands —
    /// otherwise a progress bar or a partial `test … ok` would show as two rows.
    fn push_log(&mut self, text: &str) {
        self.partial.push_str(text);
        while let Some(nl) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=nl).collect();
            self.log.push_back(line.trim_end_matches(['\n', '\r']).to_string());
        }
        while self.log.len() > MAX_LOG_LINES {
            self.log.pop_front();
        }
    }
}

/// Same keyword rules the dashboard's log viewer uses, so a line is the same
/// colour in the terminal and in the browser.
fn line_style(text: &str) -> Style {
    let lower = text.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("panicked") || lower.contains("fatal") {
        Style::default().fg(BAD)
    } else if lower.contains("warning") || lower.contains("warn:") {
        Style::default().fg(WARN)
    } else if lower.starts_with("[executor]") {
        Style::default().fg(DIM).add_modifier(Modifier::ITALIC)
    } else if lower.ends_with(" ok") || lower.contains("finished") || lower.contains("passed") {
        Style::default().fg(OK)
    } else {
        Style::default().fg(Color::Rgb(0xd2, 0xd6, 0xcb))
    }
}

fn bar(pct: f32, width: usize) -> String {
    let filled = ((pct / 100.0) * width as f32).round().clamp(0.0, width as f32) as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn secs(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 60 { format!("{}m {:02}s", s / 60, s % 60) } else { format!("{s}s") }
}

pub async fn run(mut rx: UnboundedReceiver<Event>) {
    // installs a panic hook that restores the terminal, so a crash does not
    // leave the user with a dead shell
    let mut term = ratatui::init();
    let mut app = App::default();

    // crossterm's read() blocks, so it lives on its own thread and reports back
    let (key_tx, mut key_rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        loop {
            if event::poll(Duration::from_millis(200)).unwrap_or(false) {
                if let Ok(TermEvent::Key(k)) = event::read() {
                    if key_tx.send(k).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut redraw = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            Some(ev) = rx.recv() => app.apply(ev),
            Some(key) = key_rx.recv() => {
                if key.kind == KeyEventKind::Press {
                    let quit = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL));
                    if quit {
                        break;
                    }
                }
            }
            _ = redraw.tick() => {}
        }
        let _ = term.draw(|f| draw(f, &app));
    }

    ratatui::restore();
    // worker::run never returns, so quitting the dashboard has to end the process
    std::process::exit(0);
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(6),
        Constraint::Length(3),
    ])
    .areas(f.area());

    header(f, head, app);

    let [left, right] = Layout::horizontal([Constraint::Length(34), Constraint::Min(30)]).areas(body);
    let [job, recent] = Layout::vertical([Constraint::Length(9), Constraint::Min(3)]).areas(left);
    current_job(f, job, app);
    recent_jobs(f, recent, app);
    log_pane(f, right, app);

    footer(f, foot, app);
}

fn header(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let (dot, state) = if !app.registered {
        (BAD, "connecting")
    } else if app.last_beat.map(|b| b.elapsed() > Duration::from_secs(10)).unwrap_or(false) {
        (WARN, "stale")
    } else {
        (OK, "connected")
    };
    let beat = app
        .last_beat
        .map(|b| format!("  beat {}", secs(b.elapsed())))
        .unwrap_or_default();

    let name = if app.name.is_empty() { "worker".to_string() } else { app.name.clone() };
    let tags = if app.tags.is_empty() { "none".into() } else { app.tags.join(", ") };

    let lines = vec![
        Line::from(vec![
            Span::styled(name, Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  id {}", app.id), Style::default().fg(DIM)),
            Span::raw("   "),
            Span::styled("●", Style::default().fg(dot)),
            Span::styled(format!(" {state}{beat}"), Style::default().fg(DIM)),
        ]),
        Line::from(vec![
            Span::styled("coordinator ", Style::default().fg(DIM)),
            Span::raw(app.coordinator.clone()),
            Span::styled("   executor ", Style::default().fg(DIM)),
            Span::raw(app.executor.clone()),
        ]),
        Line::from(vec![
            Span::styled("tags ", Style::default().fg(DIM)),
            Span::raw(tags),
            match &app.last_issue {
                Some(m) => Span::styled(format!("   ! {m}"), Style::default().fg(WARN)),
                None => Span::raw(""),
            },
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(DIM))),
        area,
    );
}

fn current_job(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(" current job ", Style::default().fg(LIME)));
    let body = match &app.current {
        Some(c) => vec![
            Line::from(vec![
                Span::styled("▶ ", Style::default().fg(WARN)),
                Span::styled(c.name.clone(), Style::default().add_modifier(Modifier::BOLD)),
            ]),
            // job id deliberately omitted: run + stage + name identify it, and the
            // elapsed line below is worth more than an id nobody types
            Line::from(Span::styled(format!("run #{}  ·  {}", c.run_id, c.stage), Style::default().fg(DIM))),
            Line::from(""),
            Line::from(Span::styled(format!("$ {}", c.command), Style::default().fg(LIME))),
            Line::from(""),
            Line::from(Span::styled(format!("{} elapsed", secs(c.started.elapsed())), Style::default().fg(DIM))),
        ],
        None => vec![
            Line::from(""),
            Line::from(Span::styled("  idle — waiting for work", Style::default().fg(DIM).add_modifier(Modifier::ITALIC))),
        ],
    };
    f.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(block), area);
}

fn recent_jobs(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(" recent ", Style::default().fg(LIME)));
    let rows: Vec<Line> = if app.recent.is_empty() {
        vec![Line::from(Span::styled("  nothing yet", Style::default().fg(DIM).add_modifier(Modifier::ITALIC)))]
    } else {
        app.recent
            .iter()
            .map(|j| {
                let (mark, colour) = if j.passed { ("✓", OK) } else { ("✕", BAD) };
                let code = match (j.passed, j.exit_code) {
                    (false, Some(c)) => format!(" exit {c}"),
                    _ => String::new(),
                };
                Line::from(vec![
                    Span::styled(format!("{mark} "), Style::default().fg(colour)),
                    Span::raw(j.name.clone()),
                    Span::styled(
                        format!("  {:.1}s{code}", j.elapsed_ms as f64 / 1000.0),
                        Style::default().fg(DIM),
                    ),
                ])
            })
            .collect()
    };
    f.render_widget(Paragraph::new(rows).block(block), area);
}

fn log_pane(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let title = match &app.current {
        Some(c) => format!(" {} · live ", c.name),
        None => " executor log ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(title, Style::default().fg(LIME)));

    // tail: show what fits, newest at the bottom, like a real terminal
    let rows = area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(rows);
    let mut lines: Vec<Line> = app
        .log
        .iter()
        .skip(start)
        .map(|l| Line::from(Span::styled(l.clone(), line_style(l))))
        .collect();
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  waiting for output…",
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )));
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn footer(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let mem = if app.mem_total_mb > 0 {
        format!(
            "{:.1}/{:.1} GB",
            app.mem_used_mb as f64 / 1024.0,
            app.mem_total_mb as f64 / 1024.0
        )
    } else {
        "—".into()
    };
    let line = Line::from(vec![
        Span::styled("cpu ", Style::default().fg(DIM)),
        Span::styled(bar(app.cpu, 14), Style::default().fg(LIME)),
        Span::raw(format!(" {:>3.0}%", app.cpu)),
        Span::styled("    mem ", Style::default().fg(DIM)),
        Span::styled(bar(app.mem_pct, 14), Style::default().fg(LIME)),
        Span::raw(format!(" {:>3.0}%  {mem}", app.mem_pct)),
        Span::styled("        q quit", Style::default().fg(DIM)),
    ]);
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(DIM))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn registered() -> App {
        let mut app = App::default();
        app.apply(Event::Registered {
            name: "beefy-1".into(),
            id: "3f9c".into(),
            coordinator: "http://ci.example.com".into(),
            executor: "http://127.0.0.1:9000".into(),
            tags: vec!["heavy".into(), "docker".into()],
        });
        app
    }

    #[test]
    fn idle_worker_shows_identity_and_says_it_is_idle() {
        let out = render(&registered(), 100, 24);
        assert!(out.contains("beefy-1"), "{out}");
        assert!(out.contains("http://ci.example.com"), "{out}");
        assert!(out.contains("heavy, docker"), "{out}");
        assert!(out.contains("connecting") || out.contains("connected"), "{out}");
        assert!(out.contains("idle"), "{out}");
        assert!(out.contains("q quit"), "{out}");
    }

    #[test]
    fn a_running_job_shows_its_command_and_streamed_output() {
        let mut app = registered();
        app.apply(Event::JobStarted {
            id: 7,
            run_id: 143,
            stage: "build".into(),
            name: "compile".into(),
            command: "cargo build --release".into(),
        });
        app.apply(Event::JobLog { id: 7, text: "Compiling orchestrator\n".into() });
        let out = render(&app, 100, 24);
        assert!(out.contains("compile"), "{out}");
        assert!(out.contains("cargo build --release"), "{out}");
        assert!(out.contains("run #143"), "{out}");
        assert!(out.contains("Compiling orchestrator"), "{out}");
    }

    #[test]
    fn finished_jobs_move_to_recent_with_their_outcome() {
        let mut app = registered();
        app.apply(Event::JobStarted {
            id: 7, run_id: 143, stage: "build".into(),
            name: "compile".into(), command: "cargo build".into(),
        });
        app.apply(Event::JobFinished {
            id: 7, name: "compile".into(), passed: false,
            exit_code: Some(101), elapsed_ms: 11_400,
        });
        let out = render(&app, 100, 24);
        assert!(out.contains("✕ compile"), "{out}");
        assert!(out.contains("exit 101"), "{out}");
        assert!(out.contains("idle"), "expected the job pane to clear: {out}");
    }

    #[test]
    fn a_late_chunk_from_a_finished_job_does_not_pollute_the_next_one() {
        let mut app = registered();
        app.apply(Event::JobStarted {
            id: 1, run_id: 1, stage: "build".into(),
            name: "first".into(), command: "a".into(),
        });
        app.apply(Event::JobFinished {
            id: 1, name: "first".into(), passed: true, exit_code: Some(0), elapsed_ms: 10,
        });
        app.apply(Event::JobStarted {
            id: 2, run_id: 1, stage: "test".into(),
            name: "second".into(), command: "b".into(),
        });
        // straggler POST for job 1 arrives after job 2 started
        app.apply(Event::JobLog { id: 1, text: "STRAGGLER\n".into() });
        app.apply(Event::JobLog { id: 2, text: "mine\n".into() });
        let out = render(&app, 100, 24);
        assert!(!out.contains("STRAGGLER"), "stale chunk leaked into the new pane: {out}");
        assert!(out.contains("mine"), "{out}");
    }

    #[test]
    fn partial_chunks_are_joined_into_whole_lines() {
        let mut app = App::default();
        app.push_log("test one ...");
        assert!(app.log.is_empty(), "a line without its newline must not render yet");
        app.push_log(" ok\ntest two ... ok\n");
        assert_eq!(app.log.len(), 2);
        assert_eq!(app.log[0], "test one ... ok");
    }

    #[test]
    fn log_is_capped_so_a_long_build_cannot_grow_without_bound() {
        let mut app = App::default();
        for i in 0..(MAX_LOG_LINES + 500) {
            app.push_log(&format!("line {i}\n"));
        }
        assert_eq!(app.log.len(), MAX_LOG_LINES);
        assert_eq!(app.log.back().unwrap(), &format!("line {}", MAX_LOG_LINES + 499));
    }

    /// Not an assertion — a way to eyeball the layout:
    ///   cargo test tui::tests::preview -- --ignored --nocapture
    #[test]
    #[ignore]
    fn preview() {
        let mut app = registered();
        app.apply(Event::Heartbeat { cpu_pct: 73.0, mem_pct: 61.0, mem_used_mb: 10_240, mem_total_mb: 16_384 });
        app.apply(Event::JobFinished { id: 5, name: "lint".into(), passed: true, exit_code: Some(0), elapsed_ms: 6_200 });
        app.apply(Event::JobFinished { id: 4, name: "migrate-db".into(), passed: false, exit_code: Some(1), elapsed_ms: 1_200 });
        app.apply(Event::JobStarted { id: 7, run_id: 143, stage: "test".into(),
            name: "unit-tests".into(), command: "cargo test --workspace --no-fail-fast".into() });
        for l in ["[executor] worker beefy-1 · linux/aarch64",
                  "   Compiling orchestrator v0.4.2 (/work)",
                  "warning: unused variable: `ctx`",
                  "    Finished `test` profile in 4.81s",
                  "running 340 tests",
                  "test pipeline::tests::parses_simple_stage ... ok",
                  "test store::tests::inserts_run ... ok",
                  "test worker::tests::reaper_marks_offline ... ok"] {
            app.apply(Event::JobLog { id: 7, text: format!("{l}\n") });
        }
        println!("\n{}\n", render(&app, 104, 26));
    }

    #[test]
    fn heartbeat_drives_the_load_gauges() {
        let mut app = registered();
        app.apply(Event::Heartbeat {
            cpu_pct: 73.0, mem_pct: 61.0, mem_used_mb: 10_240, mem_total_mb: 16_384,
        });
        let out = render(&app, 100, 24);
        assert!(out.contains("73%"), "{out}");
        assert!(out.contains("10.0/16.0 GB"), "{out}");
        assert!(out.contains('█'), "expected a filled gauge: {out}");
    }
}
