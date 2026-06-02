# Module 3: Web Dashboard (Read-Only)

A simple web page that shows a list of pipelines and their status, with a detail view for logs. Read-only — no buttons, no auth, no real-time. The most beginner-friendly slice of the platform: visual, instantly rewarding, no infrastructure.

## key notes

- Visual feedback every time you change a line of code
- Can be built with **mock JSON files** — no backend needed for weeks
- Plugs into the Coordinator API later by swapping the data source
- Teaches: HTML/CSS, `fetch`, async JS, basic state, deploying a static site

## What to build

1. A **pipeline list** page — table of recent runs (name, branch, status, duration, time)
2. A **pipeline detail** page — shows each job's status (pending / running / passed / failed)
3. A **logs view** — shows stdout/stderr for a selected job
4. **Auto-refresh** every 3 seconds (just re-fetch — no WebSockets)
5. Color-code statuses (green / red / yellow / grey)

## Pages / routes

- `/` → list of pipelines
- `/pipelines/:id` → job graph for one pipeline
- `/pipelines/:id/jobs/:jobId` → logs for one job

## Tools

| Purpose | Tool |
|---|---|
| Language | HTML + CSS + plain JavaScript (no framework) |
| Styling | Tailwind CSS via CDN (no build step) |
| Mock data | Static JSON files in a `/mock` folder |
| Dev server | `python -m http.server` or VS Code Live Server |
| Deploy | GitHub Pages or Netlify (free, drag-and-drop) |

If they want a small step up later: **React + Vite + Tailwind**.

## Mock data shape

Drop these in `/mock/` and `fetch()` them like a real API:

```json
// mock/pipelines.json
[
  { "id": "p1", "name": "build-and-test", "branch": "main",
    "status": "passed", "duration_sec": 142, "started_at": "2026-05-18T10:00:00Z" },
  { "id": "p2", "name": "build-and-test", "branch": "feature/auth",
    "status": "running", "duration_sec": 30, "started_at": "2026-05-18T10:05:00Z" }
]
```

```json
// mock/pipeline-p1.json
{
  "id": "p1",
  "jobs": [
    { "id": "j1", "name": "compile",    "status": "passed", "duration_sec": 40 },
    { "id": "j2", "name": "unit-tests", "status": "passed", "duration_sec": 102 }
  ]
}
```

```json
// mock/logs-j1.json
{ "stdout": "Compiling...\nDone.\n", "stderr": "" }
```

## Done when

- All three pages render real-looking data from the mock files
- Statuses are color-coded and easy to scan
- Auto-refresh updates the list without a full page reload
- Works on mobile (Tailwind makes this trivial)
- Deployed to a public URL you can share

## Don't build yet

Auth, login, retrying jobs, triggering pipelines, live log streaming, user settings, dark mode toggle. Read-only is the whole point of this module.

## How it connects later

Replace each `fetch('/mock/pipelines.json')` with `fetch('/api/pipelines')` once the Coordinator API exists. The UI itself doesn't change — that's the magic of starting with mocks.
