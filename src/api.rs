use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::sse::{self, Sse};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tower_http::cors::CorsLayer;

use crate::forgejo;
use crate::pipeline::{self, Plan};
use crate::store::SharedStore;
use crate::types::{
    AddRepoRequest, CalendarDay, ClaimedJob, Commit, HealthReport, Job, RegisterResponse, Repo, ReportRequest, Run,
    TriggerKind, TriggerRequest, Worker, WorkerActivity, WorkerRequest, WorkerStatsSeries,
};

type ApiError = (StatusCode, String);

fn internal(e: String) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, e)
}

/// Dashboard auth is on when both env vars are set; workers and webhooks
/// are never behind it (they authenticate machines, not people — see the
/// route split in `router`).
fn dashboard_creds() -> Option<(String, String)> {
    let user = std::env::var("DASHBOARD_USERNAME").ok().filter(|s| !s.is_empty())?;
    let pass = std::env::var("DASHBOARD_PASSWORD").ok().filter(|s| !s.is_empty())?;
    Some((user, pass))
}

async fn require_session(
    State(store): State<SharedStore>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if dashboard_creds().is_none() {
        return Ok(next.run(req).await);
    }
    let header_token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_string();

    // EventSource cannot set headers, so the log stream — and ONLY the log
    // stream — also accepts the session token as a query parameter. Without
    // this, turning on dashboard auth would silently downgrade live logs to
    // polling. Kept to one path on purpose: tokens in URLs end up in access
    // logs, and that is a cost worth paying once, not everywhere.
    let query_token = if req.uri().path().ends_with("/logs/stream") {
        req.uri()
            .query()
            .and_then(|q| {
                q.split('&')
                    .find_map(|kv| kv.strip_prefix("token=").map(|v| v.to_string()))
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    let token = if header_token.is_empty() { query_token } else { header_token };
    if !token.is_empty() && store.session_valid(&token).await.map_err(internal)? {
        return Ok(next.run(req).await);
    }
    Err((StatusCode::UNAUTHORIZED, "sign in required".to_string()))
}

pub fn router(store: SharedStore) -> Router {
    // Everything the browser dashboard reads or triggers sits behind the
    // login session; worker/executor/webhook traffic stays open so machines
    // keep working without a password.
    let dashboard = Router::new()
        .route("/api/workers", get(list_workers))
        .route("/api/workers/stats", get(worker_stats))
        .route("/api/workers/{name}/activity", get(worker_activity))
        .route("/api/pipelines/trigger", post(trigger_pipeline))
        .route("/api/jobs", get(list_jobs))
        .route("/api/runs", get(list_runs))
        .route("/api/runs/{id}", get(get_run))
        .route("/api/jobs/{id}/logs", get(job_logs))
        .route("/api/jobs/{id}/logs/stream", get(job_log_stream))
        .route("/api/repos", get(list_repos).post(add_repo))
        .route("/api/repos/{name}", delete(delete_repo))
        .route("/api/repos/{name}/pipeline", get(pipeline_file))
        .route("/api/activity/calendar", get(calendar))
        .route("/api/insights", get(insights))
        .route_layer(middleware::from_fn_with_state(store.clone(), require_session));

    let router = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(login))
        .route("/api/workers/register", post(register))
        .route("/api/workers/heartbeat", post(heartbeat))
        .route("/api/webhooks/forgejo", post(forgejo_webhook))
        .route("/api/jobs/claim", post(claim_job))
        .route("/api/jobs/{id}/report", post(report_job))
        .route("/api/jobs/{id}/progress", post(job_progress))
        .route("/api/jobs/{id}/artifacts", post(upload_artifacts).get(download_artifacts))
        .merge(dashboard)
        // dashboard dev server runs on another origin (127.0.0.1:4173)
        .layer(CorsLayer::permissive())
        // artifact tarballs exceed axum's 2MB default body cap
        .layer(axum::extract::DefaultBodyLimit::max(512 * 1024 * 1024))
        .with_state(store);

    // Serve the built dashboard when it's around (hash routing — no URL
    // rewrites needed), so one container/binary is the whole UI + API.
    let dist = std::env::var("DASHBOARD_DIST").unwrap_or_else(|_| "dashboard/dist".into());
    if std::path::Path::new(&dist).join("index.html").exists() {
        println!("Serving dashboard from {dist}");
        router.fallback_service(tower_http::services::ServeDir::new(&dist))
    } else {
        router.route("/", get(root))
    }
}

async fn root() -> &'static str {
    "Coordinator Online"
}

async fn health(State(store): State<SharedStore>) -> Result<Json<HealthReport>, ApiError> {
    let online_workers = store.online_workers().await.map_err(internal)?;
    Ok(Json(HealthReport { health: "Ok", online_workers }))
}

/// Tells the dashboard whether to show the login screen.
async fn auth_status() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "required": dashboard_creds().is_some() }))
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn login(
    State(store): State<SharedStore>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some((user, pass)) = dashboard_creds() else {
        // no creds configured — login is a no-op, hand out a token anyway
        let token = store.create_session().await.map_err(internal)?;
        return Ok(Json(serde_json::json!({ "token": token })));
    };
    if req.username != user || req.password != pass {
        return Err((StatusCode::UNAUTHORIZED, "wrong username or password".to_string()));
    }
    let token = store.create_session().await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "token": token })))
}

