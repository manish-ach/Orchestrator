use reqwest::Client;
use crate::types::{Job, JobStatus, ReportRequest, RunRequest, RunResponse, WorkerRequest};

const COORDINATOR_URL: &str = "http://127.0.0.1:8080";
const JOB_RUNNER_URL: &str = "http://127.0.0.1:9000";
const HEARTBEAT_INTERVAL_SECS: u64 = 2;

pub async fn run(name: String) {
    let client = Client::new();
    let body = WorkerRequest{
        worker_name: name.clone(),
    };
    register(&client, &body).await;

    let hb_client = client.clone();
    let hb_body = WorkerRequest{ worker_name: name.clone()};
    tokio::spawn(async move { heartbeat(&hb_client, &hb_body).await; });

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        ticker.tick().await;
        if let Some(job) = claim(&client, &body).await {
            run_job(&client, &job).await;
        }
    }
}

async fn register(client: &Client, body: &WorkerRequest) {
    let endpoint = format!("{COORDINATOR_URL}/api/workers/register");
    match client.post(endpoint).json(body).send().await {
        Ok(response) => println!("Worker successfully registered: {}", response.status()),
        Err(error) => println!("Error: {}", error),
    }
}

async fn heartbeat(client: &Client, body: &WorkerRequest) {
    let endpoint = format!("{COORDINATOR_URL}/api/workers/heartbeat");
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    loop {
        ticker.tick().await;
        match client.post(&endpoint).json(body).send().await {
            Ok(r) => println!("Status::Online...[{}]", r.status()),
            Err(error) => {
                println!("Error: {}", error);
                break;
            }
        }
    }
}

async fn claim(client: &Client, body: &WorkerRequest) -> Option<Job> {
    let endpoint = format!("{COORDINATOR_URL}/api/jobs/claim");
    match client.post(endpoint).json(body).send().await {
        Ok(resp) => resp.json::<Option<Job>>().await.unwrap_or(None),
        Err(_) => None,
    }
}

async fn run_job(client: &Client, job: &Job) {
    println!("Running job: {} [{}]: {}", job.id, job.stage_name, job.command);

    let result: RunResponse = client
        .post(format!("{JOB_RUNNER_URL}/run"))
        .json(&RunRequest{ command: job.command.clone(), timeout: 300})
        .send().await.unwrap()
        .json().await.unwrap();

    let status = match result.status.as_str() {
        "passed" => JobStatus::Passed,
        _ => JobStatus::Failed,
    };

    let report = ReportRequest {
        status: status.clone(),
        output: result.output,
    };
    let endpoint = format!("{COORDINATOR_URL}/api/jobs/{}/report", job.id);
    let _ = client.post(&endpoint).json(&report).send().await.unwrap();
    println!("Reported job {} as {:?}", job.id, status);
}