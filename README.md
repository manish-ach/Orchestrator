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

## Self-deploy (the orchestrator ships itself)

With this enabled, a push to `main` rebuilds and redeploys the running
stack through its own pipeline — the `self-deploy` job in
`.orchestrator/actions.yml`. The job syncs the server's checkout to the
pushed commit, builds the images there (so the server's
`docker-compose.override.yml` and `.env` still apply), then hands the final
`docker compose up -d` to a small detached helper container. The swap
happens ~10s *after* the run finishes, so the pipeline never kills itself
mid-run; if the build fails, the run goes red and the old stack keeps
running.

Enable it on the server with a `docker-compose.override.yml` next to the
checkout (adjust `/home/ubuntu/Orchestrator` to your path), then
`docker compose up -d --build` once by hand so the executor gains the
docker CLI:

    services:
      executor:
        volumes:
          - /var/run/docker.sock:/var/run/docker.sock
          - /home/ubuntu/Orchestrator:/host/stack
        environment:
          HOST_STACK_DIR: /home/ubuntu/Orchestrator

Caveats:

- the checkout's `origin` must be fetchable from inside a container —
  a public https remote works; an ssh remote using the server user's key
  does not (`git remote set-url origin https://...` if needed)
- deploy runs `git reset --hard` to the pushed commit in that checkout;
  local commits there are discarded (untracked files like the override
  and `.env` survive)
- mounting the docker socket gives every pipeline job on this executor
  control of the host's docker — only register repos you trust
- without the override, `self-deploy` fails with a hint and the rest of
  the pipeline is unaffected

## Pieces

| Directory           | What                                                      |
| ------------------- | --------------------------------------------------------- |
| `src/`              | coordinator + worker binary (axum, sqlx, redis)            |
| `yaml-parser/`      | pipeline schema/needs/cycle validation + execution planner |
| `command-executor/` | runs job commands as subprocesses, keeps logs              |
| `dashboard/`        | Svelte UI; full HTTP contract in dashboard/README.md       |
| `.orchestrator/`    | this repo's own CI pipeline (actions.yml)                  |
