[i lost my hosted server for personal git instance, so now this is the original🥲]

# CI/CD Orchestrator


Distributed CI/CD platform. A Rust **coordinator** plans pipelines and hands
out jobs; Rust **workers** claim them and run each command on the FastAPI
**command executor**. Pipelines are defined in `.orchestrator/actions.yml`
(validated by the Python **yaml-parser**) and can be triggered manually, by a
**Forgejo push webhook**, or on a **cron schedule**. Runs, jobs, and repos
persist in **Postgres**; the worker registry and ready-job queue live in
**Redis**. The Svelte **dashboard** is served by the coordinator itself.

The point of the design is that work is distributed: any machine that can reach
the coordinator can join the pool and take jobs, so a weak server can host the
platform while a laptop does the compiling.

## How it works

### The path a run takes

1. **Trigger.** A Forgejo push webhook, `POST /api/pipelines/trigger`, or a
   pipeline whose file declares a `schedule:`. The webhook carries the branch
   and commit; the coordinator records both on the run.

2. **Plan.** The coordinator fetches `.orchestrator/actions.yml` from the repo
   at that branch and hands it to the yaml-parser, which validates the stages
   and `needs:` edges — rejecting dependency cycles and unknown references —
   and returns an ordered plan. The run and one row per job go into Postgres.

3. **Queue.** A job becomes *ready* when every job it `needs` has passed. The
   coordinator then decides where it may run, in this order: a hard
   `WORKER_PIN`, then `tags:`, then a preference for whichever worker produced
   most of its dependencies (its files are already warm there). The job id goes
   onto a Redis list — the global queue, or that one worker's.

4. **Claim.** Workers poll `POST /api/jobs/claim` once a second, taking from
   their own queue before the global one. Claiming stamps `started_at` and
   marks the worker busy.

5. **Execute.** The worker forwards the command to its command executor, which
   clones the repo at the commit into a per-run workspace, downloads artifacts
   from the jobs this one `needs`, and runs the command as a subprocess through
   `sh -x`.

6. **Stream.** The executor posts its accumulated log back every couple of
   seconds. When the executor is local to the worker it posts *to the worker*,
   which forwards the same bytes on — so the worker's terminal UI and the
   website can never disagree about what a job printed. The dashboard tails it
   over server-sent events, so output appears while a job runs rather than once
   it has finished.

7. **Report.** On exit the worker posts the status, exit code and full output.
   The coordinator distils the first error line (so run lists can explain a
   failure without shipping whole build logs), stores declared artifacts, and
   re-evaluates which jobs are now ready — which starts the next stage.

### What lives where

**Postgres** is the source of truth: runs, jobs and their captured logs,
registered repos, webhook delivery records, and schedule state. It is what
survives a restart.

**Redis** holds only live or ephemeral state: the worker registry and its
heartbeats, the ready-job queues, a rolling ~15 minutes of CPU/RAM samples per
worker, and dashboard login sessions. It can be flushed without losing history —
the coordinator rebuilds the queues from Postgres on boot.

**Artifacts** are files, uploaded to the coordinator when a job passes and
downloaded by jobs that `needs` them. That indirection is what lets `needs`
work across machines.

### When a worker dies

Workers heartbeat every 2 seconds. Silence for more than:

- **5s** — shown offline in the dashboard, and no new jobs are placed there
- **30s** — the reconciler steals its running jobs back and requeues them, so a
  closed laptop does not strand a run. Each requeue is counted on the job
- **24h** — dropped from the registry, so decommissioned machines do not
  clutter the fleet view

A job pinned or tagged to a machine that is offline is not failed — it waits in
the queue and runs when a matching worker appears.

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

The dashboard also runs against a simulated coordinator with no backend at all,
which is how the UI is developed. See `dashboard/README.md` — it carries the
full HTTP contract between the dashboard and the coordinator.

## Hooking up Forgejo

1. Register the repo in the dashboard (Repos → **+ Add repo**) or:

       curl -X POST localhost:8080/api/repos -H 'Content-Type: application/json' \
         -d '{"remote": "https://git.manishacharya.name.np/Manish/Orchestrator"}'

   On registration the coordinator reads the repo's pipeline files and indexes
   their shape, so a pipeline shows its stages before it has ever run — and one
   whose YAML does not validate says so instead of appearing empty.

2. Give the repo a pipeline: commit `.orchestrator/actions.yml` (this repo's
   own is the reference).

3. In Forgejo: repo → Settings → Webhooks → Add webhook → Forgejo,
   target `http://<coordinator-host>:8080/api/webhooks/forgejo`,
   content type JSON, trigger on push.

Every push then creates a run. The coordinator records whether the webhook
actually reached it, which is what tells "nobody has pushed" apart from "the
webhook is misconfigured" — otherwise the two look identical.

## Self-deploy (the orchestrator ships itself)

A push to `main` rebuilds and redeploys the running stack through its own
pipeline — the `self-deploy` job in `.orchestrator/actions.yml`. The job syncs
the server's checkout to the pushed commit, builds the images there (so the
server's `docker-compose.override.yml` and `.env` still apply), then hands the
final `docker compose up -d` to a small detached helper container. The swap
happens ~10s *after* the run finishes, so the pipeline never kills itself
mid-run; if the build fails, the run goes red and the old stack keeps running.

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
- before swapping, the new image must pass `orchestrator --help`, so a binary
  built for the wrong architecture cannot take the stack down. That proves the
  binary runs, not that the coordinator starts — watch the logs after a deploy
  that changed database migrations

