# Design

Implementation: **Svelte 5 + TypeScript + Vite** (`dashboard/src/`), hash-routed
static bundle. All HTTP goes through `src/lib/api.ts`; the whole visual system
lives in `src/shell.css`. There is no second stylesheet — the old `app.css` was
retired once the last page moved over, so a class defined in `shell.css` is the
only definition of it, and a collision is a bug rather than a cascade to reason
about.

The shape is a **persistent shell**: a dark sidebar on the left, a status rail
across the top, and a light workbench underneath. The shell never reloads; only
the page body changes. `100vh`, `overflow: hidden` — the app owns the viewport,
and each pane scrolls itself. Two pages opt out with `.page-body.scrolly` (the
device page and Insights) because they are genuinely taller than a screen.

## What each page is for

The site has one rule, and every page boundary follows from it:

> **If a card's primary key is a run id, it belongs on Control center or Runs.
> If its primary key is a job name, stage, worker or time bucket, it belongs on
> Insights.**

With the corollary: **Runs is about executions; Repositories is about
definitions.** Two pages may show the same underlying rows, but never answer the
same question with them.

| Route | The question it answers |
| --- | --- |
| `#/` | Is the system healthy *right now*? A glimpse only — no "view all" links, no history. |
| `#/runs` | What has executed, newest first? Filterable by status, repo, branch, trigger and window; searchable; keyboard-navigable. |
| `#/run/<id>` | What happened in this one run, and what is happening in it now? |
| `#/repos` | What is registered here? |
| `#/repos/<repo>` | What pipelines does this repo define? |
| `#/repos/<repo>/<pipeline>` | How does *this pipeline* behave over its own history? |
| `#/workers` | Is the fleet healthy right now? Every column is live state. |
| `#/workers/<name>` | What machine is this, and what has it done? |
| `#/insights` | Where is time going, what is flaky, and what should we fix? |

Pipelines have no nav item: a pipeline only exists inside a repository, so it is
reached by drilling down rather than by a fourth top-level list. Nav collapse
persists in `localStorage` (`orch.sidebar`).

The dashboard is strictly **read-only**. No trigger, retry or cancel buttons
anywhere — runs come from webhooks or `curl`, and the empty states teach the
`curl`. A button that lies about what it can do is worse than no button.

## Honesty rules

These are design constraints, not style preferences, and they are the reason
several obvious-looking widgets are missing:

- **Never invent a field.** No package counts or GPU rows in the device
  profile, because `sysinfo` cannot answer them portably. Where a field was
  missing only because the coordinator was discarding it, the fix was to record
  it rather than to fake it: branch, webhook delivery status and the pipeline
  definition index all became real for that reason. A pipeline with no indexed
  definition and no runs still says *"Shape appears after its first run"*.
- **Distinguish absent from broken.** A repo nobody has pushed to and one whose
  webhook URL is wrong are the same picture without a delivery record; a
  never-run pipeline and one whose YAML does not parse are the same empty card
  without a parse error. Both now say which they are.
- **A missing value says which kind of missing it is.** `none detected` and
  `unavailable · heartbeat stale` are different statements; so are `never went
  red` and `none`.
- **"Healthy" has to be earned.** The status rail reads *Degraded* whenever a
  worker is offline or the last poll failed. Reassurance during an outage is a
  bug.
- **Aggregates return `None`, not `0`, on no data.** An empty window reports
  nothing rather than a median of zero.
- **Every analytical panel ends in one sentence the numbers support.** A panel
  that cannot produce one is decoration and should be cut.
- **Say what was dropped.** Lists footer with "showing X of Y matching · Z
  total" rather than silently truncating.

## Color

OKLCH throughout. Restrained rather than committed: the dark band is now the
sidebar only, so colour in the content area is almost entirely semantic.

- Sidebar / dark surfaces: `--band` oklch(0.205 0.02 118), lines `--band-line`
  oklch(0.31), text `--band-ink` oklch(0.96), muted oklch(0.7 0.018 115)
- Ground `--bg` oklch(0.976 0.003 110); `--card` white; `--surface`
  oklch(0.962) for wells and hover; lines 0.912 / 0.855
- Ink: `--ink` oklch(0.225 0.012 110), `--ink-2` 0.38, `--muted` 0.53,
  `--faint` 0.66
- `--brand` oklch(0.5 0.105 112) olive — links, primary button, chart lines,
  active nav. `--lime` oklch(0.8 0.15 118) is the active-nav marker only.
- Status vocabulary — **glyph + word + colour, never colour alone**:
  passed `✓` `--ok` oklch(0.53 0.13 145) · failed `✕` `--fail` oklch(0.53 0.19 27)
  · running `●` `--run` oklch(0.63 0.13 78), pulsing · pending `○` line grey
- Dark islands (log terminal, device spec block): bg oklch(0.185–0.235 0.012 110),
  ink oklch(0.85–0.92), keys oklch(0.78 0.075 112)

## Typography