async fn list_workers(State(store): State<SharedStore>) -> Result<Json<Vec<Worker>>, ApiError> {
    Ok(Json(store.list_workers().await.map_err(internal)?))
}

/// Rolling CPU/RAM history per worker (fed by heartbeats) — the dashboard's
/// device monitor graphs read this.
async fn worker_stats(State(store): State<SharedStore>) -> Result<Json<Vec<WorkerStatsSeries>>, ApiError> {
    Ok(Json(store.worker_stats().await.map_err(internal)?))
}

/// What one machine has done: its job history, calendar and totals. Answers
/// for a name with no jobs rather than 404ing — a worker that has registered
/// but not yet run anything is a real, empty state the page must show.
async fn worker_activity(
    State(store): State<SharedStore>,
    Path(name): Path<String>,
) -> Result<Json<WorkerActivity>, ApiError> {
    Ok(Json(store.worker_activity(&name).await.map_err(internal)?))
}

async fn register(
    State(store): State<SharedStore>,
    Json(req): Json<WorkerRequest>,
) -> Result<Json<RegisterResponse>, ApiError> {
    let worker_id = store
        .register(&req.worker_name, req.worker_id.clone(), &req.tags, req.device.clone())
        .await
        .map_err(internal)?;
    println!(
        "Worker {} registered as {worker_id}{}{}",
        req.worker_name,
        if req.tags.is_empty() { String::new() } else { format!(" (tags: {})", req.tags.join(", ")) },
        req.device
            .as_ref()
            .map(|d| format!(" [{} · {} cores]", d.os.as_deref().unwrap_or(&d.os_id), d.cpu_cores))
            .unwrap_or_default()
    );
    Ok(Json(RegisterResponse { worker_id }))
}

/// Workers identify by id after registration; plain-name callers (demo
/// scripts) fall back to using the name as the id.
fn ident(req: &WorkerRequest) -> String {
    req.worker_id.clone().unwrap_or_else(|| req.worker_name.clone())
}

async fn heartbeat(
    State(store): State<SharedStore>,
    Json(req): Json<WorkerRequest>,
) -> Result<(), ApiError> {
    let known = store.heartbeat(&ident(&req), req.stats.as_ref()).await.map_err(internal)?;
    if !known {
        println!("Worker {} not known!", req.worker_name);
    }
    Ok(())
}

/// Fetch + parse a repo's pipeline YAML from Forgejo at the given branch.
async fn plan_for_repo(repo: &Repo, branch: &str) -> Result<(Plan, String), ApiError> {
    let remote = repo.remote.clone().ok_or((
        StatusCode::BAD_REQUEST,
        format!("repo '{}' has no remote configured", repo.name),
    ))?;

    let client = reqwest::Client::new();
    let mut candidates: Vec<String> = repo.pipelines.iter().map(|p| p.file.clone()).collect();
    candidates.extend(forgejo::PIPELINE_FILES.iter().map(|f| f.to_string()));
    candidates.dedup();

    for file in candidates {
        if let Some(yaml) = forgejo::fetch_raw_file(&client, &remote, branch, &file).await {
            let plan = pipeline::plan_from_yaml(&yaml)
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("{file}: {e}")))?;
            return Ok((plan, file));
        }
    }
    Err((
        StatusCode::BAD_REQUEST,
        format!("repo '{}' has no pipeline file on '{branch}' — add .orchestrator/actions.yml", repo.name),
    ))
}

