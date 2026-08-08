// Shared domain types. These mirror the coordinator contract documented in
// dashboard/README.md — if a field changes here, change it there too.

export type JobStatus = 'pending' | 'running' | 'passed' | 'failed';
export type RunStatus = JobStatus;
export type WorkerState = 'online' | 'offline';
export type Trigger = 'webhook' | 'manual' | 'schedule';

export interface Commit {
  sha: string;
  message: string;
  author: string;
  /** files touched by the commit — the overview feed filters on *.yml/*.yaml */
  files: string[];
}

export interface Job {
  id: number;
  run_id: number;
  stage: string;
  name: string;
  command: string;
  /** names of jobs this one depends on — the DAG edges */
  needs?: string[];
  status: JobStatus;
  worker: string | null;
  /**
   * when every `needs` had passed and a placement was found — the point the job
   * could have started. `started_at - ready_at` is the queue wait, and it is not
   * derivable from anything else: a requeue clears `started_at`, and jobs with no
   * `needs` are ready at run creation rather than after some parent finished.
   */
  ready_at: number | null;
  /** times a worker died mid-job and the reconciler handed it back */
  requeue_count: number;
  started_at: number | null;
  finished_at: number | null;
  exit_code: number | null;
  /** One line explaining a failure, extracted by the coordinator on report. */
  first_error?: string | null;
  /**
   * Full captured log. Absent from list responses by design — a run list
   * carrying every job's build output is megabytes per poll. Fetch it from the
   * run/job endpoints when you actually need it.
   */
  output?: string | null;
  /** declared artifact paths; uploaded to the coordinator when passed */
  artifacts?: string[];
  has_artifacts?: boolean;
  /** worker capability labels this job requires (yml `tags:`) */
  tags?: string[];
  /** mock-only: planned duration used by the simulator */
  planned?: number;
}

export interface Run {
  id: number;
  pipeline: string;
  repo: string;
  pipeline_file: string;
  trigger: Trigger;
  /** branch this run built; null for runs created before it was recorded */
  branch: string | null;
  commit: Commit | null;
  status: RunStatus;
  created_at: number;
  started_at: number;
  finished_at: number | null;
  jobs: Job[];
}

/** Machine stats a worker samples and ships with every heartbeat. */
export interface WorkerStats {
  /** whole-machine CPU usage, 0–100 */
  cpu_pct: number;
  /** used / total physical memory, 0–100 */
  mem_pct: number;
  mem_used_mb: number;
  mem_total_mb: number;
}

/** One stored stats sample; `t` is ms epoch, cpu/mem are 0–100. */
export interface StatSample {
  t: number;
  cpu: number;
  mem: number;
}

/** GET /api/workers/stats — recent sample history of one worker. */
export interface WorkerStatsSeries {
  id: string;
  name: string;
  status: WorkerState;
  samples: StatSample[];
}

/**
 * What machine a worker is. Sampled once at registration, so nothing here
 * moves while the agent lives. Fields are optional wherever a platform can
 * fail to answer — the UI says "unknown" rather than inventing a value.
 */
export interface DeviceProfile {
  os: string | null;
  /** lowercase distribution id — the key the OS logo is chosen by */
  os_id: string;
  kernel: string | null;
  host: string | null;
  arch: string;
  cpu: string | null;
  cpu_cores: number;
  cpu_physical: number | null;
  cpu_mhz: number;
  mem_total_mb: number;
  /** the filesystem the workspace lives on, not every mounted disk */
  disk_total_gb: number;
  disk_free_gb: number;
  shell: string | null;
  agent: string;
  executor: string;
}

export interface Worker {
  /** unique id minted at first registration */
  id?: string;
  name: string;
  status: WorkerState;
  last_heartbeat: number;
  /** ms epoch of first registration — used for the uptime readout */
  registered_at?: number;
  /** capability labels the worker advertised (--tags heavy,docker) */
  tags?: string[];
  job_id: number | null;
  /** latest machine stats from the heartbeat; absent until one arrives */
  stats?: WorkerStats | null;
  /** absent for workers that registered before the profile existed */
  device?: DeviceProfile | null;
}

/** One job a worker executed, flattened with the run it belonged to. */
export interface WorkerJob {
  job_id: number;
  run_id: number;
  repo: string;
  pipeline: string;
  stage: string;
  name: string;
  status: JobStatus;
  started_at: number | null;
  finished_at: number | null;
}

/** GET /api/workers/{name}/activity — the per-device page's history. */
export interface WorkerActivity {
  name: string;
  /** a year of daily job counts, oldest first; empty days are present as 0 */
  calendar: CalendarDay[];
  total_jobs: number;
  passed: number;
  failed: number;
  /** wall-clock ms this worker spent executing */
  busy_ms: number;
  median_ms: number | null;
  /** most recent jobs, newest first */
  recent: WorkerJob[];
}