**IBM Plex Sans** and **IBM Plex Mono**, self-hosted via `@fontsource` — the
dashboard has to render on a LAN with no route to the internet, and a webfont
that never arrives is a page of fallbacks. Plex has engineering character and
holds its shape at 13px, which is the size most of this UI actually runs at.

- UI: `var(--ui)` — Plex Sans, 12.5–13.5px body, 19px page titles at -0.015em
- Data, readouts, logs, identifiers: `var(--mono)` — Plex Mono, 11–12.5px
- `font-variant-numeric: tabular-nums` on every number that updates in place
- Small-caps labels (10.5px/600, +0.07em uppercase) are for stat-well labels and
  section kickers only — never over prose

Sentence case everywhere. Em dashes are used sparingly and deliberately; when a
sentence needs three of them, it needs rewriting instead.

## Components

Shared, in `src/lib/components/`:

- **AppShell** — sidebar (collapsible; collapsed, the mark becomes the toggle),
  nav with live counts, status rail, ⌘K palette trigger. Owns the "Degraded"
  determination.
- **Palette** — native `<dialog>`, so focus trap, Escape and an inert backdrop
  come from the platform rather than from hand-written key handlers.
- **FlowCanvas** — the stage DAG, shared by Control center and Run detail so the
  graph behaves identically wherever you meet it. Stage = column; edges are
  bezier paths drawn from each job's `needs`, *not* from stage order, and are
  coloured by the dependency's status. Drag to pan (with a drag-vs-click guard),
  ⌘+scroll to zoom cursor-anchored, clamped 40–250%.
- **FleetChart** — demand vs capacity as a step function, not bars. The metric is
  a whole number of workers, and a step function is the honest mark type for a
  quantity that changes at instants. Demand is derived from `ready_at →
  finished_at`; capacity steps down when a worker is reaped; a saturation rail
  marks exact ties.
- **Calendar** — a year of daily counts. The shade scale is quartiles of the
  busiest day rather than a fixed count, so a machine doing 2 jobs a day and one
  doing 40 both produce a readable gradient. The legend names the busiest day so
  the shading is anchored to a real number.
- **OsLogo** — macOS / Windows / Ubuntu / Debian / Arch / Fedora / Alpine /
  generic Tux, drawn inline. An unrecognised `os_id` gets initials, not a guessed
  logo.
- **FacetDropdown** — disables itself when there is only one option rather than
  presenting a control that cannot do anything.
- **Strip** — one segment per job, in plan order: a run's shape at table width.
- **Sparkline** — trend only, scaled to its own min/max, since the headline
  number beside it already carries the absolute value.

## Page notes

- **Control center** — four KPI tiles, a pipeline picker feeding the DAG canvas
  (defaults to the newest running pipeline, falling back to the newest completed
  one), a worker cluster table, four recent runs, and the fleet chart. It fits
  one screen without scrolling, by design: it is a glimpse, and anything that
  would need a "view all" link belongs on the page that link would point at.
- **Runs** — status chips with counts, repo/trigger/when facets, search, a
  query-scoped summary line, day grouping with sticky headers, the failing job's
  distilled first error line inline, and attempt/requeue badges. Filter state
  mirrors into the URL so any view is linkable. `↑↓` navigate, `⏎` opens, `/`
  focuses search.
- **Run detail** — three panes. Left: what the pipeline *is* — its file, its
  stages, and the command each job runs (the command is under the name, because
  a job is what it runs). Centre: what is *happening* — the DAG when nothing is
  selected, that job's live log when something is. Right: the selected job's
  facts and its siblings, so moving between jobs in a stage does not mean going
  back to the tree. **Logs stream** (`lib/logstream.ts`) rather than poll: a
  stage's output has to appear while the stage runs, not after it. The terminal
  splits the log into steps on the `sh -x` `+ cmd` trace markers
  (`lib/logsteps.ts`), each collapsible, with follow/wrap/copy.
- **Repositories** — three levels behind one nav item. Repo cards carry a
  language bar, contributors, pass rate and latest run. Pipeline cards derive
  their stage chain from the most recent run. The pipeline's own run list carries
  duration-vs-median bars, which are only meaningful there because repo and
  pipeline are held constant.
- **Workers** — a list where every column is live state, and a device page that
  owns all the history. The spec block is a fastfetch-style dark panel at half
  width that scrolls inside itself, with the contribution calendar beside it, so
  a machine reporting more rows cannot shove the calendar off screen.
- **Insights** — every panel states its question in the header and ends with the
  answer. Run activity leads, because everything below is a claim about a window
  and that panel shows what the window contains. The rest: where the time goes
  (per-stage wall time, median behind p90), flaky jobs (both outcomes on the
  *same* commit — anything else is the code, not the job), duration trend
  (p50 vs p90), queue wait vs execution (more workers or faster ones?), worker
  skew for the busiest job, and failures ranked by cause.
- **404** — a broken-pipeline diagram that names the route that failed.

## Motion

150–250ms, ease-out, state changes only: the running pulse, the streaming-log
cursor, bar-width growth, row hover. No entrance choreography. The sidebar width
is switched rather than animated — tweening it would reflow the DAG canvas and
every chart on each frame.
