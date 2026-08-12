use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::{Json, Router, extract::{Path, State}, http::StatusCode, routing::post};
use reqwest::Client;
use serde::Deserialize;
use crate::types::{
    ClaimedJob, DeviceProfile, JobStatus, RegisterResponse, ReportRequest, RunRequest, RunResponse, WorkerRequest,
    WorkerStats,
};

const HEARTBEAT_INTERVAL_SECS: u64 = 2;

// Set once at startup from --coordinator/--executor flags, else env, else
// localhost — so `orchestrator worker --name w1 --coordinator http://IP:8080`
// is all a fresh laptop needs.
static COORDINATOR: OnceLock<String> = OnceLock::new();
static EXECUTOR: OnceLock<String> = OnceLock::new();

fn coordinator_url() -> &'static str {
    COORDINATOR.get().map(String::as_str).unwrap_or("http://127.0.0.1:8080")
}

fn executor_url() -> &'static str {
    EXECUTOR.get().map(String::as_str).unwrap_or("http://127.0.0.1:9000")
}

/// The unique id minted by the coordinator on first registration survives
/// restarts in this file, so a worker keeps its identity for life.
/// WORKER_STATE_DIR points it at a mounted volume in containers, where the
/// working directory is wiped on every rebuild.
fn id_file(name: &str) -> String {
    let dir = std::env::var("WORKER_STATE_DIR").unwrap_or_else(|_| ".".into());
    format!("{}/.worker-id-{name}", dir.trim_end_matches('/'))
}