/// Everything a trigger resolved: the plan plus where it came from, so the
/// run can label itself and jobs get REPO_URL/REPO_BRANCH/COMMIT_SHA env.
struct RunSource {
    plan: Plan,
    repo: String,
    file: String,
    remote: Option<String>,
    branch: String,
}

/// Env injected into every job so pipelines never hardcode their repo:
/// `git clone $REPO_URL` works in any repo's actions.yml.
fn run_env(remote: Option<&str>, branch: &str, sha: Option<&str>) -> std::collections::HashMap<String, String> {
    let mut env = std::collections::HashMap::new();
    if let Some(remote) = remote {
        env.insert("REPO_URL".to_string(), remote.to_string());
    }
    env.insert("REPO_BRANCH".to_string(), branch.to_string());
    if let Some(sha) = sha {
        env.insert("COMMIT_SHA".to_string(), sha.to_string());
    }
    env
}

/// Resolve which pipeline to run: the named repo's YAML from Forgejo; with
/// no name, the single registered repo, else this checkout's own
/// .orchestrator/actions.yml. No fake fallback plans.
async fn resolve_plan(
    store: &SharedStore,
    repo_name: Option<String>,
) -> Result<RunSource, ApiError> {
    // an unregistered name (e.g. retrying an old run) falls through
    let repo = match repo_name {
        Some(name) => store.get_repo(&name).await.map_err(internal)?,
        None => {
            let repos = store.list_repos().await.map_err(internal)?;
            // bare trigger with exactly one registered repo → run that repo
            if repos.len() == 1 { repos.into_iter().next() } else { None }
        }
    };

    let mut fallback: Option<(String, Option<String>, String)> = None;
    if let Some(repo) = repo {
        let branch = repo.branch.clone();
        match plan_for_repo(&repo, &branch).await {
            Ok((plan, file)) => {
                return Ok(RunSource { plan, repo: repo.name, file, remote: repo.remote, branch });
            }
            // not on Forgejo (yet) — fall through to this checkout's own
            // actions.yml, still grouping the run under the repo
            Err((_, reason)) => {
                println!("{reason}; trying the local .orchestrator/actions.yml");
                fallback = Some((repo.name, repo.remote, branch));
            }
        }
    }

    let yaml = std::fs::read_to_string(".orchestrator/actions.yml").map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "nothing to run: register a repo with a pipeline file, or add .orchestrator/actions.yml next to the coordinator".to_string(),
        )
    })?;
    let plan = pipeline::plan_from_yaml(&yaml)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let (repo, remote, branch) =
        fallback.unwrap_or_else(|| (plan.name.clone(), None, "main".to_string()));
    Ok(RunSource { plan, repo, file: ".orchestrator/actions.yml".into(), remote, branch })
}

async fn trigger_pipeline(
    State(store): State<SharedStore>,
    body: Option<Json<TriggerRequest>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let repo_name = body.and_then(|Json(b)| b.repo);
    let src = resolve_plan(&store, repo_name).await?;

    let env = run_env(src.remote.as_deref(), &src.branch, None);
    let id = store
        .create_run(&src.plan.name, &src.repo, &src.file, TriggerKind::Manual, &src.branch, None, &env, &src.plan)
        .await
        .map_err(internal)?;
    println!("Triggered run {id}: pipeline '{}' from {} ({} jobs)", src.plan.name, src.file, src.plan.jobs.len());
    Ok(Json(serde_json::json!({ "id": id })))
}

