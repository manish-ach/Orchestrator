# Dashboard

Web UI for the CI/CD orchestrator. **Svelte 5 + TypeScript + Vite**, built as a
plain static bundle (hash routing — no server-side rendering, no URL-rewrite
config needed on the host).

## Run it

    cd dashboard
    npm install
    npm run dev        # dev server on http://127.0.0.1:4173
    npm run build      # production bundle in dist/ (~53 KB gzipped total)
    npm run check      # svelte-check + TypeScript (strict)

Deploying: serve `dist/` from any static file server. The intended setup is
the coordinator itself serving it (axum + `tower_http::services::ServeDir`),
which also makes CORS a non-issue because the UI and the API share an origin.

## Pages

| Route | What it shows |
| --- | --- |
| `#/` | Control center: KPI tiles, live pipeline DAG, worker cluster, recent runs, fleet chart |
| `#/runs` | Run history: status chips, repo/trigger/when facets, search, day grouping |
| `#/run/<id>` | One run: stage tree + pipeline file, DAG canvas, workers, live streaming logs |
| `#/run/<id>?job=<id>` | Deep link straight into one job's log |
| `#/repos` | Registered repositories |
| `#/repos/<repo>` | That repo's pipelines |
| `#/repos/<repo>/<pipeline>` | That pipeline's own run history, with duration-vs-median bars |
| `#/workers` | The fleet as a list — live state per machine |
| `#/workers/<name>` | One machine: device profile, activity calendar, CPU/RAM, what it runs |
| `#/insights` | Aggregates: stage cost, flaky jobs, duration trend, queue wait, worker skew, failure causes |

`#/monitor` and `#/repo/<name>` redirect to their replacements, so older links
still resolve. Anything else renders a styled 404 naming the route that failed.

## Data modes

All data access goes through **`src/lib/api.ts`** — the only file that knows
about HTTP. It exposes:

- `ENDPOINTS` — every coordinator URL in one registry (see the contract below)
- `api` — the active data source, one of:
  - **live** (default): the Rust coordinator's REST API.
  - **mock**: `src/lib/mock.ts`, a simulated coordinator with believable
    history and a live-advancing run. This is what the UI was designed
    against; the demo works with no backend at all.

Switch with `?mode=live` / `?mode=mock` on any URL (persists in
localStorage). Change the coordinator address with
`localStorage.setItem('dash.apiBase', 'http://vm:8080')`.

Types for every payload live in `src/lib/types.ts` and are the
machine-checked version of the contract below.

---

## The coordinator contract

**The dashboard never talks to the database.** PostgreSQL (and Redis) are
internal details of the coordinator. The UI speaks only HTTP+JSON to the
coordinator's REST API, for three reasons:

1. **One contract.** Workers, curl, and the dashboard all use the same API;
   the DB schema can change freely without breaking the UI.
2. **Security.** The DB never needs to be reachable from browsers, and the
   coordinator can enforce whatever auth it grows later in one place.
3. **Mockability.** Because the boundary is HTTP, the whole UI runs against
   `mock.ts` — which is how it is demoed and developed.

So: everything below is data the **coordinator must serve** over HTTP. Where
the coordinator gets it (Postgres, memory, the Forgejo API) is its business.

### Implemented today

| Endpoint                      | Notes                                     |
| ----------------------------- | ----------------------------------------- |
| `GET  /api/health`            | `{ "health": "Ok", "online_workers": 5 }` |
| `GET  /api/auth/status`       | `{ "required": true }` when the coordinator has `DASHBOARD_USERNAME`/`DASHBOARD_PASSWORD` set |
| `POST /api/auth/login`        | `{ "username", "password" }` → `{ "token" }`; the dashboard sends it as `Authorization: Bearer <token>` on every call below (workers/webhooks stay tokenless) |
| `GET  /api/workers`           | see Worker below                          |
| `GET  /api/workers/stats`     | rolling CPU/RAM sample history per worker (shape below) |
| `GET  /api/workers/{name}/activity` | one machine's job history: year calendar, totals, median, busy time, recent jobs |
| `GET  /api/jobs`              | flat job list across all runs             |
| `POST /api/pipelines/trigger` | optional `{ "repo": "<name>" }` body — runs that repo's pipeline YAML (parsed by yaml-parser); without it, the local pipeline.yml. Returns `{ "id": <run id> }` |
| `GET  /api/runs`              | runs with nested jobs (shape below)       |
| `GET  /api/runs/{id}`         | one run                                   |
| `GET  /api/jobs/{id}/logs`    | `{ "output": "<full stdout+stderr>" }`    |
| `GET  /api/jobs/{id}/logs/stream` | server-sent events; each `log` event carries only the bytes written since the last one, and an `end` event closes the stream when the job is terminal |
| `GET  /api/repos`             | registered Forgejo repos (shape below)    |
| `POST /api/repos`             | register a repo: `{ "remote": "https://git.example.com/owner/repo" }` |
| `GET  /api/activity/calendar` | daily run counts for the past year        |
| `GET  /api/insights?range=<days>` | everything the Insights page shows for one window (1–365, clamped) |

