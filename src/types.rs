use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Offline,
}

#[derive(Serialize)]
pub struct HealthReport {
    pub health: &'static str,
    pub online_workers: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct Worker {
    pub name: String,
    pub status: Status,
    pub last_heartbeat: DateTime<Local>,
    pub job_id: Option<u16>,
}

#[derive(Serialize, Deserialize)]
pub struct WorkerRequest {
    pub worker_name: String,
}