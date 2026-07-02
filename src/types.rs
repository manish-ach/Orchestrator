use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: u32,
    pub stage_name: String,
    pub command: String,
    pub status: JobStatus,
    pub assigned_worker: Option<String>
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ReportRequest {
    pub status: JobStatus,
    pub output: String,
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
    pub job_id: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct WorkerRequest {
    pub worker_name: String,
}