Repos are registered from the dashboard's **+ Add repo** button (or curl).
The coordinator fetches metadata from the Forgejo instance in the URL
(repo info, languages, recent-commit authors, pipeline file probe),
persists the remotes in `repos.json` next to the binary, and re-fetches
every 2 minutes. Set `FORGEJO_TOKEN` in the coordinator's environment for
private repos.

### The shapes

**1. Runs.** The jobs created by one trigger, grouped under a run:

```jsonc
// GET /api/runs           → Run[]
// GET /api/runs/{id}      → Run
{
  "id": 3,
  "pipeline": "orchestrator-ci",
  "repo": "CI-CD-orchestrator",
  "pipeline_file": ".orchestrator/ci.yml",
  "trigger": "webhook",              // "webhook" | "manual" | "schedule"
  "branch": "main",                  // null for runs created before it was
                                     // recorded; not backfillable
  "commit": {                        // null for schedule-triggered runs
    "sha": "9c04b17",
    "message": "fix: reaper marks offline after 5s, not 50s",
    "author": "manish",
    "files": ["src/state.rs", ".orchestrator/ci.yml"]  // the feed filters on *.yml
  },
  "status": "failed",                // "pending" | "running" | "passed" | "failed"
  "created_at": 1783240000000,       // ms epoch — all timestamps below too
  "started_at": 1783240001500,
  "finished_at": 1783240050600,      // null while running
  "jobs": [ /* Job[], see below */ ]
}
```

**2. Jobs** with timing, exit code, and output (persisted from
`POST /api/jobs/{id}/report`; `output` powers every log view):

```jsonc
{
  "id": 7,
  "run_id": 3,
  "stage": "test",
  "name": "unit-tests",
  "command": "cargo test --lib",
  "status": "failed",
  "worker": "rohan-mac",             // null until claimed
  "ready_at": 1783240024000,         // when every `needs` passed AND a placement
                                     // was found — `started_at - ready_at` is the
                                     // queue wait, and nothing else records it
  "requeue_count": 0,                // times a worker died mid-job
  "started_at": 1783240026000,       // set on claim
  "finished_at": 1783240049000,      // set on report
  "exit_code": 1,                    // from the executor
  "first_error": "error: test failed, to rerun pass `--lib`",
                                     // one line distilled on report, so list
                                     // responses can explain a failure
  "output": null                     // OMITTED on list responses by design — a
                                     // run list carrying every job's build log
                                     // is megabytes per poll. Fetch it from
                                     // /api/jobs/{id}/logs when you need it.
}
```

