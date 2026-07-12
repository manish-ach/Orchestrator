# app/runner.py
#
# Executes shell commands as subprocesses and records everything in SQLite.
# Log output goes to LOGS_DIR/<job_id>.log so it survives restarts.

import asyncio
import io
import json
import os
import tarfile
import time
import uuid
from collections import defaultdict
from pathlib import Path

import httpx

from app.config import settings
from app.db.session import get_db

# job_id -> live process, so /cancel can kill it
RUNNING: dict[str, asyncio.subprocess.Process] = {}
# job_ids killed via /cancel, so run_job reports "cancelled" not "failed"
CANCELLED: set[str] = set()
# one lock per workspace so parallel jobs of a run don't race the clone
_WS_LOCKS: dict[str, asyncio.Lock] = defaultdict(asyncio.Lock)


class WorkspaceError(RuntimeError):
    """Raised when a workspace cannot be prepared or artifacts moved."""


async def _sh(command: str, cwd: str | None = None) -> tuple[int, str]:
    proc = await asyncio.create_subprocess_shell(
        command, cwd=cwd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT
    )
    out, _ = await proc.communicate()
    return proc.returncode or 0, out.decode(errors="replace")


async def prepare_workspace(name: str, repo_url: str | None, commit_sha: str | None) -> Path:
    """Materialize a per-run workspace on THIS machine: clone once, reuse
    for every later job of the run that lands here."""
    ws = Path(settings.WORKSPACES_DIR) / name
    async with _WS_LOCKS[name]:
        if not ws.exists():
            if not repo_url:
                ws.mkdir(parents=True, exist_ok=True)
                return ws
            code, out = await _sh(f'git clone "{repo_url}" "{ws}"')
            if code != 0:
                raise WorkspaceError(f"workspace clone failed: {out.strip()}")
            if commit_sha:
                code, out = await _sh(f'git checkout -q "{commit_sha}"', cwd=str(ws))
                if code != 0:
                    raise WorkspaceError(f"checkout of {commit_sha[:7]} failed: {out.strip()}")
    return ws


async def fetch_artifacts(url: str, ws: Path) -> None:
    """Download a dependency's artifact bundle from the coordinator and
    unpack it into the workspace — how files cross machine boundaries."""
    async with httpx.AsyncClient(timeout=120) as client:
        resp = await client.get(url)
        if resp.status_code != 200:
            raise WorkspaceError(f"artifact download failed ({resp.status_code}) from {url}")
        with tarfile.open(fileobj=io.BytesIO(resp.content), mode="r:gz") as tar:
            tar.extractall(ws)


async def upload_artifacts(ws: Path, paths: list[str], url: str) -> int:
    """Bundle declared output paths and push them to the coordinator."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for p in paths:
            full = ws / p
            if not full.exists():
                raise WorkspaceError(f"declared artifact '{p}' was not produced by the job")
            tar.add(full, arcname=p)
    data = buf.getvalue()
    async with httpx.AsyncClient(timeout=120) as client:
        resp = await client.post(url, content=data)
        if resp.status_code >= 300:
            raise WorkspaceError(f"artifact upload failed ({resp.status_code})")
    return len(data)


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
