# app/api/routes/jobs.py
#
# Async job API per SPEC.md: submit, inspect, fetch logs, cancel.

import asyncio

from fastapi import APIRouter, HTTPException
from fastapi.responses import PlainTextResponse
from pydantic import BaseModel, Field

from app import runner
from app.config import settings

router = APIRouter()


class JobSubmit(BaseModel):
    command: str
    env: dict[str, str] = Field(default_factory=dict)
    timeout: int = settings.DEFAULT_TIMEOUT


@router.post("")
async def submit_job(req: JobSubmit):
    job_id = runner.create_job(req.command, env=req.env, timeout=req.timeout)
    asyncio.get_running_loop().create_task(runner.run_job(job_id))
    return {"id": job_id}


@router.get("")
async def list_jobs(limit: int = 50):
    return runner.list_jobs(limit)


@router.get("/{job_id}")
async def get_job(job_id: str):
    job = runner.get_job(job_id)
    if job is None:
        raise HTTPException(status_code=404, detail="job not found")
    return job


@router.get("/{job_id}/logs", response_class=PlainTextResponse)
async def get_logs(job_id: str):
    logs = runner.read_logs(job_id)
    if logs is None:
        raise HTTPException(status_code=404, detail="job not found")
    return logs


@router.post("/{job_id}/cancel")
async def cancel_job(job_id: str):
    if runner.get_job(job_id) is None:
        raise HTTPException(status_code=404, detail="job not found")
    if not runner.cancel_job(job_id):
        raise HTTPException(status_code=409, detail="job is not running")
    return {"cancelled": job_id}
