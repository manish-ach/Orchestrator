<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, MODE } from '../lib/api';
  import { drawUtilChart, utilizationSeries } from '../lib/charts';
  import Snackbar from '../lib/components/Snackbar.svelte';
  import StatusPill from '../lib/components/StatusPill.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { classify, parseSteps, type LineKind } from '../lib/logsteps';
  import { highlightYaml } from '../lib/yamlhl';
  import { now, startPolling } from '../lib/poll';
  import type { Job, JobStatus, LogLine, Run, Worker } from '../lib/types';

  let { id, initialJob = null }: { id: string; initialJob?: string | null } = $props();

  let run = $state<Run | null>(null);
  let workers = $state<Worker[]>([]);
  let error = $state('');
  let view = $state<string>('overview');
  let jobSel = $state<number | null>(null);
  let logFollow = $state(true);
  let logWrap = $state(false);
  let logRaw = $state(false);
  let log = $state<LogLine[]>([]);
  // svelte-ignore state_referenced_locally — the deep link is consumed once, on purpose
  let deepLinkPending = initialJob !== null;
  let logBody = $state<HTMLElement | null>(null);
  let copied = $state(false);

  const stop = startPolling(async () => {
    try {
      const [r, o] = await Promise.all([api.run(id), api.overview()]);
      run = r;
      workers = o.workers;
      error = '';
      if (r && deepLinkPending) {
        deepLinkPending = false;
        const job = r.jobs.find((j) => j.id === Number(initialJob));
        if (job) {
          view = job.stage;
          jobSel = job.id;
        }
      }
    } catch (e) {
      error = `Cannot reach the data source (${(e as Error).message}). Retrying on the next poll.`;
    }
  });
  onDestroy(stop);

  function runEnd(r: Run): number {
    return r.finished_at ?? Date.now();
  }

  const stages = $derived(run ? [...new Set(run.jobs.map((j) => j.stage))] : []);
  const workerStatus = $derived(new Map(workers.map((w) => [w.name, w.status])));

  function stageJobs(stage: string): Job[] {
    return run ? run.jobs.filter((j) => j.stage === stage) : [];
  }

  function stageStatus(jobs: Job[]): JobStatus {
    if (jobs.some((j) => j.status === 'failed')) return 'failed';
    if (jobs.some((j) => j.status === 'running')) return 'running';
    if (jobs.every((j) => j.status === 'passed')) return 'passed';
    if (jobs.some((j) => j.status === 'passed')) return 'running';
    return 'pending';
  }

  function stageWindow(jobs: Job[]): { t0: number; t1: number } | null {
    const started = jobs.filter((j) => j.started_at);
    if (!started.length) return null;
    const t0 = Math.min(...started.map((j) => j.started_at!));
    const running = started.some((j) => !j.finished_at);
    const t1 = running ? Date.now() : Math.max(...started.map((j) => j.finished_at!));
    return { t0, t1: Math.max(t1, t0 + 1000) };
  }

  function jobDur(j: Job): string {
    if (!j.started_at) return '';
    return fmtDur((j.finished_at ?? $now) - j.started_at);
  }

  // ---- overview stats ---------------------------------------------------------
  const passed = $derived(run ? run.jobs.filter((j) => j.status === 'passed').length : 0);
  const failedCount = $derived(run ? run.jobs.filter((j) => j.status === 'failed').length : 0);
  const pending = $derived(run ? run.jobs.filter((j) => j.status === 'pending').length : 0);
  const runWorkers = $derived(run ? [...new Set(run.jobs.filter((j) => j.worker).map((j) => j.worker!))] : []);

  // ---- job selection + log ----------------------------------------------------
  function selectJob(job: Job) {
    view = job.stage;
    jobSel = job.id;
    logFollow = true;
    collapsedSteps = new Set();
  }
  function selectStage(stage: string) {
    view = stage;
    jobSel = null;
  }

  const selJob = $derived(run && jobSel !== null ? (run.jobs.find((j) => j.id === jobSel) ?? null) : null);

  $effect(() => {
    const r = run;
    const j = selJob;
    // skipped jobs finish without ever starting but still carry a log line
    if (!r || !j || (!j.started_at && !j.finished_at)) {
      log = [];
      return;
    }
    let cancelled = false;
    api.job(r.id, j.id).then((d) => {
      if (!cancelled && d) log = d.log;
    });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    void log;
    if (logFollow && logBody) logBody.scrollTop = logBody.scrollHeight;
  });

  // ---- step breakdown ---------------------------------------------------------
  let collapsedSteps = $state<Set<number>>(new Set());
  const rawLines = $derived(log.map((l) => l.t));
  const steps = $derived(parseSteps(rawLines));
  const hasSteps = $derived(steps.some((s) => s.cmd !== null));

  function toggleStep(i: number) {
    const next = new Set(collapsedSteps);
    if (next.has(i)) next.delete(i);
    else next.add(i);
    collapsedSteps = next;
  }
  function setAllSteps(collapsed: boolean) {
    collapsedSteps = collapsed ? new Set(steps.map((_, i) => i)) : new Set();
  }
  function rawKind(t: string): LineKind | 'cmd' {
    return /^\++ /.test(t) ? 'cmd' : classify(t);
  }

  // full logs of every job in the selected stage (stage view, no job
  // selected) — refetched on each poll so running jobs tail live
  let stageLogs = $state<Record<number, LogLine[]>>({});
  $effect(() => {
    const r = run;
    const stage = view;
    if (!r || stage === 'overview' || jobSel !== null) {
      stageLogs = {};
      return;
    }
    const started = r.jobs.filter((j) => j.stage === stage && (j.started_at || j.finished_at));
    let cancelled = false;
    Promise.all(
      started.map((j) => api.job(r.id, j.id).then((d) => [j.id, d?.log ?? []] as const)),
    ).then((entries) => {
      if (!cancelled) stageLogs = Object.fromEntries(entries);
    });
    return () => {
      cancelled = true;
    };
  });

  function onLogScroll() {
    if (!logBody) return;
    const atBottom = logBody.scrollHeight - logBody.scrollTop - logBody.clientHeight < 24;
    if (!atBottom && logFollow) logFollow = false;
  }

  async function copyLog() {
    await navigator.clipboard.writeText(rawLines.join('\n'));
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  // ---- pipeline YAML viewer (its own nav view) -----------------------------------
  let yaml = $state<{ file: string; content: string } | null>(null);
  let yamlError = $state('');
  // string keys so the effect refires only when the run actually changes,
  // not on every poll's fresh run object
  const yamlRepo = $derived(run?.repo ?? null);
  const yamlFile = $derived(run && /\.ya?ml$/.test(run.pipeline_file) ? run.pipeline_file : null);
  const yamlLines = $derived(yaml ? highlightYaml(yaml.content) : []);

  $effect(() => {
    const repo = yamlRepo;
    const file = yamlFile;
    if (view !== 'yaml' || !repo) return;
    yaml = null;
    yamlError = '';
    let cancelled = false;
    api.pipelineFile(repo, file ?? undefined).then(
      (d) => {
        if (!cancelled) yaml = d;
      },
      (e) => {
        if (!cancelled) yamlError = (e as Error).message;
      },
    );
    return () => {
      cancelled = true;
    };
  });

  // ---- DAG canvas layout --------------------------------------------------------
  // Real dependency edges from `needs`, not implied stage order: nodes are
  // laid out on a fixed grid (stage = column) and edges are bezier paths.
  const NODE_W = 220;
  const NODE_H = 62;
  const COL_GAP = 110;
  const ROW_GAP = 16;
  const DAG_TOP = 52;
  const DAG_PAD = 24;

  interface FlowNode {
    job: Job;
    x: number;
    y: number;
  }

  const flowNodes = $derived.by<FlowNode[]>(() => {
    if (!run) return [];
    return stages.flatMap((stage, ci) =>
      stageJobs(stage).map((j, ri) => ({
        job: j,
        x: DAG_PAD + ci * (NODE_W + COL_GAP),
        y: DAG_TOP + ri * (NODE_H + ROW_GAP),
      })),
    );
  });
  const dagW = $derived(DAG_PAD * 2 + Math.max(stages.length, 1) * (NODE_W + COL_GAP) - COL_GAP);
  const dagH = $derived.by(() => {
    const maxRows = Math.max(1, ...stages.map((s) => stageJobs(s).length));
    return DAG_TOP + maxRows * (NODE_H + ROW_GAP) - ROW_GAP + DAG_PAD;
  });

  let hoverJob = $state<string | null>(null);

  interface FlowEdge {
    from: string;
    to: string;
    path: string;
    status: JobStatus;
    active: boolean;
  }

  const flowEdges = $derived.by<FlowEdge[]>(() => {
    const byName = new Map(flowNodes.map((n) => [n.job.name, n]));
    const edges: FlowEdge[] = [];
    for (const n of flowNodes) {
      for (const dep of n.job.needs ?? []) {
        const from = byName.get(dep);
        if (!from) continue;
        const x1 = from.x + NODE_W;
        const y1 = from.y + NODE_H / 2;
        const x2 = n.x;
        const y2 = n.y + NODE_H / 2;
        const dx = Math.max(36, (x2 - x1) / 2);
        edges.push({
          from: dep,
          to: n.job.name,
          path: `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`,
          status: from.job.status,
          active: n.job.status === 'running',
        });
      }
    }
    return edges;
  });

  function edgeHl(e: FlowEdge): boolean {
    return hoverJob !== null && (e.from === hoverJob || e.to === hoverJob);
  }

  // ---- pan / zoom canvas ------------------------------------------------------
  let pan = $state({ x: 0, y: 0 });
  let scale = $state(1);
  let dragging = $state<{ x: number; y: number } | null>(null);
  let dragMoved = false;
  let canvasEl = $state<HTMLElement | null>(null);

  function setScale(next: number, cx: number, cy: number) {
    const s = Math.min(2.5, Math.max(0.4, next));
    pan = { x: cx - ((cx - pan.x) / scale) * s, y: cy - ((cy - pan.y) / scale) * s };
    scale = s;
  }
  function onPointerDown(e: PointerEvent) {
    dragging = { x: e.clientX - pan.x, y: e.clientY - pan.y };
    dragMoved = false;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    const nx = e.clientX - dragging.x;
    const ny = e.clientY - dragging.y;
    if (Math.abs(nx - pan.x) > 3 || Math.abs(ny - pan.y) > 3) dragMoved = true;
    pan = { x: nx, y: ny };
  }
  function onPointerUp() {
    dragging = null;
  }
  function onCanvasClick(e: MouseEvent) {
    if (dragMoved) {
      e.stopPropagation();
      e.preventDefault();
      dragMoved = false;
    }
  }
  function onWheel(e: WheelEvent) {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    const rect = (canvasEl as HTMLElement).getBoundingClientRect();
    setScale(scale * (e.deltaY < 0 ? 1.12 : 0.89), e.clientX - rect.left, e.clientY - rect.top);
  }
  function zoomCenter(factor: number) {
    if (!canvasEl) return;
    setScale(scale * factor, canvasEl.clientWidth / 2, canvasEl.clientHeight / 2);
  }

  // ---- utilization chart action ------------------------------------------------
  interface UtilParams {
    values: number[];
    totalLabel: string;
    midLabel: string;
  }
  function utilChart(canvas: HTMLCanvasElement, params: UtilParams) {
    drawUtilChart(canvas, params.values, params);
    return {
      update(next: UtilParams) {
        drawUtilChart(canvas, next.values, next);
      },
    };
  }

  function workerUtilParams(name: string, jobs: Job[], t0: number, t1: number): UtilParams {
    const mine = jobs.filter((j) => j.worker === name);
    return {
      values: utilizationSeries(mine, t0, t1),
      totalLabel: fmtDur(Math.max(t1 - t0, 1000)),
      midLabel: fmtDur(Math.max(t1 - t0, 1000) / 2),
    };
  }

  function busyPct(name: string, jobs: Job[], t0: number, t1: number): number {
    const total = Math.max(t1 - t0, 1000);
    const busy = jobs
      .filter((j) => j.worker === name && j.started_at)
      .reduce((s, j) => s + (Math.min(j.finished_at ?? t1, t1) - j.started_at!), 0);
    return Math.min(100, Math.round((busy / total) * 100));
  }

  function jobsOf(name: string, jobs: Job[]): number {
    return jobs.filter((j) => j.worker === name && j.started_at).length;
  }
</script>

{#snippet logline(text: string, n: number, kind: LineKind | 'cmd')}
  <div class="lnrow" class:err={kind === 'err'} class:warn={kind === 'warn'} class:ok={kind === 'ok'} class:meta={kind === 'meta'} class:cmd={kind === 'cmd'}>
    <span class="lnum" aria-hidden="true">{n}</span>
    <span class="ltxt">{text || ' '}</span>
  </div>
{/snippet}

<Snackbar fallback="/">
  {#if run}
    <StatusPill status={run.status} />
    <span class="stitle" title={run.commit?.message ?? ''}>{run.commit?.message ?? `Run #${run.id}`}</span>
    <span class="smeta">
      {run.pipeline} #{run.id}
      {#if run.commit}<span aria-hidden="true">·</span> <span class="sha-chip">{run.commit.sha}</span>
        <span aria-hidden="true">·</span> by {run.commit.author}{/if}
      <span aria-hidden="true">·</span> via {run.trigger}
      <span aria-hidden="true">·</span> {ago(run.started_at, $now)}
      <span aria-hidden="true">·</span> {fmtDur(runEnd(run) - run.started_at)}
    </span>
  {:else}
    <span class="stitle">Run #{id}</span>
  {/if}
</Snackbar>

<div class="run-shell">
  <nav class="side-nav" aria-label="Run sections">
    <button type="button" class="snav-item" class:active={view === 'overview'} onclick={() => (view = 'overview')}>
      <span class="g" aria-hidden="true">◈</span><span class="nname">Overview</span>
    </button>
    <button type="button" class="snav-item" class:active={view === 'yaml'} onclick={() => { view = 'yaml'; jobSel = null; }}>
      <span class="g" aria-hidden="true">☰</span><span class="nname">Pipeline file</span>
      <span class="ndur">{run?.pipeline_file.split('/').pop() ?? ''}</span>
    </button>
    <div class="snav-group">Stages</div>
    {#each stages as stage (stage)}
      {@const jobs = stageJobs(stage)}
      <button
        type="button"
        class="snav-item snav-stage"
        class:active={view === stage && jobSel === null}
        onclick={() => selectStage(stage)}
      >
        <span class="g {stageStatus(jobs)}" aria-hidden="true">{GLYPH[stageStatus(jobs)]}</span>
        <span class="nname">{stage}</span>
        <span class="ndur">{jobs.filter((j) => j.status === 'passed').length}/{jobs.length}</span>
      </button>
      <div class="snav-jobs">
        {#each jobs as j (j.id)}
          <button
            type="button"
            class="snav-item snav-sub"
            class:active={jobSel === j.id}
            title="$ {j.command}"
            onclick={() => selectJob(j)}
          >
            <span class="g {j.status}" aria-hidden="true">{GLYPH[j.status]}</span>
            <span class="ncol">
              <span class="nrow">
                <span class="nname">{j.name}</span>
                <span class="ndur">{jobDur(j)}</span>
              </span>
              <span class="ncmd">$ {j.command}</span>
            </span>
          </button>
        {/each}
      </div>
    {/each}
  </nav>

  <div class="run-content">
    {#if error}<div class="err-banner">{error}</div>{/if}

    {#if run}
      {#if view === 'overview'}
        <div class="ov-stats">
          <span class="card ovstat"><span class="k">duration</span><span class="v">{fmtDur(runEnd(run) - run.started_at)}</span></span>
          <span class="card ovstat">
            <span class="k">jobs</span>
            <span class="v">
              {passed}/{run.jobs.length}
              {#if failedCount}<span class="bad">· {failedCount} failed</span>{:else}passed{/if}
            </span>
          </span>
          <span class="card ovstat"><span class="k">stages</span><span class="v">{stages.length}</span></span>
          <span class="card ovstat"><span class="k">workers</span><span class="v">{runWorkers.length}</span></span>
        </div>

        <div class="card flow-card-canvas">
          <div class="flow-head">
            <span class="glabel">Pipeline execution flow</span>
            <button
              type="button"
              class="fname fname-btn"
              title={run.repo ? `${run.pipeline_file} — view the file` : 'this run has no registered repo to fetch the file from'}
              disabled={!run.repo}
              onclick={() => (view = 'yaml')}
            >
              {run.pipeline_file.split('/').pop()} →
            </button>
            <span class="fon">on: {run.trigger === 'webhook' ? 'push' : run.trigger}</span>
            <span class="zoom-ctrls">
              <button class="zoombtn" onclick={() => zoomCenter(1 / 1.2)} aria-label="Zoom out">−</button>
              <span class="zoom-pct">{Math.round(scale * 100)}%</span>
              <button class="zoombtn" onclick={() => zoomCenter(1.2)} aria-label="Zoom in">+</button>
              <button class="zoombtn fit" onclick={() => { pan = { x: 0, y: 0 }; scale = 1; }}>reset</button>
            </span>
          </div>
          <div
            class="flow-canvas"
            class:grabbing={dragging !== null}
            role="application"
            aria-label="Pipeline flow canvas — drag to pan, ctrl+scroll to zoom"
            bind:this={canvasEl}
            onpointerdown={onPointerDown}
            onpointermove={onPointerMove}
            onpointerup={onPointerUp}
            onclickcapture={onCanvasClick}
            onwheel={onWheel}
          >
            <div class="flow-stage" style="transform: translate({pan.x}px, {pan.y}px) scale({scale})">
              <div class="dag" style="width:{dagW}px;height:{dagH}px">
                <svg class="dag-svg" width={dagW} height={dagH} aria-hidden="true">
                  {#each flowEdges as e (e.from + '→' + e.to)}
                    <path
                      class="dag-edge {e.status}"
                      class:activeflow={e.active}
                      class:hl={edgeHl(e)}
                      class:dim={hoverJob !== null && !edgeHl(e)}
                      d={e.path}
                    />
                  {/each}
                </svg>
                {#each stages as stage, ci (stage)}
                  {@const jobs = stageJobs(stage)}
                  <span class="fstage-row dag-label" style="left:{DAG_PAD + ci * (NODE_W + COL_GAP)}px; width:{NODE_W}px">
                    <span class="fstage-label">{stage}</span>
                    <span class="fst-ico {stageStatus(jobs)}" aria-label={stageStatus(jobs)}>{GLYPH[stageStatus(jobs)]}</span>
                  </span>
                {/each}
                {#each flowNodes as n (n.job.id)}
                  <button
                    type="button"
                    class="fnode dag-node"
                    class:is-failed={n.job.status === 'failed'}
                    class:is-pending={n.job.status === 'pending'}
                    class:dimmed={hoverJob !== null && hoverJob !== n.job.name && !flowEdges.some((e) => edgeHl(e) && (e.from === n.job.name || e.to === n.job.name))}
                    style="left:{n.x}px; top:{n.y}px; width:{NODE_W}px; height:{NODE_H}px"
                    onclick={() => selectJob(n.job)}
                    onmouseenter={() => (hoverJob = n.job.name)}
                    onmouseleave={() => (hoverJob = null)}
                  >
                    <span class="fcircle {n.job.status}" aria-hidden="true">{n.job.status === 'pending' ? '' : GLYPH[n.job.status]}</span>
                    <span class="fbody">
                      <span class="fname2">{n.job.name}</span>
                      {#if n.job.status === 'failed'}
                        <span class="fsub bad">
                          {n.job.started_at
                            ? `failed at ${fmtDur((n.job.finished_at ?? $now) - n.job.started_at)}`
                            : 'skipped — dependency failed'}
                        </span>
                      {:else if n.job.status === 'pending'}
                        <span class="fsub">{n.job.needs?.length ? `waiting on ${n.job.needs.join(', ')}` : 'queued'}</span>
                      {:else if n.job.started_at}
                        <span class="fsub">{fmtDur((n.job.finished_at ?? $now) - n.job.started_at)}{#if n.job.worker} · {n.job.worker}{/if}</span>
                      {/if}
                    </span>
                  </button>
                {/each}
              </div>
            </div>
            <span class="flow-hint">drag to pan · ctrl+scroll to zoom · click a job for its log</span>
          </div>
        </div>

        <div class="section-label">Worker utilization
          <span class="meta">share of time busy across the run window</span>
        </div>
        <div class="util-grid" style="margin-bottom:24px">
          {#if runWorkers.length}
            {#each runWorkers as name (name)}
              <div class="card util-card" class:offline={workerStatus.get(name) === 'offline'}>
                <div class="util-head">
                  <span class="dot" aria-hidden="true"></span>{name}
                  <span class="umeta">
                    jobs <b>{jobsOf(name, run.jobs)}</b> · busy <b>{busyPct(name, run.jobs, run.started_at, runEnd(run))}%</b> of run
                  </span>
                </div>
                <canvas
                  class="utilchart"
                  use:utilChart={workerUtilParams(name, run.jobs, run.started_at, runEnd(run))}
                  aria-label="Utilization of {name} during this run"
                ></canvas>
              </div>
            {/each}
          {:else}
            <div class="empty">No jobs have been assigned to a worker yet.</div>
          {/if}
        </div>
      {:else if view === 'yaml'}
        <div class="card yaml-card">
          <div class="yaml-card-head">
            <span class="glabel">Pipeline file</span>
            <span class="mono yfile">{yaml?.file ?? run.pipeline_file}</span>
            <span class="fon">defines this run's stages, jobs and placement</span>
          </div>
          {#if !run.repo}
            <div class="empty">This run has no registered repo to fetch the file from.</div>
          {:else if yamlError}
            <div class="empty">{yamlError}</div>
          {:else if !yaml}
            <div class="empty">fetching pipeline file…</div>
          {:else}
            <pre class="yaml-pre yaml-hl">{#each yamlLines as toks, i (i)}<span class="yline"><span class="ylnum" aria-hidden="true">{i + 1}</span><span class="ycode">{#each toks as tk, ti (ti)}{#if tk.cls}<span class={tk.cls}>{tk.text}</span>{:else}{tk.text}{/if}{:else}{' '}{/each}</span></span>{/each}</pre>
            <div class="yaml-note">current version on the repo — the run may have used an older commit</div>
          {/if}
        </div>
      {:else}
        {@const jobs = stageJobs(view)}
        {@const win = stageWindow(jobs)}
        {@const stWorkers = [...new Set(jobs.filter((j) => j.worker).map((j) => j.worker!))]}
        {@const failedJob = jobs.find((j) => j.status === 'failed')}
        {#if selJob}
          <div class="job-shell">
            <div class="job-main">
              <div class="term term-xl" class:failed={selJob.status === 'failed'}>
                <div class="term-head">
                  <span class="dot" aria-hidden="true"></span>{selJob.worker ?? 'unassigned'}
                  <span class="tjob" title={selJob.name}>{selJob.status === 'failed' ? `${GLYPH.failed} ` : ''}{selJob.name}</span>
                  <span class="term-tools">
                    {#if hasSteps}
                      <button type="button" class="ttool" aria-pressed={!logRaw} onclick={() => (logRaw = !logRaw)}>
                        {logRaw ? 'raw' : 'steps'}
                      </button>
                      {#if !logRaw}
                        <button type="button" class="ttool" onclick={() => setAllSteps(collapsedSteps.size === 0)}>
                          {collapsedSteps.size === 0 ? 'collapse all' : 'expand all'}
                        </button>
                      {/if}
                    {/if}
                    <button type="button" class="ttool" aria-pressed={logFollow}
                      onclick={() => { logFollow = !logFollow; if (logFollow && logBody) logBody.scrollTop = logBody.scrollHeight; }}>
                      follow
                    </button>
                    <button type="button" class="ttool" aria-pressed={logWrap} onclick={() => (logWrap = !logWrap)}>wrap</button>
                    <button type="button" class="ttool" onclick={copyLog}>{copied ? 'copied ✓' : 'copy'}</button>
                  </span>
                </div>
                <div
                  class="term-body full xl"
                  class:wrapped={logWrap}
                  bind:this={logBody}
                  onscroll={onLogScroll}
                  role="log"
                  aria-label="Log for {selJob.name}"
                >
                  {#if !log.length}
                    <div class="lnrow">
                      <span class="lnum" aria-hidden="true"></span>
                      <span class="ltxt" style="color:var(--term-muted)">queued — no output yet</span>
                    </div>
                  {:else if hasSteps && !logRaw}
                    {#each steps as s, si (si)}
                      <div class="step" class:stepfail={selJob.status === 'failed' && si === steps.length - 1}>
                        <button type="button" class="step-head" aria-expanded={!collapsedSteps.has(si)} onclick={() => toggleStep(si)}>
                          <span class="chev" aria-hidden="true">{collapsedSteps.has(si) ? '▸' : '▾'}</span>
                          <span class="step-ix">{si + 1}</span>
                          <span class="step-cmd">{s.cmd ?? 'setup'}</span>
                          <span class="step-meta">{s.lines.length} line{s.lines.length === 1 ? '' : 's'}</span>
                        </button>
                        {#if !collapsedSteps.has(si)}
                          {#each s.lines as l, i (i)}
                            {@render logline(l, s.start + i, classify(l))}
                          {/each}
                        {/if}
                      </div>
                    {/each}
                  {:else}
                    {#each rawLines as l, i (i)}
                      {@render logline(l, i + 1, rawKind(l))}
                    {/each}
                  {/if}
                </div>
                <div class="term-foot">
                  <span class="tcmd" title="$ {selJob.command}">$ {selJob.command}</span>
                  {#if selJob.status === 'running'}
                    <span class="tlines streaming">streaming…</span>
                  {:else}
                    <span class="tlines">{rawLines.length} lines{#if selJob.exit_code !== null} · exit {selJob.exit_code}{/if}</span>
                  {/if}
                </div>
              </div>
            </div>

            <aside class="card stage-info" aria-label="Job details">
              <div class="si-head"><span class="sname">{selJob.name}</span><StatusPill status={selJob.status} /></div>
              <div class="sirow"><span class="sik">stage</span><span class="siv">{selJob.stage}</span></div>
              <div class="sirow"><span class="sik">worker</span><span class="siv">{selJob.worker ?? 'unassigned'}</span></div>
              <div class="sirow"><span class="sik">duration</span><span class="siv">{jobDur(selJob) || '—'}</span></div>
              <div class="sirow">
                <span class="sik">started</span>
                <span class="siv">{selJob.started_at ? `+${fmtDur(selJob.started_at - run.started_at)} into the run` : 'not started'}</span>
              </div>
              {#if selJob.exit_code !== null}
                <div class="sirow"><span class="sik">exit code</span><span class="siv" class:bad={selJob.exit_code !== 0}>{selJob.exit_code}</span></div>
              {/if}
              {#if selJob.needs?.length}
                <div class="sirow"><span class="sik">needs</span><span class="siv">{selJob.needs.join(', ')}</span></div>
              {/if}
              {#if selJob.tags?.length}
                <div class="sirow"><span class="sik">worker tags</span><span class="siv">{selJob.tags.join(', ')}</span></div>
              {/if}
              {#if selJob.artifacts?.length}
                <div class="sirow">
                  <span class="sik">artifacts</span>
                  <span class="siv">{selJob.artifacts.join(', ')}{selJob.has_artifacts ? ' · uploaded ✓' : ''}</span>
                </div>
              {/if}
              <div class="sirow"><span class="sik">command</span><pre class="si-cmd">{selJob.command}</pre></div>
              <button type="button" class="siv-link si-back" onclick={() => selectStage(selJob.stage)}>← all of {selJob.stage}</button>
            </aside>
          </div>
        {:else}
          <div class="stage-row">
            <div>
              <div class="section-label">Worker utilization <span class="meta">within this stage's window</span></div>
              {#if !win || !stWorkers.length}
                <div class="empty">This stage has not started yet.</div>
              {:else}
                <div class="util-grid">
                  {#each stWorkers as name (name)}
                    <div class="card util-card" class:offline={workerStatus.get(name) === 'offline'}>
                      <div class="util-head">
                        <span class="dot" aria-hidden="true"></span>{name}
                        <span class="umeta">jobs <b>{jobsOf(name, jobs)}</b> · busy <b>{busyPct(name, jobs, win.t0, win.t1)}%</b></span>
                      </div>
                      <canvas
                        class="utilchart"
                        use:utilChart={workerUtilParams(name, jobs, win.t0, win.t1)}
                        aria-label="Utilization of {name}"
                      ></canvas>
                    </div>
                  {/each}
                </div>
              {/if}

              <div class="section-label" style="margin-top:20px">Command logs
                <span class="meta">full output of every job in {view}</span>
              </div>
              {#each jobs as j (j.id)}
                {@const jlog = stageLogs[j.id] ?? []}
                <div class="term stage-term" class:failed={j.status === 'failed'}>
                  <div class="term-head">
                    <span class="dot" aria-hidden="true"></span>{j.worker ?? 'unassigned'}
                    <span class="tjob" title={j.name}>{j.status === 'failed' ? `${GLYPH.failed} ` : ''}{j.name}</span>
                  </div>
                  <div class="term-body full stage-log" role="log" aria-label="Log for {j.name}">
                    {#each jlog as l, i (i)}
                      {@render logline(l.t, i + 1, rawKind(l.t))}
                    {:else}
                      <div class="lnrow">
                        <span class="lnum" aria-hidden="true"></span>
                        <span class="ltxt" style="color:var(--term-muted)">
                          {j.started_at || j.finished_at ? 'no output yet' : 'queued — waiting for a worker'}
                        </span>
                      </div>
                    {/each}
                  </div>
                  <div class="term-foot">
                    <span class="tcmd" title="$ {j.command}">$ {j.command}</span>
                    {#if j.status === 'running'}
                      <span class="tlines streaming">streaming…</span>
                    {:else}
                      <span class="tlines">{jlog.length} lines · <button type="button" class="siv-link" onclick={() => selectJob(j)}>open</button></span>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>

            <aside class="card stage-info" aria-label="Stage details">
              <div class="si-head"><span class="sname">{view}</span><StatusPill status={stageStatus(jobs)} /></div>
              <div class="sirow"><span class="sik">jobs</span><span class="siv">{jobs.filter((j) => j.status === 'passed').length}/{jobs.length} passed</span></div>
              <div class="sirow"><span class="sik">duration</span><span class="siv">{win ? fmtDur(win.t1 - win.t0) : '—'}</span></div>
              <div class="sirow"><span class="sik">starts at</span><span class="siv">{win ? `+${fmtDur(win.t0 - run.started_at)} into the run` : 'not started'}</span></div>
              <div class="sirow"><span class="sik">workers</span><span class="siv">{stWorkers.length ? stWorkers.join(', ') : '—'}</span></div>
              {#if failedJob}
                <div class="sirow">
                  <span class="sik">failed at</span>
                  <span class="siv">
                    <button type="button" class="siv-link" onclick={() => selectJob(failedJob)}>
                      {failedJob.name} · exit {failedJob.exit_code}
                    </button>
                  </span>
                </div>
              {/if}
            </aside>
          </div>
        {/if}
      {/if}

      <footer class="foot">
        mode: {MODE} · polling every 3s · ✓ {passed} passed · ✕ {failedCount} failed · ○ {pending} pending
      </footer>
    {:else if !error}
      <div class="empty" style="margin:24px 0">Run #{id} not found.</div>
    {/if}
  </div>
</div>
