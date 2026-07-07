import asyncio
import json
import os
import signal
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from uuid import uuid4

from fastapi import APIRouter, HTTPException
from pydantic import BaseModel, Field

from app.config import settings
from app.db.session import get_db


router = APIRouter()


TERMINAL_STATUSES = {"passed", "failed", "timeout", "cancelled"}


class JobCreate(BaseModel):
    command: str = Field(..., min_length=1)
    env: dict[str, str] = Field(default_factory=dict)
    timeout: int | None = None


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def row_to_dict(row: Any) -> dict[str, Any] | None:
    if row is None:
        return None
    return dict(row)


def get_job_or_404(job_id: str) -> dict[str, Any]:
    with get_db() as conn:
        row = conn.execute("SELECT * FROM jobs WHERE id = ?", (job_id,)).fetchone()

    job = row_to_dict(row)
    if job is None:
        raise HTTPException(status_code=404, detail="Job not found")
    return job


def kill_process_group(pid: int) -> None:
    try:
        os.killpg(pid, signal.SIGTERM)
    except ProcessLookupError:
        return


def make_log_paths(job_id: str) -> tuple[str, str]:
    logs_dir = Path(settings.LOGS_DIR)
    logs_dir.mkdir(parents=True, exist_ok=True)
    return str(logs_dir / f"{job_id}.stdout.log"), str(logs_dir / f"{job_id}.stderr.log")


async def stream_to_file(stream: asyncio.StreamReader | None, path: str) -> None:
    if stream is None:
        return

    with open(path, "wb") as file:
        while True:
            chunk = await stream.read(8192)
            if not chunk:
                break
            file.write(chunk)
            file.flush()


async def run_job(job_id: str, command: str, env: dict[str, str], timeout: int) -> None:
    stdout_path, stderr_path = make_log_paths(job_id)
    process_env = os.environ.copy()
    process_env.update(env)

    with get_db() as conn:
        row = conn.execute("SELECT status FROM jobs WHERE id = ?", (job_id,)).fetchone()
        if row is None or row["status"] != "pending":
            return

    try:
        process = await asyncio.create_subprocess_shell(
            command,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=process_env,
            start_new_session=True,
        )
    except Exception:
        finished_at = utc_now()
        with get_db() as conn:
            conn.execute(
                """
                UPDATE jobs
                SET status = ?, exit_code = ?, stdout_path = ?, stderr_path = ?,
                    started_at = COALESCE(started_at, ?), finished_at = ?
                WHERE id = ? AND status IN ('pending', 'running')
                """,
                ("failed", 127, stdout_path, stderr_path, finished_at, finished_at, job_id),
            )
        return

    started_at = utc_now()
    with get_db() as conn:
        conn.execute(
            """
            UPDATE jobs
            SET status = ?, pid = ?, stdout_path = ?, stderr_path = ?, started_at = ?
            WHERE id = ? AND status = 'pending'
            """,
            ("running", process.pid, stdout_path, stderr_path, started_at, job_id),
        )
        should_continue = conn.total_changes > 0

    if not should_continue:
        kill_process_group(process.pid)
        await process.wait()
        return

    stdout_task = asyncio.create_task(stream_to_file(process.stdout, stdout_path))
    stderr_task = asyncio.create_task(stream_to_file(process.stderr, stderr_path))

    timed_out = False
    try:
        await asyncio.wait_for(process.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        timed_out = True
        kill_process_group(process.pid)
        await process.wait()
    finally:
        await asyncio.gather(stdout_task, stderr_task, return_exceptions=True)

    finished_at = utc_now()
    with get_db() as conn:
        row = conn.execute("SELECT status FROM jobs WHERE id = ?", (job_id,)).fetchone()
        current_status = row["status"] if row else None

        if current_status == "cancelled":
            return

        if timed_out:
            status = "timeout"
        else:
            status = "passed" if process.returncode == 0 else "failed"

        conn.execute(
            """
            UPDATE jobs
            SET status = ?, exit_code = ?, finished_at = ?
            WHERE id = ?
            """,
            (status, process.returncode, finished_at, job_id),
        )


@router.post("")
async def create_job(job: JobCreate):
    timeout = job.timeout if job.timeout is not None else settings.DEFAULT_TIMEOUT
    if timeout > settings.MAX_TIMEOUT:
        raise HTTPException(status_code=400, detail=f"timeout must be <= {settings.MAX_TIMEOUT}")
    if timeout <= 0:
        raise HTTPException(status_code=400, detail="timeout must be greater than 0")

    job_id = str(uuid4())
    created_at = utc_now()

    with get_db() as conn:
        conn.execute(
            """
            INSERT INTO jobs (id, command, env, timeout, status, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            """,
            (job_id, job.command, json.dumps(job.env), timeout, "pending", created_at),
        )

    asyncio.create_task(run_job(job_id, job.command, job.env, timeout))
    return {"id": job_id, "status": "pending"}


@router.get("")
async def list_jobs():
    with get_db() as conn:
        rows = conn.execute(
            """
            SELECT id, command, status, exit_code, created_at, started_at, finished_at
            FROM jobs
            ORDER BY created_at DESC
            LIMIT 100
            """
        ).fetchall()

    return [dict(row) for row in rows]


@router.get("/{job_id}")
async def get_job(job_id: str):
    job = get_job_or_404(job_id)
    return {
        "id": job["id"],
        "command": job["command"],
        "status": job["status"],
        "exit_code": job["exit_code"],
        "created_at": job["created_at"],
        "started_at": job["started_at"],
        "finished_at": job["finished_at"],
    }


@router.get("/{job_id}/logs")
async def get_job_logs(job_id: str):
    job = get_job_or_404(job_id)

    def read_text(path: str | None) -> str:
        if not path:
            return ""
        log_path = Path(path)
        if not log_path.exists():
            return ""
        return log_path.read_text(errors="replace")

    return {
        "stdout": read_text(job["stdout_path"]),
        "stderr": read_text(job["stderr_path"]),
    }


@router.post("/{job_id}/cancel")
async def cancel_job(job_id: str):
    job = get_job_or_404(job_id)
    if job["status"] in TERMINAL_STATUSES:
        raise HTTPException(status_code=409, detail="Job already finished")

    if job["pid"] is not None:
        kill_process_group(int(job["pid"]))

    finished_at = utc_now()
    with get_db() as conn:
        conn.execute(
            """
            UPDATE jobs
            SET status = ?, finished_at = ?
            WHERE id = ? AND status NOT IN ('passed', 'failed', 'timeout', 'cancelled')
            """,
            ("cancelled", finished_at, job_id),
        )

    return {"id": job_id, "status": "cancelled"}
