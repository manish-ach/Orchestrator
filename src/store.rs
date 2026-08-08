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

use crate::insights;
use crate::pipeline::Plan;
use crate::types::{
    CalendarDay, ClaimedJob, Commit, DeviceProfile, Job, JobStatus, Repo, ReportRequest, Run, StatSample, Status,
    TriggerKind, WebhookDelivery, Worker, WorkerActivity, WorkerJob, WorkerStats, WorkerStatsSeries,
};

pub type SharedStore = Arc<Store>;

const QUEUE_KEY: &str = "jobs:ready";
const WORKERS_KEY: &str = "workers";
const HEARTBEAT_TIMEOUT_MS: i64 = 5_000;
/// A worker must be silent this long before its jobs are stolen back —
/// much longer than the offline display threshold so a hiccup doesn't
/// double-execute a job that is still running.
const REQUEUE_AFTER_MS: i64 = 30_000;
/// Workers silent this long are dropped from the registry entirely, so a
/// decommissioned machine doesn't clutter the monitor forever.
const PRUNE_AFTER_MS: i64 = 24 * 60 * 60 * 1000;
/// Dashboard sessions live this long in Redis (seconds).
const SESSION_TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// How many heartbeat stat samples to keep per worker. At the 2s heartbeat
/// interval this is ~15 minutes of CPU/RAM history for the dashboard.
const STATS_HISTORY_LEN: isize = 450;

fn worker_queue(worker_id: &str) -> String {
    format!("jobs:ready:{worker_id}")
}

