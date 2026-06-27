use axum::extract::State;
use axum::{Json, Router};
use axum::routing::{get, post};
use crate::state::SharedState;
use crate::types::{HealthReport, Job, Worker, WorkerRequest};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/api/health", get(health))
        .route("/api/workers", get(list_workers))
        .route("/api/workers/register", post(register))
        .route("/api/workers/heartbeat", post(heartbeat))
        .route("/api/pipelines/trigger", post(trigger_pipeline))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/claim", post(claim_job))
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

async fn trigger_pipeline(State(state): State<SharedState>) {
    state.lock().unwrap().trigger_pipeline();
    println!("Triggered pipeline!");
}

async fn list_jobs(State(state): State<SharedState>) -> Json<Vec<Job>> {
    let jobs = state.lock().unwrap().list_jobs();
    Json(jobs)
}

async fn claim_job(State(state): State<SharedState>, Json(req): Json<WorkerRequest>) -> Json<Option<Job>> {
    let job = state.lock().unwrap().claim_job(req.worker_name);
    Json(job)
}