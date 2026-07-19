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
  started_at: number | null;
  finished_at: number | null;
  exit_code: number | null;
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
  commit: Commit | null;
  status: RunStatus;
  created_at: number;
  started_at: number;
  finished_at: number | null;
  jobs: Job[];
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

export interface PipelineRef {
  name: string;
  file: string;
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
}

export interface CalendarDay {
  /** YYYY-MM-DD */
  date: string;
  count: number;
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
  calendar(): Promise<CalendarDay[]>;
}
