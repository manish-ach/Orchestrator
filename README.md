# CI/CD Orchestrator

Distributed CI/CD platform. A Rust **coordinator** plans pipelines and hands
out jobs; Rust **workers** claim them and run each command on the FastAPI
**command executor**. Pipelines are defined in `.orchestrator/actions.yml`
(validated by the Python **yaml-parser**) and can be triggered manually or
by a **Forgejo push webhook**. Runs, jobs, and repos persist in **Postgres**;
the worker registry and ready-job queue live in **Redis**. The Svelte
**dashboard** is served by the coordinator itself.

## Run everything (containers)

    docker compose up -d --build
    open http://localhost:8080          # dashboard + API, one origin

Scale workers: `docker compose up -d --scale worker=3`.

## Run natively (dev)

    docker compose up -d postgres redis            # just the stores
    cargo run -- coordinator --port 8080
    cd command-executor && uv run uvicorn app.main:app --port 9000
    cargo run -- worker --name rechek              # one per machine

Python components use [uv](https://docs.astral.sh/uv/): `uv sync` in
`yaml-parser/` or `command-executor/` sets up the venv from the lockfile.

Worker configuration: `COORDINATOR_URL` and `EXECUTOR_URL` env vars
(default `http://127.0.0.1:8080` / `http://127.0.0.1:9000`).

## Hooking up Forgejo

1. Register the repo in the dashboard (Repos → **+ Add repo**) or:

       curl -X POST localhost:8080/api/repos -H 'Content-Type: application/json' \
         -d '{"remote": "https://git.manishacharya.name.np/Manish/Orchestrator"}'

2. Give the repo a pipeline: commit `.orchestrator/actions.yml` (this repo's
   own is the reference).

3. In Forgejo: repo → Settings → Webhooks → Add webhook → Forgejo,
   target `http://<coordinator-host>:8080/api/webhooks/forgejo`,
   content type JSON, trigger on push.

Every push then creates a run (trigger `webhook`, with commit sha/message/
author) that the dashboard picks up on its next poll.

## Pieces

| Directory           | What                                                      |
| ------------------- | --------------------------------------------------------- |
| `src/`              | coordinator + worker binary (axum, sqlx, redis)            |
| `yaml-parser/`      | pipeline schema/needs/cycle validation + execution planner |
| `command-executor/` | runs job commands as subprocesses, keeps logs              |
| `dashboard/`        | Svelte UI; full HTTP contract in dashboard/README.md       |
| `.orchestrator/`    | this repo's own CI pipeline (actions.yml)                  |
