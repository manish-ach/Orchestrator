use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::cors::CorsLayer;

use crate::forgejo;
use crate::pipeline::{self, Plan};
use crate::store::SharedStore;
use crate::types::{
    AddRepoRequest, CalendarDay, Commit, HealthReport, Job, Repo, ReportRequest, Run, TriggerKind, TriggerRequest,
    Worker, WorkerRequest,
};

type ApiError = (StatusCode, String);

fn internal(e: String) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, e)
}

pub fn router(store: SharedStore) -> Router {
    let router = Router::new()
        .route("/api/health", get(health))
        .route("/api/workers", get(list_workers))
        .route("/api/workers/register", post(register))
        .route("/api/workers/heartbeat", post(heartbeat))
        .route("/api/pipelines/trigger", post(trigger_pipeline))
        .route("/api/webhooks/forgejo", post(forgejo_webhook))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/claim", post(claim_job))
        .route("/api/jobs/{id}/report", post(report_job))
        .route("/api/runs", get(list_runs))
        .route("/api/runs/{id}", get(get_run))
        .route("/api/jobs/{id}/logs", get(job_logs))
        .route("/api/repos", get(list_repos).post(add_repo))
        .route("/api/activity/calendar", get(calendar))
        // dashboard dev server runs on another origin (127.0.0.1:4173)
        .layer(CorsLayer::permissive())
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

async fn list_workers(State(store): State<SharedStore>) -> Result<Json<Vec<Worker>>, ApiError> {
    Ok(Json(store.list_workers().await.map_err(internal)?))
}

async fn register(
    State(store): State<SharedStore>,
    Json(req): Json<WorkerRequest>,
) -> Result<(), ApiError> {
    store.register(&req.worker_name).await.map_err(internal)?;
    println!("Worker {} registered successfully!", req.worker_name);
    Ok(())
}

async fn heartbeat(
    State(store): State<SharedStore>,
    Json(req): Json<WorkerRequest>,
) -> Result<(), ApiError> {
    let known = store.heartbeat(&req.worker_name).await.map_err(internal)?;
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

/// Resolve which pipeline to run: the named repo's YAML from Forgejo; with
/// no name, the single registered repo, else this checkout's own
/// .orchestrator/actions.yml. No fake fallback plans.
async fn resolve_plan(
    store: &SharedStore,
    repo_name: Option<String>,
) -> Result<(Plan, String, String), ApiError> {
    // an unregistered name (e.g. retrying an old run) falls through
    let repo = match repo_name {
        Some(name) => store.get_repo(&name).await.map_err(internal)?,
        None => {
            let repos = store.list_repos().await.map_err(internal)?;
            // bare trigger with exactly one registered repo → run that repo
            if repos.len() == 1 { repos.into_iter().next() } else { None }
        }
    };

    let mut repo_label: Option<String> = None;
    if let Some(repo) = repo {
        let branch = repo.branch.clone();
        match plan_for_repo(&repo, &branch).await {
            Ok((plan, file)) => return Ok((plan, repo.name, file)),
            // not on Forgejo (yet) — fall through to this checkout's own
            // actions.yml, still grouping the run under the repo
            Err((_, reason)) => {
                println!("{reason}; trying the local .orchestrator/actions.yml");
                repo_label = Some(repo.name);
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
    let repo = repo_label.unwrap_or_else(|| plan.name.clone());
    Ok((plan, repo, ".orchestrator/actions.yml".into()))
}

async fn trigger_pipeline(
    State(store): State<SharedStore>,
    body: Option<Json<TriggerRequest>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let repo_name = body.and_then(|Json(b)| b.repo);
    let (plan, repo, file) = resolve_plan(&store, repo_name).await?;

    let id = store
        .create_run(&plan.name, &repo, &file, TriggerKind::Manual, None, &plan)
        .await
        .map_err(internal)?;
    println!("Triggered run {id}: pipeline '{}' from {file} ({} jobs)", plan.name, plan.jobs.len());
    Ok(Json(serde_json::json!({ "id": id })))
}

/// Forgejo push webhook. Point the repo's webhook at
/// POST http://<coordinator>/api/webhooks/forgejo (content type JSON).
async fn forgejo_webhook(
    State(store): State<SharedStore>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use serde_json::Value;

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

    let (plan, file) = plan_for_repo(&repo, &branch).await?;
    let id = store
        .create_run(&plan.name, &repo.name, &file, TriggerKind::Webhook, commit.as_ref(), &plan)
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
) -> Result<Json<Option<Job>>, ApiError> {
    Ok(Json(store.claim_job(&req.worker_name).await.map_err(internal)?))
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

async fn calendar(State(store): State<SharedStore>) -> Result<Json<Vec<CalendarDay>>, ApiError> {
    Ok(Json(store.calendar().await.map_err(internal)?))
}
