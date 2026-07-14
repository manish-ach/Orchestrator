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
    /// workspace paths uploaded to the coordinator when the job passes
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub has_artifacts: bool,
    /// worker capability labels this job requires (yml `tags:`)
    #[serde(default)]
    pub tags: Vec<String>,
    pub status: JobStatus,
    pub worker: Option<String>,
    #[serde(default)]
    pub worker_id: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub output: Option<String>,
}

/// What claim hands a worker: the job plus which dependency jobs have
/// artifacts waiting on the coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedJob {
    #[serde(flatten)]
    pub job: Job,
    #[serde(default)]
    pub input_jobs: Vec<i64>,
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
/// dashboard contract. Live state lives in Redis, keyed by the unique id
/// minted at first registration.
#[derive(Debug, Clone, Serialize)]
pub struct Worker {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub last_heartbeat: i64,
    /// ms epoch of the first registration — the dashboard shows uptime
    pub registered_at: i64,
    /// capability labels (`--tags heavy,docker`) that `tags:` in a
    /// pipeline yml matches against
    pub tags: Vec<String>,
    pub job_id: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct WorkerRequest {
    pub worker_name: String,
    /// unique id from a previous registration; absent on first contact
    #[serde(default)]
    pub worker_id: Option<String>,
    /// capability labels; only meaningful on register
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct RegisterResponse {
    pub worker_id: String,
}

/// Worker -> executor. Beyond the command itself: which per-run workspace
/// to run in (auto-cloned from repo_url@commit_sha), which artifact
/// bundles to pull in first, and which paths to upload where afterwards.
#[derive(Serialize)]
pub struct RunRequest {
    pub command: String,
    pub timeout: u32,
    pub env: std::collections::HashMap<String, String>,
    pub workspace: Option<String>,
    pub repo_url: Option<String>,
    pub commit_sha: Option<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub upload_url: Option<String>,
    /// coordinator endpoint the executor POSTs the growing log to while
    /// the job runs, so the dashboard can tail it live
    pub progress_url: Option<String>,
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
