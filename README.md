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

### Fast deploys on a weak server

Compiling Rust on a small server is the slow part of self-deploy. Two
mechanisms deal with it:

1. **Docker layer caching** — the Dockerfile compiles dependencies against a
   dummy `main.rs` in their own layer, so a normal push only rebuilds the
   crate itself, not axum/sqlx/tokio.
2. **Offloaded compilation** — the `compile-release` job (`tags: [heavy,
   docker]`) runs on whatever beefy worker advertises those tags: it builds
   the linux release binary and the dashboard inside docker
   (`TARGET_PLATFORM` in actions.yml must match the server's arch) and
   uploads them as `prebuilt.tar.gz`. `self-deploy` unpacks that and builds
   the runtime-only `Dockerfile.prebuilt` instead — seconds instead of
   minutes. If the artifact is missing or empty it falls back to compiling
   locally, so deploys still work when no heavy worker was around (only
   slower).

To act as the build machine, a worker needs docker installed and tags:

    cargo run --release -- worker --name laptop --tags heavy,docker \
      --coordinator https://ci.example.com

## Workers on other machines

Any machine can join the pool and share CI load — it needs its own
executor plus a worker pointed at the coordinator:

    cd command-executor && uv sync && uv run uvicorn app.main:app --port 9000
    COORDINATOR_URL=https://ci.example.com cargo run --release -- worker --name macbook

Jobs claimed by that worker run as plain subprocesses on that machine, in a
workspace cloned from the pushed commit; artifacts still travel through the
coordinator, so `needs` works across machines.

### Job placement (pins and tags)

Machine-specific jobs (like `self-deploy`, which needs the server's docker
socket) pin themselves to one worker by name in actions.yml — either
`worker: <name>` on the job or the equivalent `env: WORKER_PIN: <name>`.
Pinned jobs are queued only for that worker while it's online.

Heavy jobs that need a *class* of machine rather than one specific box use
capability tags. Start capable workers with labels:

    cargo run --release -- worker --name beefy-1 --tags heavy,docker

(or `WORKER_TAGS=heavy,docker` in the environment), then mark the job:

    build-release:
      stage: build
      image: rust:latest
      tags: [heavy]
      script: cargo build --release

A tagged job only runs on an online worker carrying **all** of its tags
(idle workers are preferred); if none is online it waits in the queue until
one appears. Untagged jobs land in the global queue, first free worker wins.

## Dashboard login

Set both `DASHBOARD_USERNAME` and `DASHBOARD_PASSWORD` in the coordinator's
environment (see docker-compose.yml) and the dashboard shows a login screen;
sessions live in Redis for 7 days. Worker, executor, and webhook endpoints
are not affected — machines keep talking to the coordinator without a
password, so anyone who can reach the coordinator URL can still register a
worker and claim jobs.

## Pieces

| Directory           | What                                                      |
| ------------------- | --------------------------------------------------------- |
| `src/`              | coordinator + worker binary (axum, sqlx, redis)            |
| `yaml-parser/`      | pipeline schema/needs/cycle validation + execution planner |
| `command-executor/` | runs job commands as subprocesses, keeps logs              |
| `dashboard/`        | Svelte UI; full HTTP contract in dashboard/README.md       |
| `.orchestrator/`    | this repo's own CI pipeline (actions.yml)                  |
