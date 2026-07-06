// Mock data source. Simulates the coordinator's future /api/runs contract,
// including one run that progresses in real time while the page polls.

import type {
  Api, CalendarDay, Commit, Job, JobDetail, LogLine, Overview, Repo, Run, Trigger, Worker,
} from './types';

interface PlanStep { stage: string; name: string; command: string; dur: number }
interface PipelineDef { repo: string; file: string; plan: PlanStep[]; commits: Commit[] }
interface HistorySpec { pipe: string; ageMin: number; fail?: string; trigger?: Trigger }
interface Persisted { epoch: number; extra: { start: number; commit: Commit }[] }


const WORKERS: string[] = ['rechek', 'nimesh-tp', 'prabhat-hp', 'rohan-mac', 'lab-05'];

// Forgejo repos. Each repo owns one or more pipelines (workflows).
const REPOS = {
  'CI-CD-orchestrator': {
    description: 'Distributed CI/CD platform — Rust coordinator, job queue, worker registry, reaper.',
    language: 'Rust',
    branch: 'main',
    owner: 'manish',
    remote: 'https://git.manishacharya.name.np/manish/CI-CD-orchestrator',
    languages: [
      { name: 'Rust', pct: 71.3 },
      { name: 'JavaScript', pct: 12.8 },
      { name: 'CSS', pct: 8.1 },
      { name: 'Shell', pct: 4.6 },
      { name: 'Other', pct: 3.2 },
    ],
    contributors: [
      { login: 'manish', name: 'Manish Acharya' },
      { login: 'rohan', name: 'Rohan Shrestha' },
    ],
  },
  'yaml-parser': {
    description: 'Pipeline definitions — reads pipeline.yml, validates stages and needs:, emits an ordered plan.',
    language: 'Python',
    branch: 'main',
    owner: 'prabhat',
    remote: 'https://git.manishacharya.name.np/prabhat/yaml-parser',
    languages: [
      { name: 'Python', pct: 96.4 },
      { name: 'Other', pct: 3.6 },
    ],
    contributors: [
      { login: 'prabhat', name: 'Prabhat Adhikari' },
    ],
  },
  'command-executor': {
    description: 'FastAPI job runner — executes commands as subprocesses, captures logs, timeout and cancel.',
    language: 'Python',
    branch: 'main',
    owner: 'nimesh',
    remote: 'https://git.manishacharya.name.np/nimesh/command-executor',
    languages: [
      { name: 'Python', pct: 87.2 },
      { name: 'Dockerfile', pct: 8.3 },
      { name: 'Other', pct: 4.5 },
    ],
    contributors: [
      { login: 'nimesh', name: 'Nimesh Giri' },
      { login: 'manish', name: 'Manish Acharya' },
    ],
  },
};

