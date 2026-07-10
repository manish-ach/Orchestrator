// Persistence for the coordinator.
//
//   Postgres — main database: runs, jobs, repos (source of truth)
//   Redis    — the cache between coordinator and workers: the worker
//              registry (heartbeats) and the ready-job queue that
//              claim_job pops from
//
// Both come from docker-compose.yml; override with DATABASE_URL / REDIS_URL.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Local;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use crate::pipeline::Plan;
use crate::types::{
    CalendarDay, ClaimedJob, Commit, Job, JobStatus, Repo, ReportRequest, Run, Status, TriggerKind, Worker,
};

pub type SharedStore = Arc<Store>;

const QUEUE_KEY: &str = "jobs:ready";
const WORKERS_KEY: &str = "workers";
const HEARTBEAT_TIMEOUT_MS: i64 = 5_000;
/// A worker must be silent this long before its jobs are stolen back —
/// much longer than the offline display threshold so a hiccup doesn't
/// double-execute a job that is still running.
const REQUEUE_AFTER_MS: i64 = 30_000;

fn worker_queue(worker_id: &str) -> String {
    format!("jobs:ready:{worker_id}")
}

const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    id            BIGSERIAL PRIMARY KEY,
    pipeline      TEXT NOT NULL,
    repo          TEXT NOT NULL,
    pipeline_file TEXT NOT NULL,
    trigger_kind  TEXT NOT NULL,
    commit_info   JSONB,
    status        TEXT NOT NULL DEFAULT 'pending',
    created_at    BIGINT NOT NULL,
    started_at    BIGINT NOT NULL,
    finished_at   BIGINT
);
CREATE TABLE IF NOT EXISTS jobs (
    id          BIGSERIAL PRIMARY KEY,
    run_id      BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    stage       TEXT NOT NULL,
    name        TEXT NOT NULL,
    command     TEXT NOT NULL,
    needs       JSONB NOT NULL DEFAULT '[]'::jsonb,
    env         JSONB NOT NULL DEFAULT '{}'::jsonb,
    queued      BOOLEAN NOT NULL DEFAULT FALSE,
    status      TEXT NOT NULL DEFAULT 'pending',
    worker      TEXT,
    started_at  BIGINT,
    finished_at BIGINT,
    exit_code   INT,
    output      TEXT
);
CREATE INDEX IF NOT EXISTS jobs_run_id_idx ON jobs(run_id);
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS worker_id TEXT;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS queued_for TEXT;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS artifacts JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS has_artifacts BOOLEAN NOT NULL DEFAULT FALSE;
CREATE TABLE IF NOT EXISTS repos (
    remote TEXT PRIMARY KEY,
    data   JSONB NOT NULL
);
"#;

fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WorkerRecord {
    name: String,
    last_heartbeat: i64,
    job_id: Option<i64>,
}

pub struct Store {
    db: PgPool,
    redis: ConnectionManager,
}

impl Store {
    pub async fn connect() -> Result<Store, String> {
        let db_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://orchestrator:orchestrator@127.0.0.1:5432/orchestrator".into());
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());

        let db = PgPoolOptions::new()
            .max_connections(8)
            .connect(&db_url)
            .await
            .map_err(|e| format!("Postgres at {db_url}: {e}"))?;
        sqlx::raw_sql(MIGRATIONS)
            .execute(&db)
            .await
            .map_err(|e| format!("running migrations: {e}"))?;

        let client = redis::Client::open(redis_url.clone()).map_err(|e| format!("Redis URL {redis_url}: {e}"))?;
        let redis = ConnectionManager::new(client)
            .await
            .map_err(|e| format!("Redis at {redis_url}: {e}"))?;

