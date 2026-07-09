# Module 1: Single-Node Job Runner

A small service that runs shell commands as subprocesses and reports the result.

## What to build

1. An HTTP API that accepts a job (`command`, `env`, `timeout`)
2. A background runner that executes it as a subprocess
3. Storage so you can query the job's status, exit code, and logs later
4. Cancel + timeout handling

## Endpoints

- `POST /jobs` — submit a job, get an ID
- `GET /jobs/{id}` — check status
- `GET /jobs/{id}/logs` — fetch stdout/stderr
- `POST /jobs/{id}/cancel` — kill a running job
- `GET /jobs` — list recent jobs

## Tools

| Purpose | Tool |
|---|---|
| Language | Python 3.11+ |
| HTTP framework | FastAPI |
| Process execution | `subprocess` + `asyncio` (stdlib) |
| Storage | SQLite (via `sqlite3` or SQLAlchemy) |
| Tests | pytest + httpx |
| Packaging | Docker |

## Done when

- You can `curl POST /jobs` with a real command, poll its status, and fetch logs
- Timeouts kill the process
- Cancel works mid-run
- Jobs survive a service restart
- Tests cover: success, failure, timeout, cancel

## Don't build yet

Queues, multiple workers, git checkout, pipelines, auth, UI. Those are later modules.