// One pipeline per team module — different stages, commands and commits so
// repo grouping reads like a real installation.
const PIPELINES: Record<string, PipelineDef> = {
  'orchestrator-ci': {
    repo: 'CI-CD-orchestrator',
    file: '.orchestrator/ci.yml',
    plan: [
      { stage: 'build',  name: 'compile',      command: 'cargo build --release',                 dur: 34000 },
      { stage: 'build',  name: 'clippy',       command: 'cargo clippy -- -D warnings',           dur: 21000 },
      { stage: 'test',   name: 'unit-tests',   command: 'cargo test --lib',                      dur: 28000 },
      { stage: 'test',   name: 'parser-tests', command: 'pytest yaml-parser/ -q',                dur: 14000 },
      { stage: 'deploy', name: 'package',      command: 'docker build -t orchestrator:latest .', dur: 41000 },
      { stage: 'deploy', name: 'deploy',       command: './scripts/demo-pipeline.sh deploy',     dur: 12000 },
    ],
    commits: [
      { sha: 'f81d3aa', message: 'feat: requeue jobs from dead workers', author: 'manish', files: ['src/state.rs', 'src/coordinator.rs'] },
      { sha: '9c04b17', message: 'fix: reaper marks offline after 5s, not 50s', author: 'manish', files: ['src/state.rs'] },
      { sha: 'a52f908', message: 'fix: claim race when two workers poll at once', author: 'manish', files: ['src/state.rs'] },
      { sha: '5e8ab63', message: 'fix: heartbeat race when worker re-registers', author: 'manish', files: ['src/api.rs'] },
      { sha: 'd10c4f2', message: 'ci: docker-compose service + deploy stage in pipeline', author: 'manish', files: ['docker-compose.yml', '.orchestrator/ci.yml'] },
      { sha: '6ad0327', message: 'ci: cache cargo build between jobs', author: 'rohan', files: ['.orchestrator/ci.yml'] },
    ],
  },
  'orchestrator-nightly': {
    repo: 'CI-CD-orchestrator',
    file: '.orchestrator/nightly.yml',
    plan: [
      { stage: 'e2e',    name: 'spawn-cluster',     command: './scripts/e2e/spawn-cluster.sh 5',      dur: 24000 },
      { stage: 'e2e',    name: 'chaos-kill-worker', command: './scripts/e2e/kill-random-worker.sh',   dur: 31000 },
      { stage: 'report', name: 'e2e-report',        command: 'python3 scripts/e2e/report.py',         dur: 8000 },
    ],
    commits: [
      { sha: '6ad0327', message: 'nightly end-to-end on lab cluster', author: 'schedule', files: [] },
    ],
  },
  'yaml-parser-ci': {
    repo: 'yaml-parser',
    file: '.orchestrator/ci.yml',
    plan: [
      { stage: 'lint', name: 'ruff',   command: 'ruff check yaml-parser/',        dur: 6000 },
      { stage: 'lint', name: 'mypy',   command: 'mypy yaml-parser/ --strict',     dur: 16000 },
      { stage: 'test', name: 'pytest', command: 'pytest yaml-parser/ -q --cov',   dur: 19000 },
    ],
    commits: [
      { sha: 'c497d0e', message: 'feat: pipeline.yml supports needs: between jobs', author: 'prabhat', files: ['yaml-parser/parser.py', 'examples/pipeline.yml'] },
      { sha: '4f0a9d1', message: 'fix: stage order stable for equal deps', author: 'prabhat', files: ['yaml-parser/planner.py'] },
      { sha: 'b7e21c4', message: 'feat: friendlier cycle-detection errors', author: 'prabhat', files: ['yaml-parser/validator.py', 'yaml-parser/tests/bad_cycle.yml'] },
    ],
  },
  'executor-ci': {
    repo: 'command-executor',
    file: '.orchestrator/ci.yml',
    plan: [
      { stage: 'build',  name: 'docker-build', command: 'docker build -t executor:ci nimesh/', dur: 33000 },
      { stage: 'test',   name: 'api-tests',    command: 'pytest nimesh/tests -q',              dur: 17000 },
      { stage: 'deploy', name: 'compose-up',   command: 'docker compose up -d executor',       dur: 9000 },
    ],
    commits: [
      { sha: '3be92cd', message: 'feat: executor stores logs in sqlite', author: 'nimesh', files: ['nimesh/app/db.py'] },
      { sha: '218fe9b', message: 'fix: cancel kills whole process group', author: 'nimesh', files: ['nimesh/app/runner.py'] },
      { sha: '12aa9f7', message: 'fix: raise job timeout in pipeline, kill zombies', author: 'nimesh', files: ['nimesh/app/runner.py', '.orchestrator/ci.yml'] },
    ],
  },
};

