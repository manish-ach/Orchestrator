use std::sync::OnceLock;

use reqwest::Client;
use crate::types::{ClaimedJob, JobStatus, RegisterResponse, ReportRequest, RunRequest, RunResponse, WorkerRequest, WorkerStats};

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

fn save_worker_id(name: &str, id: &str) {
    if let Err(error) = std::fs::write(id_file(name), id) {
        println!("Could not persist worker id: {error}");
    }
}

pub async fn run(name: String, coordinator: Option<String>, executor: Option<String>, tags: Option<String>) {
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

    let client = Client::new();
    let worker_id = register(&client, &name, &tags).await;
    println!(
        "Worker '{name}' up — id {worker_id}, coordinator {}{}",
        coordinator_url(),
        if tags.is_empty() { String::new() } else { format!(", tags [{}]", tags.join(", ")) }
    );

    let body = WorkerRequest { worker_name: name.clone(), worker_id: Some(worker_id), tags: tags.clone(), stats: None };

    let hb_client = client.clone();
    let hb_body = WorkerRequest {
        worker_name: body.worker_name.clone(),
        worker_id: body.worker_id.clone(),
        tags: tags.clone(),
        stats: None,
    };
    tokio::spawn(async move { heartbeat(&hb_client, &hb_body).await; });

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        ticker.tick().await;
        if let Some(claimed) = claim(&client, &body).await {
            run_job(&client, &claimed).await;
        }
    }
}

/// Register until the coordinator answers; returns the (possibly newly
/// minted) unique worker id.
async fn register(client: &Client, name: &str, tags: &[String]) -> String {
    let endpoint = format!("{}/api/workers/register", coordinator_url());
    let req = WorkerRequest {
        worker_name: name.to_string(),
        worker_id: load_worker_id(name),
        tags: tags.to_vec(),
        stats: None,
    };

    loop {
        match client.post(&endpoint).json(&req).send().await {
            Ok(response) => match response.json::<RegisterResponse>().await {
                Ok(r) => {
                    save_worker_id(name, &r.worker_id);
                    return r.worker_id;
                }
                Err(error) => println!("Bad register response: {error}"),
            },
            Err(error) => println!("Cannot reach coordinator ({error}), retrying in 3s"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

/// Each heartbeat carries a fresh CPU/RAM sample, so the coordinator can
/// keep an accurate per-device usage history for the dashboard graphs.
async fn heartbeat(client: &Client, body: &WorkerRequest) {
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
        let beat = WorkerRequest {
            worker_name: body.worker_name.clone(),
            worker_id: body.worker_id.clone(),
            tags: body.tags.clone(),
            stats: Some(WorkerStats {
                cpu_pct: sys.global_cpu_usage(),
                mem_pct: if total > 0 { used as f32 / total as f32 * 100.0 } else { 0.0 },
                mem_used_mb: used / (1024 * 1024),
                mem_total_mb: total / (1024 * 1024),
            }),
        };
        if let Err(error) = client.post(&endpoint).json(&beat).send().await {
            println!("Heartbeat failed: {error}");
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

async fn run_job(client: &Client, claimed: &ClaimedJob) {
    let job = &claimed.job;
    println!("Running job: {} [{}]: {}", job.id, job.stage, job.command);

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
        inputs,
        outputs: job.artifacts.clone(),
        upload_url,
        progress_url: Some(format!("{}/api/jobs/{}/progress", coordinator_url(), job.id)),
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
    match client.post(&endpoint).json(&report).send().await {
        Ok(_) => println!("Reported job {} as {:?}", job.id, status),
        Err(error) => println!("Failed to report job {}: {error}", job.id),
    }
}
