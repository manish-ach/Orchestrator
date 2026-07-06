use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum TriggerKind {
    Manual,
    Webhook,
    Schedule
}

impl TriggerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerKind::Manual => "manual",
            TriggerKind::Webhook => "webhook",
            TriggerKind::Schedule => "schedule",
        }
    }

    pub fn from_str(s: &str) -> TriggerKind {
        match s {
            "webhook" => TriggerKind::Webhook,
            "schedule" => TriggerKind::Schedule,
            _ => TriggerKind::Manual,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Passed,
    Failed,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Passed => "passed",
            JobStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> JobStatus {
        match s {
            "running" => JobStatus::Running,
            "passed" => JobStatus::Passed,
            "failed" => JobStatus::Failed,
            _ => JobStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub run_id: i64,
    pub stage: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    pub status: JobStatus,
    pub worker: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub id: i64,
    pub pipeline: String,
    pub repo: String,
    pub pipeline_file: String,
    pub trigger: TriggerKind,
    pub commit: Option<Commit>,
    pub status: JobStatus,
    pub created_at: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub jobs: Vec<Job>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ReportRequest {
    pub status: JobStatus,
    pub output: String,
    pub exit_code: Option<i32>,
}

#[derive(Serialize)]
pub struct HealthReport {
    pub health: &'static str,
    pub online_workers: u16,
}

/// Worker as the API serves it; `last_heartbeat` is ms epoch per the
/// dashboard contract. Live state lives in Redis.
#[derive(Debug, Clone, Serialize)]
pub struct Worker {
    pub name: String,
    pub status: Status,
    pub last_heartbeat: i64,
    pub job_id: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct WorkerRequest {
    pub worker_name: String,
}

#[derive(Serialize)]
pub struct RunRequest {
    pub command: String,
    pub timeout: u32,
    pub env: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct RunResponse {
    pub output: String,
    pub status: String,
    pub exit_code: Option<i32>, // runners that predate exit codes omit this
}

#[derive(Serialize)]
pub struct CalendarDay {
    pub date: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageShare {
    pub name: String,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contributor {
    pub login: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRef {
    pub name: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub name: String,
    pub description: String,
    pub language: String,
    pub branch: String,
    pub owner: String,
    pub remote: Option<String>,
    pub languages: Vec<LanguageShare>,
    pub contributors: Vec<Contributor>,
    pub pipelines: Vec<PipelineRef>,
}

#[derive(Deserialize)]
pub struct AddRepoRequest {
    pub remote: String,
}

/// Optional body for POST /api/pipelines/trigger. With `repo`, the
/// coordinator pulls that repo's pipeline YAML from Forgejo; without it,
/// it falls back to the local pipeline.yml (then a built-in default).
#[derive(Deserialize, Default)]
pub struct TriggerRequest {
    pub repo: Option<String>,
}