        Ok(Store { db, redis })
    }

    // ---- workers (Redis, keyed by unique id) ---------------------------

    /// First contact mints a unique id; re-registration with a known id
    /// just refreshes the record (name changes included).
    pub async fn register(&self, name: &str, id: Option<String>) -> Result<String, String> {
        let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut r = self.redis.clone();
        let rec = serde_json::to_string(&WorkerRecord {
            name: name.to_string(),
            last_heartbeat: now_ms(),
            job_id: None,
        })
        .unwrap();
        let _: () = r.hset(WORKERS_KEY, &id, rec).await.map_err(|e| e.to_string())?;
        Ok(id)
    }

    async fn get_worker(&self, id: &str) -> Result<Option<WorkerRecord>, String> {
        let mut r = self.redis.clone();
        let raw: Option<String> = r.hget(WORKERS_KEY, id).await.map_err(|e| e.to_string())?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }

    async fn put_worker(&self, id: &str, rec: &WorkerRecord) -> Result<(), String> {
        let mut r = self.redis.clone();
        let _: () = r
            .hset(WORKERS_KEY, id, serde_json::to_string(rec).unwrap())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn heartbeat(&self, id: &str) -> Result<bool, String> {
        let Some(mut rec) = self.get_worker(id).await? else { return Ok(false) };
        rec.last_heartbeat = now_ms();
        self.put_worker(id, &rec).await?;
        Ok(true)
    }

    pub async fn list_workers(&self) -> Result<Vec<Worker>, String> {
        let mut r = self.redis.clone();
        let all: HashMap<String, String> = r.hgetall(WORKERS_KEY).await.map_err(|e| e.to_string())?;
        let now = now_ms();
        let mut workers: Vec<Worker> = all
            .into_iter()
            .filter_map(|(id, raw)| {
                let rec: WorkerRecord = serde_json::from_str(&raw).ok()?;
                Some(Worker {
                    id,
                    name: rec.name,
                    status: if now - rec.last_heartbeat <= HEARTBEAT_TIMEOUT_MS { Status::Online } else { Status::Offline },
                    last_heartbeat: rec.last_heartbeat,
                    job_id: rec.job_id,
                })
            })
            .collect();
        workers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(workers)
    }

    pub async fn online_workers(&self) -> Result<u16, String> {
        Ok(self
            .list_workers()
            .await?
            .iter()
            .filter(|w| matches!(w.status, Status::Online))
            .count() as u16)
    }

    /// Resolve a worker NAME (the stable label from `--name`) to the id of
    /// an online worker carrying it — how WORKER_PIN finds its queue.
    async fn find_worker_by_name(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self
            .list_workers()
            .await?
            .into_iter()
            .find(|w| w.name == name && matches!(w.status, Status::Online))
            .map(|w| w.id))
    }

    async fn worker_fresh(&self, id: &str, within_ms: i64) -> Result<bool, String> {
        Ok(self
            .get_worker(id)
            .await?
            .map(|rec| now_ms() - rec.last_heartbeat <= within_ms)
            .unwrap_or(false))
    }

    async fn set_worker_job(&self, id: &str, job_id: Option<i64>) -> Result<(), String> {
        if let Some(mut rec) = self.get_worker(id).await? {
            rec.job_id = job_id;
            self.put_worker(id, &rec).await?;
        }
        Ok(())
    }

    // ---- runs & jobs (Postgres + Redis queue) --------------------------

    pub async fn create_run(
        &self,
        pipeline: &str,
        repo: &str,
        pipeline_file: &str,
        trigger: TriggerKind,
        commit: Option<&Commit>,
        // injected into every job's env (REPO_URL, REPO_BRANCH, COMMIT_SHA)
        // so pipelines can `git clone $REPO_URL` instead of hardcoding it;
        // the job's own YAML env wins on conflicts
        inject_env: &HashMap<String, String>,
        plan: &Plan,
    ) -> Result<i64, String> {
        let now = now_ms();
        let run_id: i64 = sqlx::query_scalar(
            "INSERT INTO runs (pipeline, repo, pipeline_file, trigger_kind, commit_info, status, created_at, started_at)
             VALUES ($1, $2, $3, $4, $5, 'pending', $6, $6) RETURNING id",
        )
        .bind(pipeline)
        .bind(repo)
        .bind(pipeline_file)
        .bind(trigger.as_str())
        .bind(commit.map(|c| serde_json::to_value(c).unwrap()))
        .bind(now)
        .fetch_one(&self.db)
        .await
        .map_err(|e| e.to_string())?;

        for job in &plan.jobs {
            let mut env = inject_env.clone();
            env.extend(job.env.clone());
            sqlx::query(
                "INSERT INTO jobs (run_id, stage, name, command, needs, env, artifacts) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(run_id)
            .bind(&job.stage)
            .bind(&job.name)
            .bind(&job.command)
            .bind(serde_json::json!(job.needs))
            .bind(serde_json::json!(env))
            .bind(serde_json::json!(job.artifacts))
            .execute(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        }

        self.enqueue_ready(run_id).await?;
        Ok(run_id)
    }

    /// Push every pending, not-yet-queued job whose `needs` have all passed
    /// onto a ready queue. Placement: jobs with dependencies go to the
    /// worker that ran them (files are probably warm there); independent
    /// jobs go to the global queue for any free worker.
    async fn enqueue_ready(&self, run_id: i64) -> Result<(), String> {
        let rows = sqlx::query("SELECT id, needs, env FROM jobs WHERE run_id = $1 AND status = 'pending' AND queued = FALSE ORDER BY id")
            .bind(run_id)
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;

        // name -> worker_id of every passed job in the run
        let passed: HashMap<String, Option<String>> =
            sqlx::query("SELECT name, worker_id FROM jobs WHERE run_id = $1 AND status = 'passed'")
                .bind(run_id)
                .fetch_all(&self.db)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|row| (row.get::<String, _>("name"), row.get::<Option<String>, _>("worker_id")))
                .collect();

        let mut redis = self.redis.clone();
        for row in rows {
            let id: i64 = row.get("id");
            let needs: Vec<String> =
                serde_json::from_value(row.get::<serde_json::Value, _>("needs")).unwrap_or_default();
            if !needs.iter().all(|n| passed.contains_key(n)) {
                continue;
            }

            // a hard pin from the pipeline (env WORKER_PIN: <worker name>)
            // beats the warm-files heuristic — jobs like self-deploy only
            // make sense on one specific machine
            let mut target: Option<String> = None;
            let env: HashMap<String, String> =
                serde_json::from_value(row.get::<serde_json::Value, _>("env")).unwrap_or_default();
            if let Some(pin) = env.get("WORKER_PIN") {
                target = self.find_worker_by_name(pin).await?;
                if target.is_none() {
                    println!("Job {id}: WORKER_PIN '{pin}' matches no online worker — any worker may claim it");
                }
            }

            // otherwise prefer the worker that produced the most dependencies
            if target.is_none() {
                let mut votes: HashMap<&str, usize> = HashMap::new();
                for n in &needs {
                    if let Some(Some(wid)) = passed.get(n) {
                        *votes.entry(wid.as_str()).or_default() += 1;
                    }
                }
                if let Some((wid, _)) = votes.into_iter().max_by_key(|(_, c)| *c) {
                    if self.worker_fresh(wid, HEARTBEAT_TIMEOUT_MS).await? {
                        target = Some(wid.to_string());
                    }
                }
            }

            // conditional flip: two report_job calls can race into
            // enqueue_ready for the same run — only the one that wins this
            // row update may push, or the job would execute twice
            let res = sqlx::query(
                "UPDATE jobs SET queued = TRUE, queued_for = $2 WHERE id = $1 AND queued = FALSE AND status = 'pending'",
            )
            .bind(id)
            .bind(&target)
            .execute(&self.db)
            .await
            .map_err(|e| e.to_string())?;
            if res.rows_affected() == 1 {
                let key = target.as_deref().map(worker_queue).unwrap_or_else(|| QUEUE_KEY.to_string());
                let _: () = redis.rpush(key, id).await.map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Claim order: this worker's personal queue (affinity), then the
    /// global queue. Returns the job plus which dependency jobs have
    /// artifacts to download.
    pub async fn claim_job(&self, worker_id: &str, worker_name: &str) -> Result<Option<ClaimedJob>, String> {
        let mut redis = self.redis.clone();
        let mut popped: Option<i64> = redis
            .lpop(worker_queue(worker_id), None)
            .await
            .map_err(|e| e.to_string())?;
        if popped.is_none() {
            popped = redis.lpop(QUEUE_KEY, None).await.map_err(|e| e.to_string())?;
        }
        let Some(job_id) = popped else { return Ok(None) };

        // status guard: a stale queue entry (e.g. requeued elsewhere) must
        // not restart a job someone else already owns or finished
        let row = sqlx::query(
            "UPDATE jobs SET status = 'running', worker = $2, worker_id = $3, started_at = $4
             WHERE id = $1 AND status = 'pending' RETURNING *",
        )
        .bind(job_id)
        .bind(worker_name)
        .bind(worker_id)
        .bind(now_ms())
        .fetch_optional(&self.db)
        .await
        .map_err(|e| e.to_string())?;

        let Some(row) = row else { return Ok(None) };
        let job = job_from_row(&row);

        let input_jobs: Vec<i64> = if job.needs.is_empty() {
            Vec::new()
        } else {
            sqlx::query("SELECT id FROM jobs WHERE run_id = $1 AND has_artifacts AND name = ANY($2)")
                .bind(job.run_id)
                .bind(&job.needs)
                .fetch_all(&self.db)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|r| r.get::<i64, _>("id"))
                .collect()
        };

        self.set_worker_job(worker_id, Some(job.id)).await?;
        self.roll_up(job.run_id).await?;
        Ok(Some(ClaimedJob { job, input_jobs }))
    }

    pub async fn mark_artifacts(&self, job_id: i64) -> Result<(), String> {
        sqlx::query("UPDATE jobs SET has_artifacts = TRUE WHERE id = $1")
            .bind(job_id)
            .execute(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Steal work back from workers that have been silent too long: drain
    /// their personal queues and reset their running jobs, then requeue.
    pub async fn reconcile(&self) -> Result<(), String> {
        let workers = self.list_workers().await?;
        let now = now_ms();
        let dead: Vec<String> = workers
            .iter()
            .filter(|w| now - w.last_heartbeat > REQUEUE_AFTER_MS)
            .map(|w| w.id.clone())
            .collect();
        let known: Vec<String> = workers.iter().map(|w| w.id.clone()).collect();

        let mut redis = self.redis.clone();
        let mut touched = false;

        for id in &dead {
            loop {
                let popped: Option<i64> = redis.lpop(worker_queue(id), None).await.map_err(|e| e.to_string())?;
                let Some(job_id) = popped else { break };
                sqlx::query("UPDATE jobs SET queued = FALSE, queued_for = NULL WHERE id = $1 AND status = 'pending'")
                    .bind(job_id)
                    .execute(&self.db)
                    .await
                    .map_err(|e| e.to_string())?;
                touched = true;
            }
        }

        let running = sqlx::query("SELECT id, worker_id FROM jobs WHERE status = 'running'")
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        for row in running {
            let wid: Option<String> = row.get("worker_id");
            let orphaned = match wid {
                Some(ref wid) => dead.contains(wid) || !known.contains(wid),
                None => true,
            };
            if orphaned {
                sqlx::query(
                    "UPDATE jobs SET status = 'pending', queued = FALSE, queued_for = NULL,
                     worker = NULL, worker_id = NULL, started_at = NULL WHERE id = $1",
                )
                .bind(row.get::<i64, _>("id"))
                .execute(&self.db)
                .await
                .map_err(|e| e.to_string())?;
                touched = true;
            }
        }

        if touched {
            let run_ids: Vec<i64> =
                sqlx::query("SELECT DISTINCT run_id FROM jobs WHERE status = 'pending' AND queued = FALSE")
                    .fetch_all(&self.db)
                    .await
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|r| r.get::<i64, _>("run_id"))
                    .collect();
            for run_id in run_ids {
                self.enqueue_ready(run_id).await?;
                self.roll_up(run_id).await?;
            }
        }
        Ok(())
    }

    pub async fn report_job(&self, job_id: i64, req: &ReportRequest) -> Result<(), String> {
        let row = sqlx::query(
            "UPDATE jobs SET status = $2, output = $3, exit_code = $4, finished_at = $5 WHERE id = $1
             RETURNING run_id, worker_id",
        )
        .bind(job_id)
        .bind(req.status.as_str())
        .bind(&req.output)
        .bind(req.exit_code)
        .bind(now_ms())
        .fetch_optional(&self.db)
        .await
        .map_err(|e| e.to_string())?;

        let Some(row) = row else { return Ok(()) };
        let run_id: i64 = row.get("run_id");
        if let Some(worker_id) = row.get::<Option<String>, _>("worker_id") {
            self.set_worker_job(&worker_id, None).await?;
        }

        match req.status {
            JobStatus::Passed => self.enqueue_ready(run_id).await?,
            JobStatus::Failed => self.skip_dependents(run_id).await?,
            _ => {}
        }
        self.roll_up(run_id).await?;
        Ok(())
    }

    /// When a job fails, everything that (transitively) needs it can never
    /// run — mark those jobs failed with an explanatory output.
    async fn skip_dependents(&self, run_id: i64) -> Result<(), String> {
        loop {
            let failed: Vec<String> = sqlx::query("SELECT name FROM jobs WHERE run_id = $1 AND status = 'failed'")
                .bind(run_id)
                .fetch_all(&self.db)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|r| r.get::<String, _>("name"))
                .collect();

            let pending = sqlx::query("SELECT id, needs FROM jobs WHERE run_id = $1 AND status = 'pending'")
                .bind(run_id)
                .fetch_all(&self.db)
                .await
                .map_err(|e| e.to_string())?;

            let mut changed = false;
            for row in pending {
                let needs: Vec<String> =
                    serde_json::from_value(row.get::<serde_json::Value, _>("needs")).unwrap_or_default();
                if let Some(dep) = needs.iter().find(|n| failed.contains(n)) {
                    sqlx::query(
                        "UPDATE jobs SET status = 'failed', output = $2, finished_at = $3 WHERE id = $1",
                    )
                    .bind(row.get::<i64, _>("id"))
                    .bind(format!("skipped: dependency '{dep}' failed"))
                    .bind(now_ms())
                    .execute(&self.db)
                    .await
                    .map_err(|e| e.to_string())?;
                    changed = true;
                }
            }
            if !changed {
                return Ok(());
            }
        }
    }

    /// Recompute a run's status and finished_at from its jobs.
    async fn roll_up(&self, run_id: i64) -> Result<(), String> {
        let rows = sqlx::query("SELECT status, finished_at FROM jobs WHERE run_id = $1")
            .bind(run_id)
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;

        let statuses: Vec<String> = rows.iter().map(|r| r.get::<String, _>("status")).collect();
        let any = |v: &str| statuses.iter().any(|s| s == v);
        // still in flight while anything runs or is waiting behind finished work
        let status = if any("running") || (any("pending") && (any("passed") || any("failed"))) {
            "running"
        } else if any("failed") {
            "failed"
        } else if !statuses.is_empty() && statuses.iter().all(|s| s == "passed") {
            "passed"
        } else {
            "pending"
        };

        let finished_at: Option<i64> = if status == "passed" || status == "failed" {
            rows.iter().filter_map(|r| r.get::<Option<i64>, _>("finished_at")).max()
        } else {
            None
        };

        sqlx::query("UPDATE runs SET status = $2, finished_at = $3 WHERE id = $1")
            .bind(run_id)
            .bind(status)
            .bind(finished_at)
            .execute(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_runs(&self) -> Result<Vec<Run>, String> {
        let run_rows = sqlx::query("SELECT * FROM runs ORDER BY id DESC LIMIT 200")
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;

        let mut runs: Vec<Run> = run_rows.iter().map(run_from_row).collect();
        if runs.is_empty() {
            return Ok(runs);
        }

        let ids: Vec<i64> = runs.iter().map(|r| r.id).collect();
        let job_rows = sqlx::query("SELECT * FROM jobs WHERE run_id = ANY($1) ORDER BY id")
            .bind(&ids)
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;

        for row in &job_rows {
            let job = job_from_row(row);
            if let Some(run) = runs.iter_mut().find(|r| r.id == job.run_id) {
                run.jobs.push(job);
            }
        }
        Ok(runs)
    }

    pub async fn get_run(&self, id: i64) -> Result<Option<Run>, String> {
        let row = sqlx::query("SELECT * FROM runs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        let Some(row) = row else { return Ok(None) };

        let mut run = run_from_row(&row);
        let job_rows = sqlx::query("SELECT * FROM jobs WHERE run_id = $1 ORDER BY id")
            .bind(id)
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        run.jobs = job_rows.iter().map(job_from_row).collect();
        Ok(Some(run))
    }

    pub async fn list_jobs(&self) -> Result<Vec<Job>, String> {
        let rows = sqlx::query("SELECT * FROM jobs ORDER BY id")
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.iter().map(job_from_row).collect())
    }

    /// Live tail pushed by the executor while a job runs — lets the
    /// dashboard stream logs before the final report lands. Only running
    /// jobs accept progress so a late POST can't clobber the real output.
    pub async fn job_progress(&self, job_id: i64, output: &str) -> Result<(), String> {
        sqlx::query("UPDATE jobs SET output = $2 WHERE id = $1 AND status = 'running'")
            .bind(job_id)
            .bind(output)
            .execute(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn job_output(&self, job_id: i64) -> Result<Option<String>, String> {
        let row = sqlx::query("SELECT output FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(|r| r.get::<Option<String>, _>("output").unwrap_or_default()))
    }

    /// On boot: rebuild the Redis queues from Postgres so a flushed/stale
    /// Redis can't strand ready jobs.
    pub async fn reconcile_queue(&self) -> Result<(), String> {
        let mut redis = self.redis.clone();
        let queues: Vec<String> = redis.keys("jobs:ready*").await.map_err(|e| e.to_string())?;
        for key in queues {
            let _: () = redis.del(&key).await.map_err(|e| e.to_string())?;
        }
        sqlx::query("UPDATE jobs SET queued = FALSE WHERE status = 'pending'")
            .execute(&self.db)
            .await
            .map_err(|e| e.to_string())?;

        let run_ids: Vec<i64> = sqlx::query("SELECT DISTINCT run_id FROM jobs WHERE status = 'pending'")
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| r.get::<i64, _>("run_id"))
            .collect();

        for run_id in run_ids {
            self.enqueue_ready(run_id).await?;
        }
        Ok(())
    }

    // ---- repos (Postgres) ----------------------------------------------

    pub async fn upsert_repo(&self, repo: &Repo) -> Result<(), String> {
        let Some(remote) = &repo.remote else { return Ok(()) };
        sqlx::query(
            "INSERT INTO repos (remote, data) VALUES ($1, $2)
             ON CONFLICT (remote) DO UPDATE SET data = EXCLUDED.data",
        )
        .bind(remote)
        .bind(serde_json::to_value(repo).unwrap())
        .execute(&self.db)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_repos(&self) -> Result<Vec<Repo>, String> {
        let rows = sqlx::query("SELECT data FROM repos ORDER BY remote")
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .filter_map(|r| serde_json::from_value(r.get::<serde_json::Value, _>("data")).ok())
            .collect())
    }

    pub async fn repo_remotes(&self) -> Result<Vec<String>, String> {
        Ok(self.list_repos().await?.into_iter().filter_map(|r| r.remote).collect())
    }

    pub async fn get_repo(&self, name: &str) -> Result<Option<Repo>, String> {
        Ok(self.list_repos().await?.into_iter().find(|r| r.name == name))
    }

    /// Unregister a repo. Its runs stay in history.
    pub async fn delete_repo(&self, name: &str) -> Result<bool, String> {
        let res = sqlx::query("DELETE FROM repos WHERE data->>'name' = $1")
            .bind(name)
            .execute(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(res.rows_affected() > 0)
    }

    // ---- calendar -------------------------------------------------------

    pub async fn calendar(&self) -> Result<Vec<CalendarDay>, String> {
        use chrono::{Duration, TimeZone};
        let starts: Vec<i64> = sqlx::query("SELECT started_at FROM runs")
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| r.get::<i64, _>("started_at"))
            .collect();

        let mut counts: HashMap<String, u32> = HashMap::new();
        for ms in starts {
            if let Some(d) = Local.timestamp_millis_opt(ms).single() {
                *counts.entry(d.format("%Y-%m-%d").to_string()).or_default() += 1;
            }
        }
        let today = Local::now().date_naive();
        Ok((0..364)
            .rev()
            .map(|i| {
                let date = (today - Duration::days(i)).format("%Y-%m-%d").to_string();
                let count = counts.get(&date).copied().unwrap_or(0);
                CalendarDay { date, count }
            })
            .collect())
    }
}

fn job_from_row(row: &sqlx::postgres::PgRow) -> Job {
    Job {
        id: row.get("id"),
        run_id: row.get("run_id"),
        stage: row.get("stage"),
        name: row.get("name"),
        command: row.get("command"),
        needs: serde_json::from_value(row.get::<serde_json::Value, _>("needs")).unwrap_or_default(),
        env: serde_json::from_value(row.get::<serde_json::Value, _>("env")).unwrap_or_default(),
        artifacts: serde_json::from_value(row.get::<serde_json::Value, _>("artifacts")).unwrap_or_default(),
        has_artifacts: row.get("has_artifacts"),
        status: JobStatus::from_str(&row.get::<String, _>("status")),
        worker: row.get("worker"),
        worker_id: row.get("worker_id"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        exit_code: row.get("exit_code"),
        output: row.get("output"),
    }
}

fn run_from_row(row: &sqlx::postgres::PgRow) -> Run {
    let commit: Option<Commit> = row
        .get::<Option<serde_json::Value>, _>("commit_info")
        .and_then(|v| serde_json::from_value(v).ok());
    Run {
        id: row.get("id"),
        pipeline: row.get("pipeline"),
        repo: row.get("repo"),
        pipeline_file: row.get("pipeline_file"),
        trigger: TriggerKind::from_str(&row.get::<String, _>("trigger_kind")),
        commit,
        status: JobStatus::from_str(&row.get::<String, _>("status")),
        created_at: row.get("created_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        jobs: Vec::new(),
    }
}