/// Start a run from a pipeline's `schedule:`. Shares the whole path a webhook
/// takes — same planner, same env injection — so a scheduled run differs from
/// a pushed one only in its trigger kind and in having no commit attached.
pub async fn start_scheduled_run(
    store: &SharedStore,
    repo: &Repo,
    pipeline: &crate::types::PipelineRef,
) -> Result<i64, String> {
    let branch = repo.branch.clone();
    let yaml = forgejo::fetch_raw_file(
        &reqwest::Client::new(),
        repo.remote.as_deref().ok_or("repo has no remote")?,
        &branch,
        &pipeline.file,
    )
    .await
    .ok_or_else(|| format!("{} is gone from {branch}", pipeline.file))?;
    let plan = pipeline::plan_from_yaml(&yaml).await?;

    // No commit: a schedule fires against whatever is on the branch, and
    // inventing a sha here would put a lie in the run's history.
    let env = run_env(repo.remote.as_deref(), &branch, None);
    store
        .create_run(&plan.name, &repo.name, &pipeline.file, TriggerKind::Schedule, &branch, None, &env, &plan)
        .await
}

/// Forgejo push webhook. Point the repo's webhook at
/// POST http://<coordinator>/api/webhooks/forgejo (content type JSON).
async fn forgejo_webhook(
    State(store): State<SharedStore>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use serde_json::Value;

    let event = headers
        .get("x-forgejo-event")
        .or_else(|| headers.get("x-gitea-event"))
        .or_else(|| headers.get("x-github-event"))
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let full_name = payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str)
        .ok_or((StatusCode::BAD_REQUEST, "not a Forgejo push payload (no repository.full_name)".to_string()))?;

    let repo = store
        .list_repos()
        .await
        .map_err(internal)?
        .into_iter()
        .find(|r| {
            r.remote
                .as_deref()
                .map(|rem| rem.to_lowercase().ends_with(&full_name.to_lowercase()))
                .unwrap_or(false)
        })
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("push from {full_name}, but that repo is not registered in the dashboard"),
        ))?;

    let branch = payload
        .get("ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("refs/heads/"))
        .unwrap_or(&repo.branch)
        .to_string();

    let commit = payload
        .get("head_commit")
        .filter(|c| !c.is_null())
        .map(|c| {
            let files = ["added", "modified", "removed"]
                .iter()
                .flat_map(|k| c.get(*k).and_then(Value::as_array).cloned().unwrap_or_default())
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            Commit {
                sha: c.get("id").and_then(Value::as_str).unwrap_or("").chars().take(7).collect(),
                message: c
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string(),
                author: c
                    .pointer("/author/username")
                    .or_else(|| c.pointer("/author/name"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                files,
            }
        });

    // full sha (not the 7-char display one) so checkout jobs can pin to it
    let full_sha = payload
        .pointer("/head_commit/id")
        .or_else(|| payload.get("after"))
        .and_then(Value::as_str)
        .map(str::to_string);

    // Record the delivery before doing anything that can fail, so a repo whose
    // pipeline file is broken still shows "the hook works, the file does not"
    // rather than looking like nothing ever arrived.
    let plan_result = plan_for_repo(&repo, &branch).await;
    let outcome = match &plan_result {
        Ok(_) => ("accepted", None),
        Err((_, why)) => ("rejected", Some(why.as_str())),
    };
    if let Err(e) = store.record_delivery(&repo.name, event.as_deref(), outcome.0, outcome.1).await {
        // never fail a webhook over bookkeeping — the run matters more
        eprintln!("could not record webhook delivery for {}: {e}", repo.name);
    }
    let (plan, file) = plan_result?;
    let env = run_env(repo.remote.as_deref(), &branch, full_sha.as_deref());
    let id = store
        .create_run(&plan.name, &repo.name, &file, TriggerKind::Webhook, &branch, commit.as_ref(), &env, &plan)
        .await
        .map_err(internal)?;
    println!(
        "Webhook run {id}: {} pushed to {full_name}@{branch} — pipeline '{}' ({} jobs)",
        commit.as_ref().map(|c| c.sha.as_str()).unwrap_or("?"),
        plan.name,
        plan.jobs.len()
    );
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn list_jobs(State(store): State<SharedStore>) -> Result<Json<Vec<Job>>, ApiError> {
    Ok(Json(store.list_jobs().await.map_err(internal)?))
}

async fn claim_job(
    State(store): State<SharedStore>,
    Json(req): Json<WorkerRequest>,
) -> Result<Json<Option<ClaimedJob>>, ApiError> {
    Ok(Json(
        store.claim_job(&ident(&req), &req.worker_name).await.map_err(internal)?,
    ))
}

fn artifacts_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("ARTIFACTS_DIR").unwrap_or_else(|_| "artifacts".into()))
}

/// Executors upload a job's declared outputs here as one tar.gz; dependent
/// jobs (possibly on other machines) download it back. The coordinator is
/// the only file transport between devices.
async fn upload_artifacts(
    Path(id): Path<i64>,
    State(store): State<SharedStore>,
    body: axum::body::Bytes,
) -> Result<(), ApiError> {
    let dir = artifacts_dir();
    tokio::fs::create_dir_all(&dir).await.map_err(|e| internal(e.to_string()))?;
    tokio::fs::write(dir.join(format!("{id}.tar.gz")), &body)
        .await
        .map_err(|e| internal(e.to_string()))?;
    store.mark_artifacts(id).await.map_err(internal)?;
    println!("Artifacts stored for job {id} ({} bytes)", body.len());
    Ok(())
}

async fn download_artifacts(Path(id): Path<i64>) -> Result<axum::body::Bytes, ApiError> {
    tokio::fs::read(artifacts_dir().join(format!("{id}.tar.gz")))
        .await
        .map(axum::body::Bytes::from)
        .map_err(|_| (StatusCode::NOT_FOUND, format!("no artifacts stored for job {id}")))
}

async fn report_job(
    Path(id): Path<i64>,
    State(store): State<SharedStore>,
    Json(req): Json<ReportRequest>,
) -> Result<(), ApiError> {
    store.report_job(id, &req).await.map_err(internal)?;
    println!("Job {} reported: {:?}", id, req.status);
    Ok(())
}

#[derive(serde::Deserialize)]
struct ProgressReport {
    output: String,
}

/// Live tail from the executor while a job runs; the dashboard's normal
/// log polling picks it up, so long jobs stream instead of appearing all
/// at once at the end.
async fn job_progress(
    Path(id): Path<i64>,
    State(store): State<SharedStore>,
    Json(req): Json<ProgressReport>,
) -> Result<(), ApiError> {
    store.job_progress(id, &req.output).await.map_err(internal)?;
    Ok(())
}

async fn list_runs(State(store): State<SharedStore>) -> Result<Json<Vec<Run>>, ApiError> {
    Ok(Json(store.list_runs().await.map_err(internal)?))
}

async fn get_run(
    Path(id): Path<i64>,
    State(store): State<SharedStore>,
) -> Result<Json<Run>, ApiError> {
    store
        .get_run(id)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "run not found".into()))
}