// pipe: which pipeline, ageMin: minutes before the session epoch, fail: job name.
// The last three land inside the monitor's 15-minute window.
const HISTORY: HistorySpec[] = [
  { pipe: 'orchestrator-ci',      ageMin: 2160 },
  { pipe: 'executor-ci',          ageMin: 1900 },
  { pipe: 'orchestrator-ci',      ageMin: 1700, fail: 'unit-tests' },
  { pipe: 'orchestrator-nightly', ageMin: 1560, trigger: 'schedule' },
  { pipe: 'yaml-parser-ci',       ageMin: 1450 },
  { pipe: 'orchestrator-ci',      ageMin: 1250, fail: 'compile' },
  { pipe: 'executor-ci',          ageMin: 1020 },
  { pipe: 'yaml-parser-ci',       ageMin: 840 },
  { pipe: 'orchestrator-nightly', ageMin: 360, trigger: 'schedule', fail: 'chaos-kill-worker' },
  { pipe: 'orchestrator-ci',      ageMin: 13.4 },
  { pipe: 'executor-ci',          ageMin: 9.2, fail: 'api-tests' },
  { pipe: 'yaml-parser-ci',       ageMin: 5.1 },
];

const LOG_LINES: Record<string, string[]> = {
  'compile': [
    '   Compiling libc v0.2.155',
    '   Compiling serde v1.0.203',
    '   Compiling tokio v1.38.0',
    '   Compiling axum-core v0.4.3',
    '   Compiling chrono v0.4.38',
    '   Compiling serde_json v1.0.117',
    '   Compiling axum v0.7.5',
    '   Compiling orchestrator v0.1.0 (/work/CI-CD-orchestrator)',
    'warning: unused variable: `timeout`',
    '  --> src/worker.rs:41:9',
    '    Finished `release` profile [optimized] target(s) in 31.44s',
  ],
  'clippy': [
    '    Checking orchestrator v0.1.0 (/work/CI-CD-orchestrator)',
    'warning: this expression creates a reference which is immediately dereferenced',
    '  --> src/state.rs:83:32',
    '   = note: `#[warn(clippy::needless_borrow)]` on by default',
    '    Finished `dev` profile [unoptimized + debuginfo] target(s) in 18.02s',
  ],
  'unit-tests': [
    'running 14 tests',
    'test state::tests::register_sets_online ... ok',
    'test state::tests::heartbeat_unknown_worker ... ok',
    'test state::tests::claim_assigns_pending_job ... ok',
    'test state::tests::claim_returns_none_when_empty ... ok',
    'test state::tests::reap_marks_stale_offline ... ok',
    'test state::tests::report_frees_worker ... ok',
    'test types::tests::job_status_serializes_lowercase ... ok',
    'test api::tests::trigger_creates_three_jobs ... ok',
    'test result: ok. 14 passed; 0 failed; 0 ignored; finished in 0.42s',
  ],
  'parser-tests': [
    '............                                             [100%]',
    '12 passed in 1.83s',
  ],
  'package': [
    'Sending build context to Docker daemon  14.22MB',
    'Step 1/8 : FROM rust:1.79-slim AS builder',
    ' ---> 3f9c1b2aa10d',
    'Step 2/8 : WORKDIR /work',
    ' ---> Using cache',
    'Step 5/8 : RUN cargo build --release',
    ' ---> Running in 8a2f19c07b31',
    'Step 8/8 : ENTRYPOINT ["/usr/local/bin/orchestrator"]',
    'Successfully built 91d47f22ab03',
    'Successfully tagged orchestrator:latest',
  ],
  'deploy': [
    '+ ssh deploy@vm-mumbai systemctl stop orchestrator',
    '+ scp target/release/orchestrator deploy@vm-mumbai:/opt/orchestrator/',
    'orchestrator                    100%   12MB   4.1MB/s   00:03',
    '+ ssh deploy@vm-mumbai systemctl start orchestrator',
    '+ curl -sf http://127.0.0.1:8080/api/health',
    '{"health":"Ok","online_workers":5}',
    'deploy: ok',
  ],
  'ruff': [
    'ruff 0.5.0',
    'checked 14 files in 0.09s',
    'All checks passed!',
  ],
  'mypy': [
    'Success: no issues found in 9 source files',
  ],
  'pytest': [
    '........................                                 [100%]',
    '---------- coverage: platform linux, python 3.12 ----------',
    'yaml-parser/parser.py        96%',
    'yaml-parser/validator.py     93%',
    'yaml-parser/planner.py       91%',
    '24 passed in 2.31s',
  ],
  'docker-build': [
    'Sending build context to Docker daemon  2.87MB',
    'Step 1/6 : FROM python:3.12-slim',
    ' ---> 51a2e8b9c410',
    'Step 3/6 : RUN pip install -r requirements.txt',
    ' ---> Running in f04a71b2c9de',
    'Successfully installed fastapi-0.111.0 uvicorn-0.30.1 httpx-0.27.0',
    'Step 6/6 : CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0"]',
    'Successfully tagged executor:ci',
  ],
  'api-tests': [
    'collected 18 items',
    'tests/test_jobs.py ..........                            [ 55%]',
    'tests/test_cancel.py ....                                [ 77%]',
    'tests/test_logs.py ....                                  [100%]',
    '18 passed in 3.02s',
  ],
  'compose-up': [
    '+ docker compose up -d executor',
    ' Container executor  Recreate',
    ' Container executor  Started',
    '+ curl -sf http://127.0.0.1:8000/health',
    '{"healthy":true}',
    'compose-up: ok',
  ],
  'spawn-cluster': [
    '+ ./scripts/e2e/spawn-cluster.sh 5',
    'starting coordinator on :8080 ... ok',
    'registering worker e2e-w1 ... ok',
    'registering worker e2e-w2 ... ok',
    'registering worker e2e-w3 ... ok',
    'registering worker e2e-w4 ... ok',
    'registering worker e2e-w5 ... ok',
    '5/5 workers online',
  ],
  'chaos-kill-worker': [
    '+ ./scripts/e2e/kill-random-worker.sh',
    'victim: e2e-w3 (pid 41022), running job #7',
    'kill -9 41022',
    'waiting for reaper (timeout 10s) ...',
    'reaper marked e2e-w3 offline after 5.2s',
    'job #7 requeued and claimed by e2e-w1',
    'chaos test: ok',
  ],
  'e2e-report': [
    'runs: 3 · jobs: 9 · requeues: 1',
    'median claim latency: 412ms',
    'report written to e2e/report-nightly.json',
  ],
};

