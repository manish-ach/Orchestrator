# Command Executor

Single-node job runner (FastAPI + SQLite). The Rust worker hands each claimed
CI job to this service, which executes it as a subprocess and reports the
result. Full spec in [SPEC.md](SPEC.md).

## Run it

    cd command-executor
    uv sync                                  # one-time venv setup
    uv run uvicorn app.main:app --port 9000

Or containerized (from the repo root): `docker compose up -d executor`.

## Endpoints

| Endpoint                 | Notes                                              |
| ------------------------ | -------------------------------------------------- |
| `POST /run`              | synchronous: `{ command, timeout?, env? }` → `{ output, status, exit_code }` — used by the worker |
| `POST /jobs`             | async submit, returns `{ id }`                     |
| `GET  /jobs`             | recent jobs                                        |
| `GET  /jobs/{id}`        | status, exit code, timing                          |
| `GET  /jobs/{id}/logs`   | full stdout+stderr text                            |
| `POST /jobs/{id}/cancel` | kill a running job                                 |

Jobs persist in `jobs.db`; logs live in `storage/logs/<id>.log`. Timeouts
kill the process (status `timeout`); cancel marks it `cancelled`.
