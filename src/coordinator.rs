use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::types::{Status, Worker};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use chrono::Local;
use serde::{Deserialize, Serialize};

struct AppState {
    workers: HashMap<String, Worker>,
}

#[derive(Deserialize)]
struct RequestBody {
    worker_name: String,
}

#[derive(Serialize)]
struct HealthReport {
    health: &'static str,
    online_workers: u16,
}

pub async fn execute(port: u16) {
    run(port).await;
}

async fn run(port: u16) {
    let state = Arc::new(Mutex::new(AppState {
        workers: HashMap::new(),
    }));

    let app = Router::new()
        .route("/", get(root))
        .route("/api/health", get(health))
        .route("/api/workers", get(list_workers))
        .route("/api/workers/register", post(register))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Port unavailable");

    println!("Coordinator module executed at address: {}", addr);

    axum::serve(listener, app)
        .await
        .expect("failed to server content");
}

async fn root() -> &'static str {
    "Coordinator module is online\n"
}

async fn health(State(state): State<Arc<Mutex<AppState>>>) -> Json<HealthReport> {
    let count = state.lock().unwrap().workers.len() as u16;
    Json(HealthReport {
        health: "ok",
        online_workers: count,
    })
}

async fn list_workers(State(state): State<Arc<Mutex<AppState>>>) -> Json<Vec<Worker>> {
    let guard = state.lock().unwrap();
    let workers: Vec<Worker> = guard.workers.values().cloned().collect();
    Json(workers)
}

async fn register(State(state): State<Arc<Mutex<AppState>>>, Json(req): Json<RequestBody>) {
    let worker = Worker {
        name: req.worker_name.clone(),
        status: Status::Online,
        last_heartbeat: Local::now(),
        job_id: None,
    };
    println!("{:#?}", worker);
    let mut guard = state.lock().unwrap();
    guard.workers.insert(req.worker_name.clone(), worker);
    println!("Worker Registered");
}
