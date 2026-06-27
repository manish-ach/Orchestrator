use reqwest::Client;
use crate::types::WorkerRequest;

const COORDINATOR_URL: &str = "http://127.0.0.1:8080";
const HEARTBEAT_INTERVAL_SECS: u64 = 2;

pub async fn run(name: String) {
    let client = Client::new();
    let body = WorkerRequest{
        worker_name: name,
    };
    register(&client, &body).await;
    heartbeat(&client, &body).await;
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