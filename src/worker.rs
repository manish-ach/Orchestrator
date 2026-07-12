use reqwest::Client;
use crate::types::{Job, JobStatus, ReportRequest, RunRequest, RunResponse, WorkerRequest};

const HEARTBEAT_INTERVAL_SECS: u64 = 2;

// Overridable so containerized / multi-machine workers can find their peers.
fn coordinator_url() -> String {
    std::env::var("COORDINATOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into())
}

fn executor_url() -> String {
    std::env::var("EXECUTOR_URL").unwrap_or_else(|_| "http://127.0.0.1:9000".into())
}

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
    let endpoint = format!("{}/api/workers/register", coordinator_url());
    match client.post(endpoint).json(body).send().await {
        Ok(response) => println!("Worker successfully registered: {}", response.status()),
        Err(error) => println!("Error: {}", error),
    }
}

async fn heartbeat(client: &Client, body: &WorkerRequest) {
    let endpoint = format!("{}/api/workers/heartbeat", coordinator_url());
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
    let endpoint = format!("{}/api/jobs/claim", coordinator_url());
    match client.post(endpoint).json(body).send().await {
        Ok(resp) => resp.json::<Option<Job>>().await.unwrap_or(None),
        Err(_) => None,
    }
}

async fn run_job(client: &Client, job: &Job) {
    println!("Running job: {} [{}]: {}", job.id, job.stage, job.command);

    let response = client
        .post(format!("{}/run", executor_url()))
        .json(&RunRequest{ command: job.command.clone(), timeout: 300, env: job.env.clone() })
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