const FAIL_TAIL: Record<string, string[]> = {
  'unit-tests': [
    'test state::tests::reap_marks_stale_offline ... FAILED',
    '',
    'failures:',
    '---- state::tests::reap_marks_stale_offline stdout ----',
    "assertion `left == right` failed",
    '  left: Online',
    ' right: Offline',
    'test result: FAILED. 13 passed; 1 failed; finished in 0.40s',
    'error: test failed, to rerun pass `--lib`',
  ],
  'compile': [
    'error[E0382]: borrow of moved value: `worker_name`',
    '  --> src/state.rs:91:34',
    '   |',
    '84 |     pub fn claim_job(&mut self, worker_name: String) -> Option<Job> {',
    '   |                                 ----------- move occurs because `worker_name` has type `String`',
    'error: could not compile `orchestrator` (lib) due to 1 previous error',
  ],
  'api-tests': [
    'tests/test_cancel.py ...F                                [ 77%]',
    '',
    '=================== FAILURES ===================',
    '________ test_cancel_kills_process_group ________',
    'assert proc.poll() is not None',
    'E   AssertionError: subprocess still alive after cancel',
    '=========== 17 passed, 1 failed in 3.44s ===========',
  ],
  'chaos-kill-worker': [
    'waiting for reaper (timeout 10s) ...',
    'ERROR: e2e-w3 still marked online after 10s',
    'job #7 was never requeued',
    'chaos test: FAILED',
  ],
};