### Fast deploys on a weak server

Compiling Rust on a small server is the slow part. Two mechanisms deal with it:

1. **Docker layer caching.** The Dockerfile compiles dependencies against a
   dummy `main.rs` in their own layer, keyed on `Cargo.toml`/`Cargo.lock`.
   Changing `src/` rebuilds only the crate; changing neither — a dashboard-only
   push — skips Rust entirely. Only touching `Cargo.toml` or `Cargo.lock` costs
   a full dependency rebuild. Do not `docker system prune -a` on a build
   machine: that discards the cache this depends on.

2. **Offloaded compilation.** The `compile-release` job (`tags: [heavy,
   docker]`) runs on whatever worker advertises those tags: it builds the linux
   release binary and the dashboard inside docker (`TARGET_PLATFORM` in
   actions.yml must match the server's arch) and uploads them as
   `prebuilt.tar.gz`. `self-deploy` unpacks that and builds the runtime-only
   `Dockerfile.prebuilt` instead — seconds, rather than the better part of an
   hour on a single-core box. The building machine needs no checkout of its
   own; the workspace is cloned for it.

Because `compile-release` is tagged, **a push only deploys while a worker
carrying `heavy,docker` is online.** It does not fail without one — the job
waits in the queue and runs when such a worker appears, so "push now, start the
laptop worker after" works.

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

On first registration a worker reports what machine it is — OS, kernel, CPU,
memory, disk, shell — which the dashboard shows on that worker's page. It is
sampled once, since none of it changes while the process lives.

### Worker output

A worker on a terminal gets the full-screen dashboard; anywhere else it prints
plain timestamped lines. That decision is made from whether stdout is a tty, so
a systemd unit or container needs no extra flag:

    cargo run --release -- worker --name macbook            # dashboard on a laptop
    cargo run --release -- worker --name srv-1 --no-tui     # force plain lines
    cargo run --release -- worker --name srv-1 --tui        # force the dashboard

`--no-tui` matters when a wrapper still gives you a tty but you want the log
form — the server's own worker, for instance, alongside the coordinator.

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

## Scheduled pipelines

Give a pipeline file a top-level `schedule:` and the coordinator runs it on that
cron, no push required:

    name: nightly
    schedule: "0 2 * * *"    # min hour day-of-month month day-of-week
    jobs:
      ...

Five standard fields, with `*`, `n`, `a-b`, `a,b,c`, `*/n` and `a-b/n`. When
both day fields are restricted they are ORed, as every crontab does:
`0 0 13 * 5` means "the 13th, and every Friday", not "Friday the 13th".

The scheduler ticks on the minute and claims each firing in Postgres before
running it, so restarting the coordinator mid-minute cannot double-fire and two
coordinators against one database cannot both win. A schedule that does not
parse is reported in the log rather than silently never running. Scheduled runs
build the repo's default branch and carry no commit, since a schedule fires
against whatever is on the branch.

## Dashboard login

Set both `DASHBOARD_USERNAME` and `DASHBOARD_PASSWORD` in the coordinator's
environment — docker-compose reads them from the server's `.env`, so the
password is not committed — and the dashboard shows a login screen. Sessions
live in Redis for 7 days.

Worker, executor, and webhook endpoints are unaffected: machines keep talking to
the coordinator without a password, so anyone who can reach the coordinator URL
can still register a worker and claim jobs.

## Starting fresh

`scripts/reset.sh` wipes run history so the dashboard starts clean — runs, jobs,
their captured logs, artifacts, the worker registry, webhook delivery records
and schedule state.

    ./scripts/reset.sh              # keeps registered repositories
    ./scripts/reset.sh --all        # also unregisters every repository

Run it from anywhere; it locates the stack from its own path and uses `sudo` for
docker if it has to. It asks for confirmation (type `reset`) unless given
`--yes`. Run ids restart at 1 afterwards. The server's own worker is restarted
for you; **remote workers need a manual restart**, because each caches its id on
disk and would otherwise heartbeat with an id the registry no longer knows.

Deliberately **not** part of self-deploy: a push that silently deleted history
would make every deploy destructive, and you would only find out the first time
you wanted to look something up.

## Pieces

| Directory           | What                                                      |
| ------------------- | --------------------------------------------------------- |
| `src/`              | coordinator + worker binary (axum, sqlx, redis)            |
| `yaml-parser/`      | pipeline schema/needs/cycle validation + execution planner |
| `command-executor/` | runs job commands as subprocesses, keeps logs              |
| `dashboard/`        | Svelte UI; full HTTP contract in dashboard/README.md       |
| `.orchestrator/`    | this repo's own CI pipeline (actions.yml)                  |
| `scripts/`          | `reset.sh` (wipe history), `demo-pipeline.sh`              |

`DESIGN.md` covers the dashboard: what each page is for, the colour and type
system, and the rules it follows about never showing a number it cannot back up.
