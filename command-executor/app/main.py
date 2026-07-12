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


@app.post("/run", tags=["Sync"])
async def run_sync(req: RunRequest):
    """Synchronous execution used by the Rust worker: runs the command to
    completion and returns output + status + exit code in one response.
    The job is still recorded in SQLite like any /jobs submission."""
    job_id = runner.create_job(req.command, env=req.env, timeout=req.timeout)
    status = await runner.run_job(job_id)
    job = runner.get_job(job_id)
    return {
        "id": job_id,
        "output": runner.read_logs(job_id) or "",
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
