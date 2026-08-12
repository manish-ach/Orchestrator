# app/runner.py
#
# Executes shell commands as subprocesses and records everything in SQLite.
# Log output goes to LOGS_DIR/<job_id>.log so it survives restarts.

import asyncio
import io
import json
import os
import shutil
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


async def _matches(ws: Path, repo_url: str, commit_sha: str | None, branch: str | None) -> bool:
    """Is this existing directory already the checkout the job asked for?

    A workspace is reused by every later job of the same run, so it has to be
    proved right rather than assumed: a stale directory left by an earlier run
    that happened to take the same id belongs to a different repo entirely, and
    handing it back silently is what makes a job fail as if its own code were
    broken."""
    code, origin = await _sh("git remote get-url origin", cwd=str(ws))
    if code != 0 or origin.strip() != repo_url:
        return False
    if commit_sha:
        code, head = await _sh("git rev-parse HEAD", cwd=str(ws))
        return code == 0 and head.strip() == commit_sha
    if branch:
        code, at = await _sh("git rev-parse --abbrev-ref HEAD", cwd=str(ws))
        return code == 0 and at.strip() == branch
    return True


async def prepare_workspace(
    name: str, repo_url: str | None, commit_sha: str | None, branch: str | None = None
) -> Path:
    """Materialize a per-run workspace on THIS machine: clone once, reuse
    for every later job of the run that lands here.

    Never returns a directory that is not the requested checkout. Without a
    repo to clone there is nothing to run against, so that is an error and not
    an empty directory — a job that runs in an empty workspace fails somewhere
    far downstream, wearing the mask of a bug in the code under test."""
    if not repo_url:
        raise WorkspaceError(
            "no REPO_URL for this run — register the repo with a remote in the "
            "dashboard, or the job would run against an empty workspace"
        )

    ws = Path(settings.WORKSPACES_DIR) / name
    async with _WS_LOCKS[name]:
        if ws.exists() and not await _matches(ws, repo_url, commit_sha, branch):
            # only ever inside WORKSPACES_DIR, and only a directory already
            # proved useless — the run's real output lives on the coordinator
            if ws.resolve().parent != Path(settings.WORKSPACES_DIR).resolve():
                raise WorkspaceError(f"refusing to replace '{ws}': outside the workspaces directory")
            shutil.rmtree(ws)

        if not ws.exists():
            code, out = await _sh(f'git clone "{repo_url}" "{ws}"')
            if code != 0:
                raise WorkspaceError(f"workspace clone failed: {out.strip()}")
            # a sha when the trigger carried one, else the pushed branch: a
            # clone lands on the remote's default branch, so skipping this
            # tests main while REPO_BRANCH claims otherwise
            target = commit_sha or branch
            if target:
                code, out = await _sh(f'git checkout -q "{target}"', cwd=str(ws))
                if code != 0:
                    raise WorkspaceError(f"checkout of '{target}' failed: {out.strip()}")
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
        # errors=replace: a live tail can end mid multi-byte character
        return Path(job["stdout_path"]).read_text(errors="replace")
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
    """Execute a pending job to completion; returns the final status.

    Output streams to the log file AS IT ARRIVES, so read_logs() (and the
    coordinator's progress polling) can tail a running job live instead of
    waiting for it to finish."""
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

    log_path = Path(job["stdout_path"])
    log_path.write_bytes(b"")  # exists (and empty) from the first moment

    async def pump() -> None:
        with log_path.open("ab") as f:
            while chunk := await proc.stdout.read(4096):
                f.write(chunk)
                f.flush()

    pump_task = asyncio.create_task(pump())
    try:
        await asyncio.wait_for(proc.wait(), timeout=job["timeout"])
        status = "passed" if proc.returncode == 0 else "failed"
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()
        status = "timeout"
    finally:
        # the pipe closes once the process is dead, so this always ends
        await pump_task
        RUNNING.pop(job_id, None)

    if job_id in CANCELLED:
        CANCELLED.discard(job_id)
        status = "cancelled"

    with get_db() as conn:
        conn.execute(
            "UPDATE jobs SET status = ?, exit_code = ?, finished_at = ? WHERE id = ?",
            (status, proc.returncode, _now_ms(), job_id),
        )
    return status