fn load_worker_id(name: &str) -> Option<String> {
    std::fs::read_to_string(id_file(name))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn save_worker_id(name: &str, id: &str, ui: &crate::ui::Ui) {
    if let Err(error) = std::fs::write(id_file(name), id) {
        // not fatal: the worker runs fine, it just gets a new id after a restart
        ui.warn(format!("could not persist worker id: {error}"));
    }
}

pub async fn run(
    name: String,
    coordinator: Option<String>,
    executor: Option<String>,
    tags: Option<String>,
    mode: crate::ui::Mode,
) {
    let coord = coordinator
        .or_else(|| std::env::var("COORDINATOR_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8080".into());
    let exec = executor
        .or_else(|| std::env::var("EXECUTOR_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:9000".into());
    let _ = COORDINATOR.set(coord.trim_end_matches('/').to_string());
    let _ = EXECUTOR.set(exec.trim_end_matches('/').to_string());

    // capability labels this machine advertises; pipeline jobs with
    // `tags: [...]` only land on workers carrying all of them
    let tags: Vec<String> = tags
        .or_else(|| std::env::var("WORKER_TAGS").ok())
        .unwrap_or_default()
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let (ui, rx) = crate::ui::Ui::channel();
    match mode {
        crate::ui::Mode::Plain => {
            tokio::spawn(crate::ui::run_plain(rx));
        }
        crate::ui::Mode::Tui => {
            tokio::spawn(crate::tui::run(rx));
        }
    }

    let client = Client::new();

    // Route the executor's live tail through this process when it is local, so
    // the log has one origin. Falls back to the coordinator otherwise.
    let progress_base = if executor_is_local() {
        start_progress_sink(ui.clone(), client.clone()).await
    } else {
        None
    };
    if progress_base.is_none() && executor_is_local() {
        ui.warn("could not open a local progress port — live logs will bypass this worker");
    }

    let worker_id = register(&client, &name, &tags, &ui).await;
    ui.send(crate::ui::Event::Registered {
        name: name.clone(),
        id: worker_id.clone(),
        coordinator: coordinator_url().to_string(),
        executor: executor_url().to_string(),
        tags: tags.clone(),
    });

    let body = WorkerRequest {
        worker_name: name.clone(),
        worker_id: Some(worker_id),
        tags: tags.clone(),
        stats: None,
        device: None,
    };

    let hb_client = client.clone();
    let hb_ui = ui.clone();
    let hb_body = WorkerRequest {
        worker_name: body.worker_name.clone(),
        worker_id: body.worker_id.clone(),
        tags: tags.clone(),
        stats: None,
        device: None,
    };
    tokio::spawn(async move { heartbeat(&hb_client, &hb_body, &hb_ui).await; });

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        ticker.tick().await;
        if let Some(claimed) = claim(&client, &body).await {
            run_job(&client, &claimed, &ui, progress_base.as_ref()).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Progress fan-out
//
// The executor posts its accumulated log to a single `progress_url` every
// couple of seconds. Pointed straight at the coordinator, the worker never sees
// a line of its own output — so it cannot show one. Pointing it at the worker
// instead makes the worker the single origin: it derives the new tail for the
// local UI and forwards the same payload onward, so the terminal and the
// website can never disagree about what a job printed.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ProgressBody {
    output: String,
}

#[derive(Clone)]
struct ProgressState {
    ui: crate::ui::Ui,
    client: Client,
    /// bytes of each job's log already turned into a `JobLog` event
    seen: Arc<Mutex<HashMap<i64, usize>>>,
}

/// Only usable when the executor can reach us. Every documented setup runs the
/// executor beside its worker, but a remote one would fail to POST to our
/// loopback address, so that case keeps the old direct-to-coordinator path.
fn executor_is_local() -> bool {
    let host = executor_url()
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// Handle to the local progress endpoint.
struct ProgressSink {
    base: String,
    seen: Arc<Mutex<HashMap<i64, usize>>>,
}

impl ProgressSink {
    fn url_for(&self, job_id: i64) -> String {
        format!("{}/progress/{job_id}", self.base)
    }

    /// Drop a finished job's high-water mark. Without this the map grows for
    /// the life of the worker, which on a long-lived machine is a slow leak.
    fn finish(&self, job_id: i64) {
        self.seen.lock().unwrap().remove(&job_id);
    }
}

/// Bind an ephemeral loopback port for the executor to report into. `None` if
/// binding failed — we then fall back to the coordinator and lose only the
/// local view, never the site's.
async fn start_progress_sink(ui: crate::ui::Ui, client: Client) -> Option<ProgressSink> {
    let seen = Arc::new(Mutex::new(HashMap::new()));
    let state = ProgressState { ui, client, seen: seen.clone() };
    let app = Router::new()
        .route("/progress/{id}", post(on_progress))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
    let addr = listener.local_addr().ok()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Some(ProgressSink { base: format!("http://{addr}"), seen })
}

/// New text since the last payload for this job, advancing the high-water mark.
///
/// The executor resends the whole log each time, so the tail is everything past
/// the mark. A payload shorter than the mark means the log was replaced rather
/// than appended (a retry, or a truncating executor); treating it as entirely
/// new is both safe and correct, where slicing would panic.
fn advance(seen: &mut HashMap<i64, usize>, id: i64, output: &str) -> String {
    let mark = seen.entry(id).or_insert(0);
    // `get` rather than an index: the mark is a byte offset taken from a
    // previous payload, and if this one was rewritten rather than appended it
    // may fall inside a multi-byte character. Indexing would panic in a request
    // handler; falling back to the whole payload just re-shows a little text.
    let text = output.get(*mark..).unwrap_or(output).to_string();
    *mark = output.len();
    text
}

async fn on_progress(
    Path(id): Path<i64>,
    State(state): State<ProgressState>,
    Json(body): Json<ProgressBody>,
) -> StatusCode {
    // The executor sends the whole log each time, so the delta is whatever is
    // past the high-water mark. A shorter body than last time means the log was
    // rewritten rather than appended; treat it as fresh instead of slicing
    // out of bounds.
    let delta = advance(&mut state.seen.lock().unwrap(), id, &body.output);
    if !delta.is_empty() {
        state.ui.send(crate::ui::Event::JobLog { id, text: delta });
    }

    // Forwarded inline, not spawned: the executor posts sequentially, so
    // awaiting here preserves order. Spawning could let a longer, later
    // payload land before an earlier one and truncate the stored log.
    let endpoint = format!("{}/api/jobs/{id}/progress", coordinator_url());
    if let Err(error) = state
        .client
        .post(&endpoint)
        .json(&serde_json::json!({ "output": body.output }))
        .send()
        .await
    {
        state.ui.warn(format!("could not forward job {id} progress: {error}"));
    }
    StatusCode::OK
}

/// Describe the machine this worker runs on. Sampled once, at registration —
/// none of it changes while the process lives.
pub fn device_profile() -> DeviceProfile {
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let cpus = sys.cpus();
    let (disk_total_gb, disk_free_gb) = workspace_disk();

    DeviceProfile {
        // long_os_version is the pretty one ("Ubuntu 24.04.1 LTS"); fall back
        // to name + version rather than leaving the field blank
        os: sysinfo::System::long_os_version().or_else(|| {
            match (sysinfo::System::name(), sysinfo::System::os_version()) {
                (Some(n), Some(v)) => Some(format!("{n} {v}")),
                (n, v) => n.or(v),
            }
        }),
        os_id: sysinfo::System::distribution_id(),
        kernel: sysinfo::System::kernel_version(),
        host: sysinfo::System::host_name(),
        arch: sysinfo::System::cpu_arch(),
        cpu: cpus.first().map(|c| c.brand().trim().to_string()).filter(|b| !b.is_empty()),
        cpu_cores: cpus.len() as u16,
        cpu_physical: sysinfo::System::physical_core_count().map(|n| n as u16),
        cpu_mhz: cpus.first().map(|c| c.frequency()).unwrap_or(0),
        mem_total_mb: sys.total_memory() / (1024 * 1024),
        disk_total_gb,
        disk_free_gb,
        shell: std::env::var("SHELL").ok().or_else(|| std::env::var("ComSpec").ok()),
        agent: env!("CARGO_PKG_VERSION").to_string(),
        executor: if executor_is_local() { "local".to_string() } else { executor_url().to_string() },
    }
}

/// Space on the filesystem the worker actually builds in. Picking the longest
/// mount point that is a prefix of the working directory is how you find the
/// filesystem a path lives on without asking the OS twice; summing every
/// mounted disk would report space that no build can use.
fn workspace_disk() -> (u64, u64) {
    let Ok(cwd) = std::env::current_dir() else { return (0, 0) };
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let best = disks
        .list()
        .iter()
        .filter(|d| cwd.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    const GB: u64 = 1024 * 1024 * 1024;
    best.map(|d| (d.total_space() / GB, d.available_space() / GB)).unwrap_or((0, 0))
}

/// Register until the coordinator answers; returns the (possibly newly
/// minted) unique worker id.
async fn register(client: &Client, name: &str, tags: &[String], ui: &crate::ui::Ui) -> String {
    let endpoint = format!("{}/api/workers/register", coordinator_url());
    let req = WorkerRequest {
        worker_name: name.to_string(),
        worker_id: load_worker_id(name),
        tags: tags.to_vec(),
        stats: None,
        device: Some(device_profile()),
    };

    loop {
        match client.post(&endpoint).json(&req).send().await {
            Ok(response) => match response.json::<RegisterResponse>().await {
                Ok(r) => {
                    save_worker_id(name, &r.worker_id, ui);
                    return r.worker_id;
                }
                Err(error) => ui.send(crate::ui::Event::RegisterRetry {
                    reason: format!("bad response: {error}"),
                    retry_in_secs: 3,
                }),
            },
            Err(error) => ui.send(crate::ui::Event::RegisterRetry {
                reason: error.to_string(),
                retry_in_secs: 3,
            }),
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

/// Each heartbeat carries a fresh CPU/RAM sample, so the coordinator can
/// keep an accurate per-device usage history for the dashboard graphs.
async fn heartbeat(client: &Client, body: &WorkerRequest, ui: &crate::ui::Ui) {
    let endpoint = format!("{}/api/workers/heartbeat", coordinator_url());
    let mut sys = sysinfo::System::new();
    // prime the CPU counters — the first reading after new() is always 0
    sys.refresh_cpu_usage();
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    loop {
        ticker.tick().await;
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let total = sys.total_memory();
        let used = sys.used_memory();
        let stats = WorkerStats {
            cpu_pct: sys.global_cpu_usage(),
            mem_pct: if total > 0 { used as f32 / total as f32 * 100.0 } else { 0.0 },
            mem_used_mb: used / (1024 * 1024),
            mem_total_mb: total / (1024 * 1024),
        };
        ui.send(crate::ui::Event::Heartbeat {
            cpu_pct: stats.cpu_pct,
            mem_pct: stats.mem_pct,
            mem_used_mb: stats.mem_used_mb,
            mem_total_mb: stats.mem_total_mb,
        });
        let beat = WorkerRequest {
            worker_name: body.worker_name.clone(),
            worker_id: body.worker_id.clone(),
            tags: body.tags.clone(),
            stats: Some(stats),
            device: None,
        };
        if let Err(error) = client.post(&endpoint).json(&beat).send().await {
            ui.warn(format!("heartbeat failed: {error}"));
        }
    }
}

async fn claim(client: &Client, body: &WorkerRequest) -> Option<ClaimedJob> {
    let endpoint = format!("{}/api/jobs/claim", coordinator_url());
    match client.post(endpoint).json(body).send().await {
        Ok(resp) => resp.json::<Option<ClaimedJob>>().await.unwrap_or(None),
        Err(_) => None,
    }
}

async fn run_job(
    client: &Client,
    claimed: &ClaimedJob,
    ui: &crate::ui::Ui,
    progress: Option<&ProgressSink>,
) {
    let job = &claimed.job;
    let started = std::time::Instant::now();
    ui.send(crate::ui::Event::JobStarted {
        id: job.id,
        run_id: job.run_id,
        stage: job.stage.clone(),
        name: job.name.clone(),
        command: job.command.clone(),
    });

    // Per-run workspace: the executor clones REPO_URL@COMMIT_SHA once per
    // machine (empty dir when the run has no repo), pulls dependency
    // artifacts from the coordinator, and pushes declared outputs back —
    // that is how files cross device boundaries.
    let workspace = Some(format!("run-{}", job.run_id));
    let inputs = claimed
        .input_jobs
        .iter()
        .map(|dep| format!("{}/api/jobs/{dep}/artifacts", coordinator_url()))
        .collect();
    let upload_url = (!job.artifacts.is_empty())
        .then(|| format!("{}/api/jobs/{}/artifacts", coordinator_url(), job.id));

    // jobs default to 5 minutes; long ones (e.g. the self-deploy docker
    // build) raise it via env JOB_TIMEOUT — the executor caps at its
    // MAX_TIMEOUT either way
    let timeout = job
        .env
        .get("JOB_TIMEOUT")
        .and_then(|t| t.parse().ok())
        .unwrap_or(300);

    let request = RunRequest {
        command: job.command.clone(),
        timeout,
        env: job.env.clone(),
        workspace,
        repo_url: job.env.get("REPO_URL").cloned(),
        commit_sha: job.env.get("COMMIT_SHA").cloned(),
        branch: job.env.get("REPO_BRANCH").cloned(),
        inputs,
        outputs: job.artifacts.clone(),
        upload_url,
        // through this worker when it has a sink, else straight to the
        // coordinator exactly as before
        progress_url: Some(match progress {
            Some(sink) => sink.url_for(job.id),
            None => format!("{}/api/jobs/{}/progress", coordinator_url(), job.id),
        }),
    };

    let response = client
        .post(format!("{}/run", executor_url()))
        .json(&request)
        .send().await;

    // A dead or broken runner must still produce a report, otherwise the
    // job stays "running" on the coordinator forever.
    let result = match response {
        Ok(resp) => resp.json::<RunResponse>().await.unwrap_or_else(|error| RunResponse {
            output: format!("job runner returned an invalid response: {error}"),
            status: "failed".to_string(),
            exit_code: None,
        }),
        Err(error) => RunResponse {
            output: format!("could not reach job runner at {}: {error}", executor_url()),
            status: "failed".to_string(),
            exit_code: None,
        },
    };

    let status = match result.status.as_str() {
        "passed" => JobStatus::Passed,
        _ => JobStatus::Failed,
    };

    let report = ReportRequest {
        status: status.clone(),
        output: result.output,
        // runner may not send an exit code; derive one so the dashboard
        // always has a value to show
        exit_code: result.exit_code.or(Some(match status {
            JobStatus::Passed => 0,
            _ => 1,
        })),
    };
    let endpoint = format!("{}/api/jobs/{}/report", coordinator_url(), job.id);
    if let Err(error) = client.post(&endpoint).json(&report).send().await {
        ui.warn(format!("could not report job {}: {error}", job.id));
    }
    if let Some(sink) = progress {
        sink.finish(job.id);
    }
    ui.send(crate::ui::Event::JobFinished {
        id: job.id,
        name: job.name.clone(),
        passed: matches!(status, JobStatus::Passed),
        exit_code: report.exit_code,
        elapsed_ms: started.elapsed().as_millis(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The profile is shown to a human as fact, so the fields that cannot
    /// honestly be unknown on any supported platform must actually be filled.
    #[test]
    fn device_profile_describes_this_machine() {
        let d = device_profile();
        assert!(d.cpu_cores > 0, "a machine running this test has at least one core");
        assert!(d.mem_total_mb > 0, "and some memory");
        assert!(!d.arch.is_empty());
        assert!(!d.os_id.is_empty());
        assert_eq!(d.agent, env!("CARGO_PKG_VERSION"));
        // physical cores never exceed logical ones; a profile claiming
        // otherwise would be read as a hardware fault rather than a bug
        if let Some(p) = d.cpu_physical {
            assert!(p <= d.cpu_cores, "physical {p} > logical {}", d.cpu_cores);
        }
    }

    /// Free space is reported for one filesystem, so it can never exceed that
    /// filesystem's size — the bug summing every mount would produce.
    #[test]
    fn workspace_disk_reports_one_filesystem() {
        let (total, free) = workspace_disk();
        assert!(free <= total, "free {free}GB > total {total}GB");
    }

    #[test]
    fn advance_yields_only_the_new_tail() {
        let mut seen = HashMap::new();
        assert_eq!(advance(&mut seen, 1, "hello"), "hello");
        assert_eq!(advance(&mut seen, 1, "hello world"), " world");
        // unchanged payload: the executor polls faster than the job prints
        assert_eq!(advance(&mut seen, 1, "hello world"), "");
    }

    #[test]
    fn advance_keeps_jobs_independent() {
        let mut seen = HashMap::new();
        assert_eq!(advance(&mut seen, 1, "aaa"), "aaa");
        assert_eq!(advance(&mut seen, 2, "bbbb"), "bbbb");
        assert_eq!(advance(&mut seen, 1, "aaaZZ"), "ZZ");
    }

    #[test]
    fn advance_treats_a_shrunken_log_as_fresh_rather_than_panicking() {
        let mut seen = HashMap::new();
        advance(&mut seen, 7, "a long first log");
        assert_eq!(advance(&mut seen, 7, "short"), "short");
        assert_eq!(seen[&7], "short".len());
    }

    #[test]
    fn multibyte_output_does_not_split_a_character() {
        let mut seen = HashMap::new();
        assert_eq!(advance(&mut seen, 1, "✓ ok"), "✓ ok");
        assert_eq!(advance(&mut seen, 1, "✓ ok\n✕ bad"), "\n✕ bad");
    }

    #[test]
    fn a_rewritten_log_splitting_a_character_falls_back_instead_of_panicking() {
        let mut seen = HashMap::new();
        // 4 bytes of ASCII establishes the mark
        advance(&mut seen, 3, "abcd");
        // byte 4 lands inside the 3-byte '✓', so slicing there would panic
        let out = advance(&mut seen, 3, "ab✓cdefgh");
        assert_eq!(out, "ab✓cdefgh");
    }

    #[test]
    fn local_executor_detection() {
        // OnceLock, so this is the one place in the test binary that sets it
        let _ = EXECUTOR.set("http://127.0.0.1:9000".to_string());
        assert!(executor_is_local());
    }
}

#[cfg(test)]
mod fanout {
    use super::*;

    /// Stand-in coordinator that records what the worker forwarded.
    async fn stub_coordinator(seen: Arc<Mutex<Vec<String>>>) -> String {
        let app = Router::new().route(
            "/api/jobs/{id}/progress",
            post(move |Json(b): Json<ProgressBody>| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(b.output);
                    StatusCode::OK
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { let _ = axum::serve(l, app).await; });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn executor_progress_reaches_both_the_ui_and_the_coordinator() {
        let forwarded = Arc::new(Mutex::new(Vec::new()));
        let coord = stub_coordinator(forwarded.clone()).await;
        let _ = COORDINATOR.set(coord);

        let (ui, mut rx) = crate::ui::Ui::channel();
        let sink = start_progress_sink(ui, Client::new()).await.expect("sink binds");

        let client = Client::new();
        for payload in ["line one\n", "line one\nline two\n"] {
            client
                .post(sink.url_for(42))
                .json(&serde_json::json!({ "output": payload }))
                .send()
                .await
                .expect("executor can post");
        }

        // the UI sees deltas, not the whole log re-sent
        let mut chunks = Vec::new();
        while let Ok(ev) = tokio::time::timeout(
            std::time::Duration::from_millis(200), rx.recv()).await
        {
            match ev {
                Some(crate::ui::Event::JobLog { id, text }) => { assert_eq!(id, 42); chunks.push(text); }
                Some(_) => {}
                None => break,
            }
            if chunks.len() == 2 { break; }
        }
        assert_eq!(chunks, vec!["line one\n", "line two\n"]);

        // the coordinator still gets the accumulated form it expects
        let got = forwarded.lock().unwrap().clone();
        assert_eq!(got, vec!["line one\n", "line one\nline two\n"]);

        // and a finished job stops being tracked
        sink.finish(42);
        assert!(sink.seen.lock().unwrap().is_empty());
    }
}
