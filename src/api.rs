use axum::extract::State;
use axum::{Json, Router};
use axum::routing::{get, post};
use crate::state::SharedState;
use crate::types::{HealthReport, Worker, WorkerRequest};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/api/health", get(health))
        .route("/api/workers/list_workers", get(list_workers))
        .route("/api/workers/register", post(register))
        .route("/api/workers/heartbeat", post(heartbeat))
        .with_state(state)
}

async fn root() -> &'static str {
    "Coordinator Online"
}

async fn health(State(state): State<SharedState>) -> Json<HealthReport> {
    let online_workers = state.lock().unwrap().online_workers();
    Json(HealthReport{
        health: "Ok",
        online_workers,
    })
}

async fn list_workers(State(state): State<SharedState>) -> Json<Vec<Worker>> {
    let workers = state.lock().unwrap().list_workers();
    Json(workers)
}

async fn register(State(state): State<SharedState>, Json(req): Json<WorkerRequest>) {
    state.lock().unwrap().register(req.worker_name.clone());
    println!("Worker {} registered successfully!", req.worker_name);
}

async fn heartbeat(State(state): State<SharedState>, Json(req): Json<WorkerRequest>) {
    let known = state.lock().unwrap().heartbeat(&req.worker_name);
    if !known {
        println!("Worker {} not known!", req.worker_name);
    }
}