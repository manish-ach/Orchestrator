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

| Route              | What it shows                                                              |
| ------------------ | -------------------------------------------------------------------------- |
| `#/`               | Overview: stat tiles, contribution calendar, worker utilization, updates feed |
| `#/runs`           | Full run history: status filter chips + search over every run               |
| `#/repos`          | Repository list with search and latest-run bars                             |
| `#/repo/<name>`    | One repo: latest-run hero, pipeline switcher, runs, About/Contributors/Languages |
| `#/run/<id>`       | One run: flow canvas, per-stage stats + utilization, per-job full logs      |
| `#/run/<id>?job=<id>` | Deep link straight into a job's log view                                 |
| `#/monitor`        | Fleet utilization + per-worker health/timeline cards                        |

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
| `GET  /api/jobs`              | flat job list across all runs             |
| `POST /api/pipelines/trigger` | optional `{ "repo": "<name>" }` body — runs that repo's pipeline YAML (parsed by yaml-parser); without it, the local pipeline.yml. Returns `{ "id": <run id> }` |
| `GET  /api/runs`              | runs with nested jobs (shape below)       |
| `GET  /api/runs/{id}`         | one run                                   |
| `GET  /api/jobs/{id}/logs`    | `{ "output": "<full stdout+stderr>" }`    |
| `GET  /api/repos`             | registered Forgejo repos (shape below)    |
| `POST /api/repos`             | register a repo: `{ "remote": "https://git.example.com/owner/repo" }` |
| `GET  /api/activity/calendar` | daily run counts for the past year        |

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
  "started_at": 1783240026000,       // set on claim
  "finished_at": 1783240049000,      // set on report
  "exit_code": 1,                    // from the executor
  "output": "running 14 tests\n..."  // full stdout+stderr text
}
```

The dashboard derives *everything else* from these two shapes: timelines,
utilization charts, per-stage stats, the monitor's fleet graph — no extra
metrics endpoints are needed (and none should be invented: there is no
CPU/RAM data in this system).

**3. Workers** (`last_heartbeat`/`registered_at` are ms epoch; state lives
in Redis and `status` is computed from heartbeat age; `tags` are the
capability labels from `--tags`/`WORKER_TAGS`):

```jsonc
// GET /api/workers → Worker[]
{ "id": "6f9c…", "name": "rechek", "status": "online",
  "last_heartbeat": 1783240050000, "registered_at": 1783150000000,
  "tags": ["heavy"], "job_id": 7 }
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

### Polling, not push

Every page polls every 3 seconds (paused while the tab is hidden). No
WebSocket/SSE is required; if the coordinator later adds SSE, only
`src/lib/poll.ts` needs to change.

## Source layout

    src/
      app.css               the whole design system (documented in /DESIGN.md)
      main.ts               entry
      App.svelte            hash router outlet
      lib/
        api.ts              ENDPOINTS registry + live adapter + mode switch
        mock.ts             simulated coordinator (history + live-advancing run)
        types.ts            typed contract (mirror of this README)
        charts.ts           activity derivation + canvas charts
        format.ts           fmtDur / ago / status glyphs
        poll.ts             3s polling + 1s wall-clock store
        router.ts           tiny hash router
        components/         Topbar, Snackbar, StatusPill, Strip, Avatar, ...
      pages/
        Overview.svelte  Repos.svelte  RepoDetail.svelte  RunDetail.svelte  Monitor.svelte

    legacy/                 the original pre-redesign HTML shell (reference only)