async fn job_logs(
    Path(id): Path<i64>,
    State(store): State<SharedStore>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let output = store
        .job_output(id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "job not found".into()))?;
    Ok(Json(serde_json::json!({ "output": output })))
}

/// How often the log stream looks for new output. The executor posts its
/// growing log roughly every two seconds, so polling faster than this buys
/// nothing; polling slower makes a live log feel like a page refresh.
const LOG_POLL_MS: u64 = 400;

/// Server-sent events carrying a job's log as it is written.
///
/// Deliberately a poll over the store rather than a broadcast channel fed by
/// `job_progress`: the log has two possible writers (the worker forwarding its
/// executor's tail, or the executor posting straight here), and a channel would
/// only see one of them. Reading the row is the one place both agree.
///
/// Each event carries only the bytes since the last one, so a long build does
/// not resend its whole log every tick. The stream closes itself once the job
/// reaches a terminal state and its last bytes have gone out.
async fn job_log_stream(
    Path(id): Path<i64>,
    State(store): State<SharedStore>,
) -> Sse<impl futures::Stream<Item = Result<sse::Event, std::convert::Infallible>>> {
    struct S {
        store: SharedStore,
        id: i64,
        sent: usize,
        first: bool,
        finished: bool,
    }

    let stream = futures::stream::unfold(
        S { store, id, sent: 0, first: true, finished: false },
        |mut s| async move {
            if s.finished {
                return None;
            }
            if s.first {
                s.first = false;
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(LOG_POLL_MS)).await;
            }

            let Ok(Some((output, done))) = s.store.job_tail(s.id).await else {
                // a job that vanished (or a database blip) ends the stream
                // rather than spinning: the client falls back to polling
                s.finished = true;
                return Some((Ok(sse::Event::default().event("end").data("gone")), s));
            };

            // `get` rather than indexing: a rewritten log can leave the mark
            // inside a multi-byte character, and slicing there would panic
            let chunk = output.get(s.sent..).unwrap_or("").to_string();
            s.sent = output.len();

            if done {
                s.finished = true;
                if chunk.is_empty() {
                    return Some((Ok(sse::Event::default().event("end").data("done")), s));
                }
            }
            let event = if chunk.is_empty() {
                // an empty comment keeps the connection warm without the
                // client having to distinguish "no news" from "new blank line"
                sse::Event::default().comment("idle")
            } else {
                sse::Event::default().event("log").data(chunk)
            };
            Some((Ok(event), s))
        },
    );

    Sse::new(stream).keep_alive(sse::KeepAlive::default())
}

