use reqwest::Client;
use serde::Serialize;

#[derive(Serialize)]
struct RequestBody {
    worker_name: String,
}

pub async fn register(name: String) {
    let coordinator = "http://127.0.0.1:8080";
    let client = Client::new();
    let register_endpoint = format!("{coordinator}/api/workers/register");
    let heartbeat_endpoint = format!("{coordinator}/api/workers/heartbeat");

    let body = RequestBody {
        worker_name: name.clone(),
    };

    let res = client.post(register_endpoint).json(&body).send().await;

    match res {
        Ok(r) => println!("Worker registered: {}", r.status()),
        Err(e) => println!("Error: {}", e),
    }

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
    loop {
        ticker.tick().await;

        let res = client.post(&heartbeat_endpoint).json(&body).send().await;

        match res {
            Ok(_r) => println!("Status::Online..."),
            Err(e) => {
                println!("heartbeat failed: {}", e);
                break;
            }
        }
    }
}
