# app/runner.py
#
# Executes shell commands as subprocesses and records everything in SQLite.
# Log output goes to LOGS_DIR/<job_id>.log so it survives restarts.

import asyncio
import json
import os
import time
import uuid
from pathlib import Path

from app.config import settings
from app.db.session import get_db

# job_id -> live process, so /cancel can kill it
RUNNING: dict[str, asyncio.subprocess.Process] = {}
# job_ids killed via /cancel, so run_job reports "cancelled" not "failed"
CANCELLED: set[str] = set()


def _now_ms() -> int:
    return int(time.time() * 1000)


def create_job(command: str, env: dict | None = None, timeout: int | None = None) -> str:
    job_id = uuid.uuid4().hex[:12]
    timeout = min(timeout or settings.DEFAULT_TIMEOUT, settings.MAX_TIMEOUT)
    stdout_path = str(Path(settings.LOGS_DIR) / f"{job_id}.log")

    with get_db() as conn:
        conn.execute(
            """INSERT INTO jobs (id, command, env, timeout, status, stdout_path)
               VALUES (?, ?, ?, ?, 'pending', ?)""",
            (job_id, command, json.dumps(env or {}), timeout, stdout_path),
        )
    return job_id


def get_job(job_id: str) -> dict | None:
    with get_db() as conn:
        row = conn.execute("SELECT * FROM jobs WHERE id = ?", (job_id,)).fetchone()
    return dict(row) if row else None


def list_jobs(limit: int = 50) -> list[dict]:
    with get_db() as conn:
        rows = conn.execute(
            "SELECT * FROM jobs ORDER BY created_at DESC LIMIT ?", (limit,)
        ).fetchall()
    return [dict(r) for r in rows]


def read_logs(job_id: str) -> str | None:
    job = get_job(job_id)
    if not job or not job["stdout_path"]:
        return None
    try:
        return Path(job["stdout_path"]).read_text()
    except FileNotFoundError:
        return ""


def cancel_job(job_id: str) -> bool:
    proc = RUNNING.get(job_id)
    if proc is None:
        return False
    CANCELLED.add(job_id)
    proc.kill()
    return True


async def run_job(job_id: str) -> str:
    """Execute a pending job to completion; returns the final status."""
    job = get_job(job_id)
    if job is None:
        return "failed"

    env = {**os.environ, **json.loads(job["env"] or "{}")}
    proc = await asyncio.create_subprocess_shell(
        job["command"],
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
        env=env,
    )
    RUNNING[job_id] = proc
    with get_db() as conn:
        conn.execute(
            "UPDATE jobs SET status = 'running', pid = ?, started_at = ? WHERE id = ?",
            (proc.pid, _now_ms(), job_id),
        )

    output = b""
    try:
        output, _ = await asyncio.wait_for(proc.communicate(), timeout=job["timeout"])
        status = "passed" if proc.returncode == 0 else "failed"
    except asyncio.TimeoutError:
        proc.kill()
        output, _ = await proc.communicate()
        status = "timeout"
    finally:
        RUNNING.pop(job_id, None)

    if job_id in CANCELLED:
        CANCELLED.discard(job_id)
        status = "cancelled"

    Path(job["stdout_path"]).write_bytes(output or b"")
    with get_db() as conn:
        conn.execute(
            "UPDATE jobs SET status = ?, exit_code = ?, finished_at = ? WHERE id = ?",
            (status, proc.returncode, _now_ms(), job_id),
        )
    return status
