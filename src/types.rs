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
    /// when every `needs` had passed and a placement was found — the point the
    /// job could have started. `started_at - ready_at` is the queue wait.
    #[serde(default)]
    pub ready_at: Option<i64>,
    /// times a worker died mid-job and the reconciler handed it back
    #[serde(default)]
    pub requeue_count: i32,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    /// One line explaining a failure, extracted when the job reported. Present on
    /// list responses, where `output` deliberately is not.
    #[serde(default)]
    pub first_error: Option<String>,
    /// Full captured log. `None` on list responses — a run list carrying every
    /// job's build output is megabytes per poll.
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
    /// Branch this run built. `None` for runs created before the column
    /// existed — not backfillable, since the push payload is long gone.
    pub branch: Option<String>,
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

/// What machine a worker is. Sampled once at registration and stored
/// verbatim — none of it changes while the process lives, so re-sending it on
/// every heartbeat would be pure noise. Every field is optional or defaulted
/// because a platform that cannot answer should say so rather than invent:
/// the dashboard renders "unknown" for a `None`, which is honest, and a
/// fabricated CPU model is not.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceProfile {
    /// human OS string, e.g. "Ubuntu 24.04.1 LTS" or "macOS 15.5"
    pub os: Option<String>,
    /// lowercase distribution id ("ubuntu", "arch", "macos", "windows") —
    /// the key the dashboard picks an OS logo by
    pub os_id: String,
    pub kernel: Option<String>,
    /// machine hostname as the OS reports it, which need not equal `--name`
    pub host: Option<String>,
    pub arch: String,
    pub cpu: Option<String>,
    /// logical cores — the number that bounds how much this worker can run
    pub cpu_cores: u16,
    /// physical cores, when the platform distinguishes them
    pub cpu_physical: Option<u16>,
    /// nominal clock in MHz; 0 on platforms that do not report it
    pub cpu_mhz: u64,
    pub mem_total_mb: u64,
    /// space on the filesystem holding the worker's workspace, not the sum
    /// of every mounted disk — that is the number that runs out mid-build
    pub disk_total_gb: u64,
    pub disk_free_gb: u64,
    pub shell: Option<String>,
    /// version of this worker binary
    pub agent: String,
    /// where this worker sends commands; "local" when the executor runs on
    /// the same machine, otherwise the remote host
    pub executor: String,
}

/// Point-in-time machine stats a worker samples and sends with every
/// heartbeat, so the dashboard can graph real CPU/RAM per device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStats {
    /// whole-machine CPU usage, 0–100
    pub cpu_pct: f32,
    /// used / total physical memory, 0–100
    pub mem_pct: f32,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
}

/// One stored stats sample; `t` is ms epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatSample {
    pub t: i64,
    pub cpu: f32,
    pub mem: f32,
}

/// GET /api/workers/stats — the recent sample history of one worker.
#[derive(Debug, Clone, Serialize)]
pub struct WorkerStatsSeries {
    pub id: String,
    pub name: String,
    pub status: Status,
    pub samples: Vec<StatSample>,
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
    /// latest machine stats from the heartbeat; None until one arrives
    pub stats: Option<WorkerStats>,
    /// what machine this is; None for workers that registered before the
    /// profile existed, or that run an older agent
    pub device: Option<DeviceProfile>,
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
    /// machine stats; only sent with heartbeats
    #[serde(default)]
    pub stats: Option<WorkerStats>,
    /// what machine this is; only sent with register
    #[serde(default)]
    pub device: Option<DeviceProfile>,
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
    /// pushed branch, checked out when the trigger carried no sha —
    /// manual and scheduled runs record none on purpose
    pub branch: Option<String>,
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

#[derive(Debug, Clone, Serialize)]
pub struct CalendarDay {
    pub date: String,
    pub count: u32,
}

/// One job this worker executed, flattened with the run it belonged to so the
/// device page can name the work without a second request per row.
#[derive(Debug, Clone, Serialize)]
pub struct WorkerJob {
    pub job_id: i64,
    pub run_id: i64,
    pub repo: String,
    pub pipeline: String,
    pub stage: String,
    pub name: String,
    pub status: JobStatus,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

/// GET /api/workers/{name}/activity — everything the per-device page shows
/// that is not already in the registry entry. Keyed by worker *name* rather
/// than id: the name is the machine, and a box that lost its id file and
/// re-registered is still the same machine to the person looking at it.
#[derive(Debug, Clone, Serialize)]
pub struct WorkerActivity {
    pub name: String,
    /// a year of daily job counts, oldest first — the contribution calendar
    pub calendar: Vec<CalendarDay>,
    pub total_jobs: i64,
    pub passed: i64,
    pub failed: i64,
    /// wall-clock ms this worker spent executing jobs
    pub busy_ms: i64,
    /// median job duration in ms; None until it has finished one
    pub median_ms: Option<i64>,
    /// most recent jobs, newest first
    pub recent: Vec<WorkerJob>,
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

/// One job as the pipeline file declares it — the definition, not an
/// execution. Enough for the dashboard to draw a pipeline's shape before it
/// has ever run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineJobRef {
    pub name: String,
    pub stage: String,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRef {
    pub name: String,
    pub file: String,
    /// Stage order as declared. Empty when the file could not be parsed —
    /// which is itself worth showing, since it means the pipeline cannot run.
    #[serde(default)]
    pub stages: Vec<String>,
    #[serde(default)]
    pub jobs: Vec<PipelineJobRef>,
    /// Why the definition is missing, when it is. `None` means it parsed.
    #[serde(default)]
    pub parse_error: Option<String>,
    /// Cron expression from the file's top-level `schedule:`, verbatim.
    #[serde(default)]
    pub schedule: Option<String>,
}

/// The last time this repo's webhook actually reached the coordinator.
///
/// The point is to tell "quiet" apart from "broken": a repo nobody has pushed
/// to and a repo whose webhook URL is wrong look identical without this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub last_at: i64,
    /// Forgejo's `X-Forgejo-Event` header, when it sent one.
    pub event: Option<String>,
    /// `accepted` when it started a run, `rejected` when the coordinator
    /// refused it — a rejected delivery is still proof the hook is wired up.
    pub status: String,
    /// why it was rejected; `None` when accepted
    pub detail: Option<String>,
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
    /// Filled in on read from the deliveries table, never from the stored
    /// blob — the 2-minute Forgejo refresh overwrites that blob wholesale and
    /// would otherwise wipe this every time.
    #[serde(default)]
    pub webhook: Option<WebhookDelivery>,
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