// mulberry32: tiny seeded PRNG so history is stable across reloads
function rng(seed: number): () => number {
  return function () {
    seed |= 0; seed = (seed + 0x6D2B79F5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const state: { runs: Run[]; workers: Worker[]; nextRunId: number; nextJobId: number } =
  { runs: [], workers: [], nextRunId: 0, nextJobId: 0 };

// The simulation clock is shared across pages via sessionStorage, so a run's
// progress carries over when you navigate between list, detail and logs.
function loadPersisted(): Persisted | null {
  try { return JSON.parse(sessionStorage.getItem('dash.mock') ?? 'null'); } catch { return null; }
}
const persisted: Persisted = loadPersisted() ?? { epoch: Date.now(), extra: [] };
function persist() { sessionStorage.setItem('dash.mock', JSON.stringify(persisted)); }
persist();

function makeJobs(runId: number, rand: () => number, plan: PlanStep[]): Job[] {
  return plan.map((p) => ({
    id: state.nextJobId++,
    run_id: runId,
    stage: p.stage,
    name: p.name,
    command: p.command,
    status: 'pending',
    worker: null,
    started_at: null,
    finished_at: null,
    exit_code: null,
    planned: Math.round(p.dur * (0.7 + rand() * 0.6)),
  }));
}

function stageOrder(jobs: Job[]): string[] {
  return [...new Set(jobs.map((j) => j.stage))];
}

function buildHistory() {
  const rand = rng(20260705);
  const now = persisted.epoch;
  const commitCount: Record<string, number> = {};
  for (const spec of HISTORY) {
    const pipeline = PIPELINES[spec.pipe]!;
    const id = ++state.nextRunId;
    const start = now - spec.ageMin * 60 * 1000 - rand() * 60 * 1000;
    const jobs = makeJobs(id, rand, pipeline.plan);
    const idx = commitCount[spec.pipe] = (commitCount[spec.pipe] ?? -1) + 1;
    let cursor = start;
    let failedStage = null;
    for (const stage of stageOrder(jobs)) {
      const stageJobs = jobs.filter((j) => j.stage === stage);
      if (failedStage) { continue; }
      let stageEnd = cursor;
      for (const j of stageJobs) {
        j.worker = WORKERS[Math.floor(rand() * WORKERS.length)];
        j.started_at = cursor + Math.round(rand() * 2000);
        j.finished_at = j.started_at + j.planned!;
        stageEnd = Math.max(stageEnd, j.finished_at);
        if (spec.fail && j.name === spec.fail) {
          j.status = 'failed';
          j.exit_code = j.name === 'compile' ? 101 : 1;
          failedStage = stage;
        } else {
          j.status = 'passed';
          j.exit_code = 0;
        }
      }
      cursor = stageEnd + 500;
    }
    const done = jobs.filter((j) => j.finished_at);
    state.runs.push({
      id,
      pipeline: spec.pipe,
      repo: pipeline.repo,
      pipeline_file: pipeline.file,
      trigger: spec.trigger ?? (id % 3 === 2 ? 'manual' : 'webhook'),
      commit: pipeline.commits[idx % pipeline.commits.length],
      status: spec.fail ? 'failed' : 'passed',
      created_at: start - 1500,
      started_at: start,
      finished_at: Math.max(...done.map((j) => j.finished_at!)),
      jobs,
    });
  }
  // one run in progress, started ~35s before the epoch
  const id = ++state.nextRunId;
  const start = now - 35000;
  const pipe = PIPELINES['orchestrator-ci'];
  state.runs.push({
    id,
    pipeline: 'orchestrator-ci',
    repo: pipe.repo,
    pipeline_file: pipe.file,
    trigger: 'webhook',
    commit: pipe.commits[5],
    status: 'running',
    created_at: start - 1200,
    started_at: start,
    finished_at: null,
    jobs: makeJobs(id, rng(id * 31), pipe.plan),
  });

  state.workers = WORKERS.map((name, i) => ({
    name,
    status: i === 4 ? 'offline' : 'online',
    last_heartbeat: now - (i === 4 ? 220000 : 1000 + i * 700),
    job_id: null,
  }));

  // rehydrate runs triggered from the dashboard in this browser session
  for (const t of persisted.extra) {
    addTriggeredRun(t.start, t.commit);
  }
}

function addTriggeredRun(start: number, commit: Commit): number {
  const id = ++state.nextRunId;
  const rand = rng(id * 7919);
  state.runs.push({
    id,
    pipeline: 'orchestrator-ci',
    repo: 'CI-CD-orchestrator',
    pipeline_file: '.orchestrator/ci.yml',
    trigger: 'manual',
    commit,
    status: 'running',
    created_at: start,
    started_at: start,
    finished_at: null,
    jobs: makeJobs(id, rand, PIPELINES['orchestrator-ci'].plan),
  });
  return id;
}

// Advance active runs: within a stage jobs run in parallel on free workers;
// stages run strictly in order.
function tick() {
  const now = Date.now();
  for (const run of state.runs) {
    if (run.status !== 'running') continue;
    let cursor = run.started_at;
    let blocked = false;
    for (const stage of stageOrder(run.jobs)) {
      const stageJobs = run.jobs.filter((j) => j.stage === stage);
      if (blocked) break;
      const online = state.workers.filter((w) => w.status === 'online');
      stageJobs.forEach((j, k) => {
        if (j.started_at === null) {
          j.started_at = cursor + 400 + k * 900;
          j.worker = online[(j.id + k) % online.length].name;
        }
        if (now >= j.started_at! + j.planned!) {
          j.status = 'passed';
          j.exit_code = 0;
          j.finished_at = j.started_at! + j.planned!;
        } else if (now >= j.started_at!) {
          j.status = 'running';
        }
      });
      if (stageJobs.every((j) => j.status === 'passed')) {
        cursor = Math.max(...stageJobs.map((j) => j.finished_at!)) + 500;
      } else {
        blocked = true;
      }
    }
    if (run.jobs.every((j) => j.status === 'passed')) {
      run.status = 'passed';
      run.finished_at = Math.max(...run.jobs.map((j) => j.finished_at!));
    }
  }
  // wire worker.job_id to whatever is currently running
  for (const w of state.workers) w.job_id = null;
  for (const run of state.runs) {
    for (const j of run.jobs) {
      if (j.status === 'running') {
        const w = state.workers.find((x) => x.name === j.worker);
        if (w) w.job_id = j.id;
      }
    }
  }
  for (const w of state.workers) {
    if (w.status === 'online') w.last_heartbeat = now - 800 - (w.name.length * 130) % 2200;
  }
}

function logFor(job: Job): LogLine[] {
  const all = LOG_LINES[job.name] || ['(no output)'];
  if (job.status === 'pending') return [];
  let lines: string[];
  if (job.status === 'failed') {
    const head = all.slice(0, Math.max(1, all.length - 2));
    lines = head.concat(FAIL_TAIL[job.name] || ['error: job failed']);
  } else if (job.status === 'running') {
    const progress = Math.min(1, (Date.now() - job.started_at!) / (job.planned ?? 1));
    lines = all.slice(0, Math.max(1, Math.floor(all.length * progress)));
  } else {
    lines = all;
  }
  return lines.map((t) => ({
    t,
    err: /error|FAILED|AssertionError|warning|assertion/i.test(t) && !/0 failed/.test(t),
    ok: /test result: ok|Successfully|deploy: ok|compose-up: ok|passed in|Finished|All checks passed|no issues found/.test(t),
  }));
}

buildHistory();

function dayKey(ts: number): string {
  const d = new Date(ts);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

// One entry per day for the past year. Days before the project started are
// zero; the active months get a seeded weekday-heavy pattern; days that have
// actual runs in state use the real count.
const PROJECT_AGE_DAYS = 150;

function buildCalendar(): CalendarDay[] {
  const real: Record<string, number> = {};
  for (const r of state.runs) {
    const k = dayKey(r.started_at);
    real[k] = (real[k] || 0) + 1;
  }
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const days: CalendarDay[] = [];
  for (let i = 363; i >= 0; i--) {
    const d = new Date(today.getTime() - i * 86400000);
    const k = dayKey(d.getTime());
    let count = 0;
    if (i < PROJECT_AGE_DAYS) {
      let h = 0;
      for (const c of k) h = (h * 31 + c.charCodeAt(0)) | 0;
      const roll = rng(h)();
      const dow = d.getDay();
      const weekend = dow === 0 || dow === 6;
      if (roll < (weekend ? 0.25 : 0.8)) {
        count = 1 + Math.floor(rng(h + 1)() * (weekend ? 2 : 5));
      }
    }
    if (real[k] != null) count = real[k];
    days.push({ date: k, count });
  }
  return days;
}

// Repos registered through the + Add repo form while in mock mode.
const addedRepos: Repo[] = [];
// Repos removed via delete while in mock mode.
const deletedRepos = new Set<string>();

export const mockApi: Api = {
  async calendar(): Promise<CalendarDay[]> {
    tick();
    return buildCalendar();
  },
  async repos(): Promise<Repo[]> {
    const builtin = Object.entries(REPOS).map(([name, r]) => ({
      name,
      ...r,
      pipelines: Object.entries(PIPELINES)
        .filter(([, p]) => p.repo === name)
        .map(([pname, p]) => ({ name: pname, file: p.file })),
    }));
    return [...builtin, ...addedRepos].filter((r) => !deletedRepos.has(r.name));
  },
  async deleteRepo(name: string): Promise<void> {
    deletedRepos.add(name);
  },
  async pipelineFile(repo: string, file?: string): Promise<{ file: string; content: string }> {
    return {
      file: file ?? '.orchestrator/actions.yml',
      content: [
        `# simulated — switch to ?mode=live for the real file`,
        `name: ${repo}-ci`,
        `stages: [build, test, deploy]`,
        `jobs:`,
        `  build:`,
        `    stage: build`,
        `    image: rust:latest`,
        `    script: cargo build --release`,
      ].join('\n'),
    };
  },
  async addRepo(remote: string): Promise<Repo> {
    const m = remote
      .trim()
      .replace(/\/+$/, '')
      .match(/^https?:\/\/[^/]+\/([^/]+)\/([^/]+?)(?:\.git)?$/);
    if (!m) throw new Error(`'${remote}' is not a valid repo URL (expected https://host/owner/repo)`);
    const repo: Repo = {
      name: m[2],
      description: 'added in mock mode — switch to ?mode=live for real Forgejo metadata',
      language: '—',
      branch: 'main',
      owner: m[1],
      remote: remote.trim(),
      languages: [],
      contributors: [{ login: m[1], name: m[1] }],
      pipelines: [],
    };
    addedRepos.push(repo);
    return repo;
  },
  async overview(): Promise<Overview> {
    tick();
    return {
      workers: structuredClone(state.workers),
      runs: structuredClone(state.runs).sort((a, b) => b.started_at - a.started_at),
    };
  },
  async run(id: number | string): Promise<Run | null> {
    tick();
    const run = state.runs.find((r) => r.id === Number(id));
    return run ? structuredClone(run) : null;
  },
  async job(runId: number | string, jobId: number | string): Promise<JobDetail | null> {
    tick();
    const run = state.runs.find((r) => r.id === Number(runId));
    if (!run) return null;
    const job = run.jobs.find((j) => j.id === Number(jobId));
    if (!job) return null;
    return { run: structuredClone(run), job: structuredClone(job), log: logFor(job) };
  },
  async trigger() {
    tick();
    const start = Date.now();
    const commit = {
      sha: (start % 0xfffffff).toString(16).padStart(7, '0'),
      message: 'manual trigger from dashboard',
      author: 'you',
      files: [],
    };
    const id = addTriggeredRun(start, commit);
    persisted.extra.push({ start, commit });
    persist();
    return { id };
  },
};
