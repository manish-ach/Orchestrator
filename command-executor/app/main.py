# app/main.py

from contextlib import asynccontextmanager

from fastapi import FastAPI
from pydantic import BaseModel, Field

from app import runner
from app.api.routes.jobs import router as jobs_router
from app.config import settings
from app.db.session import init_db


@asynccontextmanager
async def lifespan(app: FastAPI):
    # Initialize database on startup
    init_db()

    # Future: recover running jobs after restart
    yield

    # Cleanup logic can go here


app = FastAPI(
    title=settings.APP_NAME,
    version=settings.APP_VERSION,
    lifespan=lifespan,
)


@app.get("/", tags=["Health"])
async def health_check():
    return {
        "status": "ok",
        "service": "command-executor",
    }


@app.get("/health", tags=["Health"])
async def health():
    return {"healthy": True}


class RunRequest(BaseModel):
    command: str
    timeout: int = settings.DEFAULT_TIMEOUT
    env: dict[str, str] = Field(default_factory=dict)
    # per-run workspace management + artifact transfer (all optional so a
    # bare {command} still works exactly as before)
    workspace: str | None = None
    repo_url: str | None = None
    commit_sha: str | None = None
    inputs: list[str] = Field(default_factory=list)
    outputs: list[str] = Field(default_factory=list)
    upload_url: str | None = None


@app.post("/run", tags=["Sync"])
async def run_sync(req: RunRequest):
    """Synchronous execution used by the Rust worker: prepare the run's
    workspace (clone once per machine), pull dependency artifacts from the
    coordinator, run the command inside the workspace, push declared
    outputs back. The job is still recorded in SQLite like any /jobs
    submission."""
    ws = None
    command = req.command
    if req.workspace:
        try:
            ws = await runner.prepare_workspace(req.workspace, req.repo_url, req.commit_sha)
            for url in req.inputs:
                await runner.fetch_artifacts(url, ws)
        except runner.WorkspaceError as e:
            return {"id": None, "output": str(e), "status": "failed", "exit_code": None}
        command = f'cd "{ws}" && ({req.command})'

    job_id = runner.create_job(command, env=req.env, timeout=req.timeout)
    status = await runner.run_job(job_id)
    job = runner.get_job(job_id)
    output = runner.read_logs(job_id) or ""

    if status == "passed" and ws and req.outputs and req.upload_url:
        try:
            size = await runner.upload_artifacts(ws, req.outputs, req.upload_url)
            output += f"\n[executor] uploaded artifacts: {', '.join(req.outputs)} ({size} bytes)"
        except runner.WorkspaceError as e:
            status = "failed"
            output += f"\n[executor] {e}"

    return {
        "id": job_id,
        "output": output,
        "status": "passed" if status == "passed" else "failed",
        "exit_code": job["exit_code"] if job else None,
    }


# Register routes
app.include_router(
    jobs_router,
    prefix="/jobs",
    tags=["Jobs"],
)
#Nimesh Giri
