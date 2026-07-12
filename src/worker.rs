use std::sync::OnceLock;

use reqwest::Client;
use crate::types::{ClaimedJob, JobStatus, RegisterResponse, ReportRequest, RunRequest, RunResponse, WorkerRequest};

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
fn id_file(name: &str) -> String {
    format!(".worker-id-{name}")
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

pub async fn run(name: String, coordinator: Option<String>, executor: Option<String>) {
    let coord = coordinator
        .or_else(|| std::env::var("COORDINATOR_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8080".into());
    let exec = executor
        .or_else(|| std::env::var("EXECUTOR_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:9000".into());
    let _ = COORDINATOR.set(coord.trim_end_matches('/').to_string());
    let _ = EXECUTOR.set(exec.trim_end_matches('/').to_string());

    let client = Client::new();
    let worker_id = register(&client, &name).await;
    println!("Worker '{name}' up — id {worker_id}, coordinator {}", coordinator_url());

    let body = WorkerRequest { worker_name: name.clone(), worker_id: Some(worker_id) };

    let hb_client = client.clone();
    let hb_body = WorkerRequest { worker_name: body.worker_name.clone(), worker_id: body.worker_id.clone() };
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
async fn register(client: &Client, name: &str) -> String {
    let endpoint = format!("{}/api/workers/register", coordinator_url());
    let req = WorkerRequest { worker_name: name.to_string(), worker_id: load_worker_id(name) };

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

async fn heartbeat(client: &Client, body: &WorkerRequest) {
    let endpoint = format!("{}/api/workers/heartbeat", coordinator_url());
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    loop {
        ticker.tick().await;
        if let Err(error) = client.post(&endpoint).json(body).send().await {
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

    let request = RunRequest {
        command: job.command.clone(),
        timeout: 300,
        env: job.env.clone(),
        workspace,
        repo_url: job.env.get("REPO_URL").cloned(),
        commit_sha: job.env.get("COMMIT_SHA").cloned(),
        inputs,
        outputs: job.artifacts.clone(),
        upload_url,
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