fn stats_key(worker_id: &str) -> String {
    format!("workers:stats:{worker_id}")
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
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS tags JSONB NOT NULL DEFAULT '[]'::jsonb;
-- When the job first became runnable (all `needs` passed and a placement was
-- found). `started_at - ready_at` is the queue wait, which is otherwise
-- unrecoverable: nothing else records when a job *could* have started, and a
-- requeue clears started_at. Set once and never overwritten, so a job that
-- waited, got orphaned and waited again reports the total.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS ready_at BIGINT;
-- How many times a worker died mid-job and the reconciler handed it back.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS requeue_count INT NOT NULL DEFAULT 0;
-- The one line worth showing next to a failure. Extracted once on report so the
-- run list can say *why* something broke without shipping whole build logs:
-- list queries deliberately omit `output`, which is megabytes per response once
-- real cargo builds are involved.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS first_error TEXT;
-- Backfill: the cause was always in `output`, it just was never distilled.
-- Rough (first matching line, SQL-side) but it means old failures are not blank.
UPDATE jobs SET first_error = LEFT((
    SELECT l FROM regexp_split_to_table(output, E'\n') AS l
    WHERE l ~* '(error|panicked|fatal|exception|traceback|failed)'
    LIMIT 1), 200)
WHERE status = 'failed' AND first_error IS NULL AND output IS NOT NULL;
-- Which branch the run built. Known at ingest (the webhook's `refs/heads/…`)
-- and already injected into every job as REPO_BRANCH, but it was never kept on
-- the run itself, so nothing could group or filter by it after the fact. Not
-- backfillable: the push payload is long gone.
ALTER TABLE runs ADD COLUMN IF NOT EXISTS branch TEXT;
CREATE TABLE IF NOT EXISTS repos (
    remote TEXT PRIMARY KEY,
    data   JSONB NOT NULL
);
-- Proof that a repo's webhook reaches us. Its own table rather than a field on
-- `repos.data`, because that blob is overwritten wholesale by the Forgejo
-- refresh every two minutes and would take this with it.
-- One row per scheduled pipeline, holding the minute it last fired. The
-- scheduler claims a minute with a conditional UPDATE, so a coordinator
-- restart mid-minute cannot double-fire and two coordinators cannot both win.
CREATE TABLE IF NOT EXISTS schedule_state (
    key        TEXT PRIMARY KEY,
    last_fired BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS webhook_deliveries (
    repo    TEXT PRIMARY KEY,
    last_at BIGINT NOT NULL,
    event   TEXT,
    status  TEXT NOT NULL,
    detail  TEXT
);
"#;

fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WorkerRecord {
    name: String,
    last_heartbeat: i64,
    // defaults keep records written by older coordinators readable
    #[serde(default)]
    registered_at: i64,
    #[serde(default)]
    tags: Vec<String>,
    job_id: Option<i64>,
    /// latest machine stats from the heartbeat
    #[serde(default)]
    stats: Option<WorkerStats>,
    /// what machine this is, from the last register
    #[serde(default)]
    device: Option<DeviceProfile>,
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
    /// just refreshes the record (name changes included). Offline records
    /// carrying the same name are dropped — a container that lost its id
    /// file re-registers as a "new" worker, and without this the registry
    /// fills with dead duplicates that break the dashboard.
    pub async fn register(
        &self,
        name: &str,
        id: Option<String>,
        tags: &[String],
        device: Option<DeviceProfile>,
    ) -> Result<String, String> {
        let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let prior = self.get_worker(&id).await?;
        let registered_at = prior
            .as_ref()
            .map(|rec| rec.registered_at)
            .filter(|&t| t > 0)
            .unwrap_or_else(now_ms);
        // an agent too old to send a profile must not erase the one an older
        // registration already recorded for this same machine
        let device = device.or_else(|| prior.and_then(|rec| rec.device));

        let mut r = self.redis.clone();
        let now = now_ms();
        let all: HashMap<String, String> = r.hgetall(WORKERS_KEY).await.map_err(|e| e.to_string())?;
        for (other_id, raw) in all {
            if other_id == id {
                continue;
            }
            let Ok(rec) = serde_json::from_str::<WorkerRecord>(&raw) else { continue };
            if rec.name == name && now - rec.last_heartbeat > HEARTBEAT_TIMEOUT_MS {
                let _: () = r.hdel(WORKERS_KEY, &other_id).await.map_err(|e| e.to_string())?;
                let _: () = r.del(stats_key(&other_id)).await.map_err(|e| e.to_string())?;
            }
        }

        let rec = serde_json::to_string(&WorkerRecord {
            name: name.to_string(),
            last_heartbeat: now,
            registered_at,
            tags: tags.to_vec(),
            job_id: None,
            stats: None,
            device,
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

    pub async fn heartbeat(&self, id: &str, stats: Option<&WorkerStats>) -> Result<bool, String> {
        let Some(mut rec) = self.get_worker(id).await? else { return Ok(false) };
        let now = now_ms();
        rec.last_heartbeat = now;
        if let Some(stats) = stats {
            rec.stats = Some(stats.clone());
        }
        self.put_worker(id, &rec).await?;

        // append to the per-worker history the dashboard graphs; capped so
        // Redis holds a rolling window, not an unbounded series
        if let Some(stats) = stats {
            let sample = serde_json::to_string(&StatSample { t: now, cpu: stats.cpu_pct, mem: stats.mem_pct }).unwrap();
            let mut r = self.redis.clone();
            let key = stats_key(id);
            let _: () = r.rpush(&key, sample).await.map_err(|e| e.to_string())?;
            let _: () = r.ltrim(&key, -STATS_HISTORY_LEN, -1).await.map_err(|e| e.to_string())?;
        }
        Ok(true)
    }

    /// Recent CPU/RAM sample history for every registered worker.
    pub async fn worker_stats(&self) -> Result<Vec<WorkerStatsSeries>, String> {
        let workers = self.list_workers().await?;
        let mut r = self.redis.clone();
        let mut out = Vec::with_capacity(workers.len());
        for w in workers {
            let raw: Vec<String> = r.lrange(stats_key(&w.id), 0, -1).await.map_err(|e| e.to_string())?;
            let samples = raw.iter().filter_map(|s| serde_json::from_str(s).ok()).collect();
            out.push(WorkerStatsSeries { id: w.id, name: w.name, status: w.status, samples });
        }
        Ok(out)
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
                    registered_at: rec.registered_at,
                    tags: rec.tags,
                    job_id: rec.job_id,
                    stats: rec.stats,
                    device: rec.device,
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

    /// Pick an online worker whose tag set covers every tag the job asks
    /// for; an idle one wins over a busy one.
    async fn find_worker_by_tags(&self, tags: &[String]) -> Result<Option<String>, String> {
        let mut candidates: Vec<Worker> = self
            .list_workers()
            .await?
            .into_iter()
            .filter(|w| matches!(w.status, Status::Online) && tags.iter().all(|t| w.tags.contains(t)))
            .collect();
        candidates.sort_by_key(|w| w.job_id.is_some());
        Ok(candidates.into_iter().next().map(|w| w.id))
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
        branch: &str,
        commit: Option<&Commit>,
        // injected into every job's env (REPO_URL, REPO_BRANCH, COMMIT_SHA)
        // so pipelines can `git clone $REPO_URL` instead of hardcoding it;
        // the job's own YAML env wins on conflicts
        inject_env: &HashMap<String, String>,
        plan: &Plan,
    ) -> Result<i64, String> {
        let now = now_ms();
        let run_id: i64 = sqlx::query_scalar(
            "INSERT INTO runs (pipeline, repo, pipeline_file, trigger_kind, branch, commit_info,
                               status, created_at, started_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $7) RETURNING id",
        )
        .bind(pipeline)
        .bind(repo)
        .bind(pipeline_file)
        .bind(trigger.as_str())
        .bind(branch)
        .bind(commit.map(|c| serde_json::to_value(c).unwrap()))
        .bind(now)
        .fetch_one(&self.db)
        .await
        .map_err(|e| e.to_string())?;

        for job in &plan.jobs {
            let mut env = inject_env.clone();
            env.extend(job.env.clone());
            // `worker: <name>` in the yml is sugar for env WORKER_PIN — the
            // explicit field wins over an env entry
            if let Some(pin) = &job.worker {
                env.insert("WORKER_PIN".to_string(), pin.clone());
            }
            sqlx::query(
                "INSERT INTO jobs (run_id, stage, name, command, needs, env, artifacts, tags) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(run_id)
            .bind(&job.stage)
            .bind(&job.name)
            .bind(&job.command)
            .bind(serde_json::json!(job.needs))
            .bind(serde_json::json!(env))
            .bind(serde_json::json!(job.artifacts))
            .bind(serde_json::json!(job.tags))
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
        let rows = sqlx::query("SELECT id, needs, env, tags FROM jobs WHERE run_id = $1 AND status = 'pending' AND queued = FALSE ORDER BY id")
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

            // `tags:` is a hard constraint: only a worker carrying every
            // tag may run the job. No match online → the job stays
            // unqueued; the reconciler retries placement until one appears.
            let tags: Vec<String> =
                serde_json::from_value(row.get::<serde_json::Value, _>("tags")).unwrap_or_default();
            if target.is_none() && !tags.is_empty() {
                target = self.find_worker_by_tags(&tags).await?;
                if target.is_none() {
                    continue;
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
            // COALESCE keeps the first ready time across requeues, so the wait
            // reported is the total a job spent runnable-but-unstarted.
            let res = sqlx::query(
                "UPDATE jobs SET queued = TRUE, queued_for = $2, ready_at = COALESCE(ready_at, $3)
                 WHERE id = $1 AND queued = FALSE AND status = 'pending'",
            )
            .bind(id)
            .bind(&target)
            .bind(now_ms())
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

        // registry hygiene: a machine silent for a day is gone, not flaky —
        // drop its record so the monitor only lists real fleet members
        for w in &workers {
            if now - w.last_heartbeat > PRUNE_AFTER_MS {
                let _: () = redis.hdel(WORKERS_KEY, &w.id).await.map_err(|e| e.to_string())?;
                let _: () = redis.del(stats_key(&w.id)).await.map_err(|e| e.to_string())?;
            }
        }

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
                // ready_at deliberately survives: the job was runnable before
                // the worker died and is runnable again, so the wait continues.
                sqlx::query(
                    "UPDATE jobs SET status = 'pending', queued = FALSE, queued_for = NULL,
                     worker = NULL, worker_id = NULL, started_at = NULL,
                     requeue_count = requeue_count + 1 WHERE id = $1",
                )
                .bind(row.get::<i64, _>("id"))
                .execute(&self.db)
                .await
                .map_err(|e| e.to_string())?;
                touched = true;
            }
        }

        // Always retry placement of pending unqueued jobs — a tag- or
        // pin-constrained job waits here until a matching worker shows up.
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
            if touched {
                self.roll_up(run_id).await?;
            }
        }
        Ok(())
    }

    pub async fn report_job(&self, job_id: i64, req: &ReportRequest) -> Result<(), String> {
        let first_error = matches!(req.status, JobStatus::Failed)
            .then(|| first_error_line(&req.output))
            .flatten();
        let row = sqlx::query(
            "UPDATE jobs SET status = $2, output = $3, exit_code = $4, finished_at = $5,
             first_error = $6 WHERE id = $1
             RETURNING run_id, worker_id",
        )
        .bind(job_id)
        .bind(req.status.as_str())
        .bind(&req.output)
        .bind(req.exit_code)
        .bind(now_ms())
        .bind(&first_error)
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
        let job_rows = sqlx::query("SELECT id, run_id, stage, name, command, needs, env, queued, status, worker, worker_id, ready_at, requeue_count, started_at, finished_at, exit_code, first_error, artifacts, has_artifacts, tags FROM jobs WHERE run_id = ANY($1) ORDER BY id")
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
        let rows = sqlx::query("SELECT id, run_id, stage, name, command, needs, env, queued, status, worker, worker_id, ready_at, requeue_count, started_at, finished_at, exit_code, first_error, artifacts, has_artifacts, tags FROM jobs ORDER BY id")
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

    /// A job's log so far plus whether it can still grow. The log stream polls
    /// this: `done` is what tells the stream to close rather than hold a
    /// connection open forever on a job that finished minutes ago.
    pub async fn job_tail(&self, job_id: i64) -> Result<Option<(String, bool)>, String> {
        let row = sqlx::query("SELECT output, status FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(|r| {
            let status = JobStatus::from_str(r.get("status"));
            let done = matches!(status, JobStatus::Passed | JobStatus::Failed);
            (r.get::<Option<String>, _>("output").unwrap_or_default(), done)
        }))
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
        // Delivery state is joined in on read and must not ride along into the
        // blob: this write is the refresh that would otherwise stale it.
        let repo = &Repo { webhook: None, ..repo.clone() };
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
        let mut repos: Vec<Repo> = rows
            .into_iter()
            .filter_map(|r| serde_json::from_value(r.get::<serde_json::Value, _>("data")).ok())
            .collect();

        let deliveries = sqlx::query("SELECT repo, last_at, event, status, detail FROM webhook_deliveries")
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        let by_repo: HashMap<String, WebhookDelivery> = deliveries
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("repo"),
                    WebhookDelivery {
                        last_at: r.get("last_at"),
                        event: r.get("event"),
                        status: r.get("status"),
                        detail: r.get("detail"),
                    },
                )
            })
            .collect();
        for repo in &mut repos {
            repo.webhook = by_repo.get(&repo.name).cloned();
        }
        Ok(repos)
    }

    /// Claim `minute` for a scheduled pipeline, returning true exactly once
    /// across every coordinator that asks.
    ///
    /// The whole guard is the `WHERE last_fired < $2` on the upsert: whoever
    /// commits first moves the watermark, and everyone else affects zero rows.
    /// Without it a restart at 02:00:30 would run the 02:00 job a second time.
    pub async fn claim_schedule(&self, key: &str, minute: i64) -> Result<bool, String> {
        let res = sqlx::query(
            "INSERT INTO schedule_state (key, last_fired) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET last_fired = EXCLUDED.last_fired
             WHERE schedule_state.last_fired < $2",
        )
        .bind(key)
        .bind(minute)
        .execute(&self.db)
        .await
        .map_err(|e| e.to_string())?;
        Ok(res.rows_affected() > 0)
    }

    /// Record that a push arrived for `repo`. Called for accepted *and*
    /// rejected deliveries: a rejection still proves the hook is wired up, and
    /// which kind it was is the whole diagnostic.
    pub async fn record_delivery(
        &self,
        repo: &str,
        event: Option<&str>,
        status: &str,
        detail: Option<&str>,
    ) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO webhook_deliveries (repo, last_at, event, status, detail)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (repo) DO UPDATE SET
               last_at = EXCLUDED.last_at, event = EXCLUDED.event,
               status  = EXCLUDED.status,  detail = EXCLUDED.detail",
        )
        .bind(repo)
        .bind(now_ms())
        .bind(event)
        .bind(status)
        .bind(detail)
        .execute(&self.db)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
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

    // ---- dashboard sessions (Redis) --------------------------------------

    /// Mint a login session for the dashboard; expires on its own in Redis.
    pub async fn create_session(&self) -> Result<String, String> {
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let mut r = self.redis.clone();
        let _: () = r
            .set_ex(format!("dash:session:{token}"), 1, SESSION_TTL_SECS)
            .await
            .map_err(|e| e.to_string())?;
        Ok(token)
    }

    pub async fn session_valid(&self, token: &str) -> Result<bool, String> {
        let mut r = self.redis.clone();
        r.exists(format!("dash:session:{token}")).await.map_err(|e| e.to_string())
    }

    // ---- calendar -------------------------------------------------------

    pub async fn calendar(&self) -> Result<Vec<CalendarDay>, String> {
        let starts: Vec<i64> = sqlx::query("SELECT started_at FROM runs")
            .fetch_all(&self.db)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| r.get::<i64, _>("started_at"))
            .collect();

        Ok(calendar_from(starts))
    }

    /// Everything the per-device page shows about what a machine has done.
    ///
    /// Scoped by worker NAME, so a box that lost its id file and re-registered
    /// keeps its history — to the person reading the page it is one machine.
    pub async fn worker_activity(&self, name: &str) -> Result<WorkerActivity, String> {
        let rows = sqlx::query(
            "SELECT j.id, j.run_id, j.stage, j.name, j.status, j.started_at, j.finished_at,
                    r.repo, r.pipeline
             FROM jobs j JOIN runs r ON r.id = j.run_id
             WHERE j.worker = $1 AND j.started_at IS NOT NULL
             ORDER BY j.started_at DESC",
        )
        .bind(name)
        .fetch_all(&self.db)
        .await
        .map_err(|e| e.to_string())?;

        let jobs: Vec<WorkerJob> = rows
            .into_iter()
            .map(|r| WorkerJob {
                job_id: r.get("id"),
                run_id: r.get("run_id"),
                repo: r.get("repo"),
                pipeline: r.get("pipeline"),
                stage: r.get("stage"),
                name: r.get("name"),
                status: JobStatus::from_str(r.get("status")),
                started_at: r.get("started_at"),
                finished_at: r.get("finished_at"),
            })
            .collect();

        let passed = jobs.iter().filter(|j| j.status == JobStatus::Passed).count() as i64;
        let failed = jobs.iter().filter(|j| j.status == JobStatus::Failed).count() as i64;
        // only finished jobs have a duration; a job still running would
        // otherwise drag the median toward zero as it is counted at 0ms
        let mut durations: Vec<i64> = jobs
            .iter()
            .filter_map(|j| match (j.started_at, j.finished_at) {
                (Some(s), Some(f)) if f >= s => Some(f - s),
                _ => None,
            })
            .collect();
        let busy_ms = durations.iter().sum();
        durations.sort_unstable();
        let median_ms = durations.get(durations.len() / 2).copied();

        Ok(WorkerActivity {
            name: name.to_string(),
            calendar: calendar_from(jobs.iter().filter_map(|j| j.started_at).collect()),
            total_jobs: jobs.len() as i64,
            passed,
            failed,
            busy_ms,
            median_ms,
            recent: jobs.into_iter().take(40).collect(),
        })
    }
}

impl Store {
    /// The Insights window: every job in the last `range_days`, flattened with
    /// its run. One join rather than eight aggregate queries — the derivation
    /// lives in `insights` as pure functions, which is where it can be tested.
    /// `output` is excluded; nothing on that page reads a build log.
    pub async fn insights(&self, range_days: i64) -> Result<insights::Insights, String> {
        let now = now_ms();
        let from = now - range_days * 24 * 60 * 60 * 1000;
        let rows = sqlx::query(
            "SELECT j.run_id, j.stage, j.name AS job_name, j.status AS job_status,
                    j.ready_at, j.started_at, j.finished_at, j.exit_code, j.first_error, j.worker,
                    r.pipeline, r.repo, r.commit_info, r.status AS run_status,
                    r.started_at AS run_started_at, r.finished_at AS run_finished_at
             FROM jobs j JOIN runs r ON r.id = j.run_id
             WHERE r.started_at >= $1
             ORDER BY r.started_at",
        )
        .bind(from)
        .fetch_all(&self.db)
        .await
        .map_err(|e| e.to_string())?;

        let rows: Vec<insights::Row> = rows
            .into_iter()
            .map(|r| insights::Row {
                run_id: r.get("run_id"),
                pipeline: r.get("pipeline"),
                repo: r.get("repo"),
                commit_sha: r
                    .get::<Option<serde_json::Value>, _>("commit_info")
                    .and_then(|v| serde_json::from_value::<Commit>(v).ok())
                    .map(|c| c.sha),
                run_status: JobStatus::from_str(r.get("run_status")),
                run_started_at: r.get("run_started_at"),
                run_finished_at: r.get("run_finished_at"),
                stage: r.get("stage"),
                job_name: r.get("job_name"),
                job_status: JobStatus::from_str(r.get("job_status")),
                ready_at: r.get("ready_at"),
                started_at: r.get("started_at"),
                finished_at: r.get("finished_at"),
                exit_code: r.get("exit_code"),
                first_error: r.get("first_error"),
                worker: r.get("worker"),
            })
            .collect();

        Ok(insights::build(&rows, range_days, now, self.calendar().await?))
    }
}

/// Bucket ms timestamps into a trailing year of daily counts, oldest first.
/// Days with nothing are present with a count of 0 — a contribution calendar
/// has to render the gaps, so the absence is data.
fn calendar_from(timestamps: Vec<i64>) -> Vec<CalendarDay> {
    use chrono::{Duration, TimeZone};
    let mut counts: HashMap<String, u32> = HashMap::new();
    for ms in timestamps {
        if let Some(d) = Local.timestamp_millis_opt(ms).single() {
            *counts.entry(d.format("%Y-%m-%d").to_string()).or_default() += 1;
        }
    }
    let today = Local::now().date_naive();
    (0..364)
        .rev()
        .map(|i| {
            let date = (today - Duration::days(i)).format("%Y-%m-%d").to_string();
            let count = counts.get(&date).copied().unwrap_or(0);
            CalendarDay { date, count }
        })
        .collect()
}

/// The most useful single line from a failed job's log.
///
/// Prefers the first line that looks like an error, and falls back to the last
/// non-empty line — a command that dies without saying "error" still leaves its
/// last words. Truncated so a runaway line cannot bloat every list response.
pub fn first_error_line(output: &str) -> Option<String> {
    const MAX: usize = 200;
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let pick = lines
        .iter()
        .find(|l| {
            let lower = l.to_ascii_lowercase();
            ["error", "panicked", "fatal", "exception", "traceback", "failed"]
                .iter()
                .any(|k| lower.contains(k))
        })
        .or_else(|| lines.last())?;
    let mut out = pick.to_string();
    if out.chars().count() > MAX {
        out = out.chars().take(MAX - 1).collect::<String>() + "…";
    }
    Some(out)
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
        tags: serde_json::from_value(row.get::<serde_json::Value, _>("tags")).unwrap_or_default(),
        status: JobStatus::from_str(&row.get::<String, _>("status")),
        worker: row.get("worker"),
        worker_id: row.get("worker_id"),
        ready_at: row.get("ready_at"),
        requeue_count: row.get("requeue_count"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        exit_code: row.get("exit_code"),
        first_error: row.try_get("first_error").unwrap_or(None),
        // absent from list queries by design; present on single-run/job reads
        output: row.try_get("output").unwrap_or(None),
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
        // try_get: runs created before the column existed have no branch, and
        // "unknown" is the honest answer rather than a guessed default
        branch: row.try_get::<Option<String>, _>("branch").ok().flatten(),
        commit,
        status: JobStatus::from_str(&row.get::<String, _>("status")),
        created_at: row.get("created_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        jobs: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::first_error_line;

    #[test]
    fn prefers_the_first_line_that_looks_like_an_error() {
        let log = "Compiling orchestrator\n   warning: unused\nerror[E0308]: mismatched types\n  --> src/x.rs:4";
        assert_eq!(first_error_line(log).unwrap(), "error[E0308]: mismatched types");
    }

    #[test]
    fn falls_back_to_the_last_line_when_nothing_says_error() {
        // a command can die without ever printing the word "error"
        let log = "running migrations\napplying 003_add_index\nkilled";
        assert_eq!(first_error_line(log).unwrap(), "killed");
    }

    #[test]
    fn ignores_blank_and_whitespace_lines() {
        assert_eq!(first_error_line("\n\n   \n  boom  \n\n").unwrap(), "boom");
    }

    #[test]
    fn empty_output_has_no_cause_rather_than_an_empty_one() {
        assert!(first_error_line("").is_none());
        assert!(first_error_line("   \n \n").is_none());
    }

    #[test]
    fn a_runaway_line_is_truncated_so_it_cannot_bloat_list_responses() {
        let long = format!("error: {}", "x".repeat(5_000));
        let got = first_error_line(&long).unwrap();
        assert_eq!(got.chars().count(), 200);
        assert!(got.ends_with('…'));
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // slicing by byte here would split a multi-byte char and panic
        let long = format!("fatal: {}", "✕".repeat(400));
        let got = first_error_line(&long).unwrap();
        assert_eq!(got.chars().count(), 200);
    }
}