async fn list_repos(State(store): State<SharedStore>) -> Result<Json<Vec<Repo>>, ApiError> {
    Ok(Json(store.list_repos().await.map_err(internal)?))
}

async fn add_repo(
    State(store): State<SharedStore>,
    Json(req): Json<AddRepoRequest>,
) -> Result<Json<Repo>, ApiError> {
    let client = reqwest::Client::new();
    let repo = forgejo::fetch_repo(&client, &req.remote)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    store.upsert_repo(&repo).await.map_err(internal)?;
    println!("Repo {}/{} registered", repo.owner, repo.name);
    Ok(Json(repo))
}

async fn delete_repo(
    Path(name): Path<String>,
    State(store): State<SharedStore>,
) -> Result<StatusCode, ApiError> {
    if store.delete_repo(&name).await.map_err(internal)? {
        println!("Repo {name} unregistered");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, format!("repo '{name}' is not registered")))
    }
}

#[derive(Deserialize)]
struct PipelineFileQuery {
    file: Option<String>,
}

/// Raw pipeline YAML for a repo, proxied from Forgejo so the browser never
/// talks to it directly. ?file= pins a specific path; otherwise the known
/// candidates are probed in order.
async fn pipeline_file(
    Path(name): Path<String>,
    Query(q): Query<PipelineFileQuery>,
    State(store): State<SharedStore>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let repo = store
        .get_repo(&name)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("repo '{name}' is not registered")))?;
    let remote = repo.remote.clone().ok_or((
        StatusCode::BAD_REQUEST,
        format!("repo '{name}' has no remote configured"),
    ))?;

    let candidates: Vec<String> = match q.file {
        Some(file) => vec![file],
        None => {
            let mut c: Vec<String> = repo.pipelines.iter().map(|p| p.file.clone()).collect();
            c.extend(forgejo::PIPELINE_FILES.iter().map(|f| f.to_string()));
            c.dedup();
            c
        }
    };

    let client = reqwest::Client::new();
    for file in candidates {
        if let Some(content) = forgejo::fetch_raw_file(&client, &remote, &repo.branch, &file).await {
            return Ok(Json(serde_json::json!({ "file": file, "content": content })));
        }
    }
    Err((
        StatusCode::NOT_FOUND,
        format!("no pipeline file found on '{}' — push .orchestrator/actions.yml first", repo.branch),
    ))
}

/// Everything the Insights page shows, for one window. `range` is a day count;
/// anything outside 1..=365 is clamped rather than rejected, since a bad query
/// string should not blank the page.
#[derive(serde::Deserialize)]
struct InsightsQuery {
    #[serde(default)]
    range: Option<i64>,
}

async fn insights(
    State(store): State<SharedStore>,
    Query(q): Query<InsightsQuery>,
) -> Result<Json<crate::insights::Insights>, ApiError> {
    let days = q.range.unwrap_or(30).clamp(1, 365);
    Ok(Json(store.insights(days).await.map_err(internal)?))
}

async fn calendar(State(store): State<SharedStore>) -> Result<Json<Vec<CalendarDay>>, ApiError> {
    Ok(Json(store.calendar().await.map_err(internal)?))
}