The dashboard derives activity data (timelines, per-stage stats, the
monitor's fleet graph) from these two shapes. Machine-level CPU/RAM is the
one exception: workers *measure* it and ship it with every heartbeat, so
the device monitor graphs real numbers instead of inferring them.

**3. Workers** (`last_heartbeat`/`registered_at` are ms epoch; state lives
in Redis and `status` is computed from heartbeat age; `tags` are the
capability labels from `--tags`/`WORKER_TAGS`; `stats` is the latest
machine sample from the heartbeat, `null` until one arrives; `device` is
sampled **once at registration** — none of it changes while the agent lives,
so re-sending it on every heartbeat would be pure noise, and it is `null` for
agents too old to send one):

```jsonc
// GET /api/workers → Worker[]
{ "id": "6f9c…", "name": "rechek", "status": "online",
  "last_heartbeat": 1783240050000, "registered_at": 1783150000000,
  "tags": ["heavy"], "job_id": 7,
  "stats": { "cpu_pct": 57.2, "mem_pct": 58.1,
             "mem_used_mb": 9420, "mem_total_mb": 16384 },
  "device": {
    "os": "Debian GNU/Linux 12 (bookworm)",  // null where the OS cannot say
    "os_id": "debian",                       // the key the UI picks a logo by
    "kernel": "6.1.0-28-arm64", "host": "nxtcloud-m", "arch": "aarch64",
    "cpu": "Ampere Altra", "cpu_cores": 4, "cpu_physical": 4, "cpu_mhz": 3000,
    "mem_total_mb": 8192,
    "disk_total_gb": 160, "disk_free_gb": 96,  // the filesystem the workspace
                                               // is on, NOT every mounted disk
    "shell": "/bin/bash", "agent": "0.1.0", "executor": "local" } }
```

**3c. One machine's history.** Keyed by worker **name**, not id: a box that
lost its id file and re-registered is still the same machine to whoever is
reading the page. Answers for a name with no jobs rather than 404ing.

```jsonc
// GET /api/workers/{name}/activity → WorkerActivity
{ "name": "rechek",
  "calendar": [{ "date": "2026-08-07", "count": 4 }, ...],  // 365 days, oldest first
  "total_jobs": 427, "passed": 401, "failed": 26,
  "busy_ms": 18400000, "median_ms": 43000,   // median null until one job finishes
  "recent": [{ "job_id": 7, "run_id": 3, "repo": "…", "pipeline": "…",
               "stage": "test", "name": "unit-tests", "status": "passed",
               "started_at": 1783240026000, "finished_at": 1783240049000 }, ...] }
```

**3b. Worker stats history.** Each heartbeat (every 2s) appends a sample;
the coordinator keeps the last 450 per worker (~15 min) in Redis. The run
screen's device monitor draws these directly:

```jsonc
// GET /api/workers/stats → WorkerStatsSeries[]
{ "id": "6f9c…", "name": "rechek", "status": "online",
  "samples": [{ "t": 1783240050000, "cpu": 57.2, "mem": 58.1 }, ...] }
```

**3d. Pipeline definitions.** Each `PipelineRef` now carries the *declared*
shape, planned through the real yaml-parser when the coordinator discovers the
repo. That is what lets a pipeline show its stages before it has ever run —
and what lets a file that does not validate say so instead of rendering half a
graph.

```jsonc
{ "name": "orchestrator-nightly", "file": ".orchestrator/nightly.yml",
  "stages": ["e2e", "report"],
  "jobs": [{ "name": "chaos-kill-worker", "stage": "e2e",
             "needs": ["spawn-cluster"], "tags": ["heavy"] }],
  "parse_error": null,          // the reason, when the file does not plan
  "schedule": "0 2 * * *" }     // top-level `schedule:` in the file, verbatim
```

**4. Repos.** The coordinator proxies Forgejo: repo info from
`/api/v1/repos/{owner}/{repo}`, languages from `.../languages` (normalized
to percentages), contributors from the authors of the latest commits, and
pipelines by probing for `pipeline.yml` / `.orchestrator/ci.yml`:

```jsonc
// GET /api/repos → Repo[]
{
  "name": "CI-CD-orchestrator",
  "description": "Distributed CI/CD platform — ...",
  "language": "Rust",
  "branch": "main",
  "owner": "manish",
  "remote": "https://git.manishacharya.name.np/manish/CI-CD-orchestrator",
  "languages": [{ "name": "Rust", "pct": 71.3 }, ...],
  "contributors": [{ "login": "manish", "name": "Manish Acharya" }, ...],
  "pipelines": [{ "name": "orchestrator-ci", "file": ".orchestrator/ci.yml" }, ...]
}
```

Missing fields degrade gracefully ("not configured" / "unknown" / "no data").

**4b. Insights.** One window's aggregates, in one response. The coordinator
fetches a single job×run join for the window and derives everything from it in
memory (`src/insights.rs`), which is why every number here is unit-tested
without a database. Fields that cannot be computed are `null`, never `0`.

```jsonc
// GET /api/insights?range=30 → Insights
{ "range_days": 30, "runs": 412, "passed": 358, "failed": 54,
  "median_ms": 48000, "p90_ms": 98000,
  "recovery_ms": 2040000,      // median red → next green, per pipeline
  "longest_red_ms": 7800000,   // a pipeline still red counts up to now
  "calendar": [ /* CalendarDay[], a full year regardless of range */ ],
  "stages": [{ "stage": "test", "jobs": 3, "median_ms": 64000,
               "p90_ms": 98000, "runs": 190 }],   // stage WALL time, not the
                                                  // sum of parallel jobs
  "flaky": [{ "name": "migrate-db", "repo": "student-service",
              "flips": 4,            // commits where it both passed and failed
              "recent": "ppfpfppp" }],
  "trend": [{ "date": "2026-08-07", "runs": 14, "p50_ms": 46000, "p90_ms": 91000 }],
  "wait":  [{ "date": "2026-08-07", "wait_ms": 240000, "exec_ms": 1800000 }],
  "skew":  { "job": "unit-tests",    // null when no job ran on 2+ machines
             "workers": [{ "worker": "beefy-1", "runs": 41,
                           "median_ms": 42000, "pass_pct": 98 }] },
  "causes": [{ "cause": "database not seeded", "job": "migrate-db",
               "exit_code": 1, "count": 21 }] }
```

**4c. Webhook delivery.** Every repo carries the last time its webhook actually
reached the coordinator. Recorded for accepted *and* rejected deliveries, since
a rejection still proves the hook is wired up, and which kind it was is the
whole diagnostic. Stored in its own table rather than on the repo blob, which
the 2-minute Forgejo refresh overwrites wholesale.

```jsonc
"webhook": { "last_at": 1783240050000, "event": "push",
             "status": "accepted",     // or "rejected"
             "detail": null }          // why, when rejected
// absent entirely when no push has ever arrived — which is the case the
// dashboard flags, since "quiet" and "misconfigured" otherwise look identical
```

**5. Daily activity** for the contribution calendar:

```jsonc
// GET /api/activity/calendar → [{ "date": "2026-07-05", "count": 4 }, ...]
// one entry per day, past year, count = runs started that day
```

**6. Trigger** is `POST /api/pipelines/trigger` with an optional
`{ "repo": "<name>" }` body (the overview's "Trigger run" sends none).
Unregistered names fall back to the local pipeline.yml, then a built-in
default plan.

### Where the data lives

The coordinator persists runs/jobs/repos in **Postgres** and keeps the
worker registry + ready-job queue in **Redis** (both from the repo's
`docker-compose.yml` — `docker compose up -d`). The dashboard never sees
any of that; it only speaks the HTTP contract above.

### Polling for state, streaming for logs

Every page polls every 3 seconds for run/worker state, paused while the tab is
hidden (`src/lib/poll.ts`). That cadence is right for a status rail and wrong
for a build log — a stage that takes 30 seconds would show its output only
after it finished. So job logs go over **server-sent events** instead
(`src/lib/logstream.ts`), each event carrying only the bytes written since the
last one.

`EventSource` cannot send an `Authorization` header, so on a coordinator with
dashboard auth enabled the stream is refused; the client falls back to polling
the log endpoint and emitting the same deltas. A slower log, never no log. Mock
mode takes the same fallback path, since there is no server to stream from.

The coordinator's end (`job_log_stream` in `src/api.rs`) polls the row rather
than subscribing to a channel, deliberately: a job's log has two possible
writers — the worker forwarding its executor's tail, or the executor posting
straight to the coordinator — and a broadcast channel would only ever see one
of them.

## Source layout

    src/
      shell.css             the whole design system (documented in /DESIGN.md)
      main.ts               entry
      App.svelte            hash router outlet
      lib/
        api.ts              ENDPOINTS registry + live adapter + mode switch
        mock.ts             simulated coordinator (history + live-advancing run)
        types.ts            typed contract (mirror of this README)
        charts.ts           activity derivation + canvas charts
        format.ts           fmtDur / ago / status glyphs
        logsteps.ts         splits a log into `sh -x` steps + line classification
        logstream.ts        SSE job-log stream, with a polling fallback
        poll.ts             3s polling + 1s wall-clock store
        router.ts           tiny hash router
        yamlhl.ts           hand-rolled YAML highlighter for the pipeline view
        components/
          AppShell  Palette  FlowCanvas  FleetChart  Calendar
          OsLogo  FacetDropdown  Sparkline  Strip
      pages/
        Overview.svelte   Control center
        History.svelte    Runs
        RunDetail.svelte  one run
        Repos.svelte      repositories → pipelines → that pipeline's runs
        Workers.svelte    the fleet, and one device
        Insights.svelte   aggregates
        NotBuilt.svelte   404
        Login.svelte      shown only when the coordinator requires auth

    legacy/                 the original pre-redesign HTML shell (reference only)