export interface Overview {
  workers: Worker[];
  runs: Run[];
}

export interface LogLine {
  t: string;
  err: boolean;
  ok: boolean;
}

export interface JobDetail {
  run: Run;
  job: Job;
  log: LogLine[];
}

/** One job as the pipeline file declares it — a definition, not an execution. */
export interface PipelineJobRef {
  name: string;
  stage: string;
  needs: string[];
  tags: string[];
}

export interface PipelineRef {
  name: string;
  file: string;
  /** declared stage order; empty when the file could not be parsed */
  stages: string[];
  jobs: PipelineJobRef[];
  /** why the definition is missing, when it is */
  parse_error: string | null;
  /** cron expression from the file's top-level `schedule:` */
  schedule: string | null;
}

/**
 * Last time this repo's webhook actually reached the coordinator. The point is
 * to tell "quiet" apart from "broken" — without it, a repo nobody has pushed to
 * and one whose webhook URL is wrong look identical.
 */
export interface WebhookDelivery {
  last_at: number;
  event: string | null;
  /** 'accepted' started a run; 'rejected' still proves the hook is wired up */
  status: string;
  detail: string | null;
}

export interface Contributor {
  login: string;
  name: string;
}

export interface LanguageShare {
  name: string;
  pct: number;
}

export interface Repo {
  name: string;
  description: string;
  language: string;
  branch: string;
  owner: string;
  remote: string | null;
  languages: LanguageShare[];
  contributors: Contributor[];
  pipelines: PipelineRef[];
  webhook?: WebhookDelivery | null;
}

export interface CalendarDay {
  /** YYYY-MM-DD */
  date: string;
  count: number;
}

// ---- insights ---------------------------------------------------------------
// Aggregates the coordinator derives from the runs/jobs tables. Every panel is
// keyed by something other than a run id — a stage, a job name, a worker, a day
// — which is exactly what keeps this page from repeating Control center or Runs.

export interface StageCost {
  stage: string;
  jobs: number;
  /** median stage wall time: last finish minus first start within a run */
  median_ms: number;
  p90_ms: number;
  runs: number;
}

export interface FlakyJob {
  name: string;
  repo: string;
  /** commits where this job produced both a pass and a fail */
  flips: number;
  /** recent outcomes, oldest first — 'p' passed, 'f' failed */
  recent: string;
}

export interface TrendPoint {
  date: string;
  runs: number;
  p50_ms: number;
  p90_ms: number;
}

export interface WaitPoint {
  date: string;
  wait_ms: number;
  exec_ms: number;
}

export interface WorkerCost {
  worker: string;
  runs: number;
  median_ms: number;
  pass_pct: number | null;
}

export interface JobSkew {
  job: string;
  workers: WorkerCost[];
}

export interface FailureCause {
  cause: string;
  job: string;
  exit_code: number | null;
  count: number;
}

export interface Insights {
  range_days: number;
  runs: number;
  passed: number;
  failed: number;
  median_ms: number | null;
  p90_ms: number | null;
  /** median time from a pipeline going red to its next green */
  recovery_ms: number | null;
  longest_red_ms: number | null;
  /** a year of daily run counts — deliberately wider than range_days */
  calendar: CalendarDay[];
  stages: StageCost[];
  flaky: FlakyJob[];
  trend: TrendPoint[];
  wait: WaitPoint[];
  skew: JobSkew | null;
  causes: FailureCause[];
}

/** Everything a data source must provide. Both mock and live implement this. */
export interface Api {
  overview(): Promise<Overview>;
  run(id: number | string): Promise<Run | null>;
  job(runId: number | string, jobId: number | string): Promise<JobDetail | null>;
  /** start a run; with `repo`, the coordinator uses that repo's pipeline YAML */
  trigger(repo?: string): Promise<{ id: number }>;
  repos(): Promise<Repo[]>;
  /** register a repo by its Forgejo URL; resolves to the fetched repo */
  addRepo(remote: string): Promise<Repo>;
  /** unregister a repo (its runs stay in history) */
  deleteRepo(name: string): Promise<void>;
  /** raw pipeline YAML for a repo, proxied through the coordinator */
  pipelineFile(repo: string, file?: string): Promise<{ file: string; content: string }>;
  /** rolling CPU/RAM history per worker, fed by heartbeats */
  workerStats(): Promise<WorkerStatsSeries[]>;
  /** what one machine has done: job history, calendar and totals */
  workerActivity(name: string): Promise<WorkerActivity>;
  /** aggregates for the Insights page over a window of `days` */
  insights(days: number): Promise<Insights>;
}
