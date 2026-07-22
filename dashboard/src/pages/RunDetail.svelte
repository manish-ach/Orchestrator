<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, MODE } from '../lib/api';
  import DeviceMonitor from '../lib/components/DeviceMonitor.svelte';
  import StatusPill from '../lib/components/StatusPill.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { classify, parseSteps, type LineKind } from '../lib/logsteps';
  import { highlightYaml } from '../lib/yamlhl';
  import { now, startPolling } from '../lib/poll';
  import { back } from '../lib/router';
  import type { Job, JobStatus, LogLine, Run, Worker, WorkerStatsSeries } from '../lib/types';

  let { id, initialJob = null }: { id: string; initialJob?: string | null } = $props();

  let run = $state<Run | null>(null);
  let workers = $state<Worker[]>([]);
  let statSeries = $state<WorkerStatsSeries[]>([]);
  let error = $state('');
  let view = $state<string>('summary');
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
      const [r, o, s] = await Promise.all([api.run(id), api.overview(), api.workerStats()]);
      run = r;
      workers = o.workers;
      statSeries = s;
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

  // ---- summary stats ----------------------------------------------------------
  const passed = $derived(run ? run.jobs.filter((j) => j.status === 'passed').length : 0);
  const failedCount = $derived(run ? run.jobs.filter((j) => j.status === 'failed').length : 0);
  const pending = $derived(run ? run.jobs.filter((j) => j.status === 'pending').length : 0);
  const failedJobs = $derived(run ? run.jobs.filter((j) => j.status === 'failed') : []);
  const artifactCount = $derived(run ? run.jobs.filter((j) => j.has_artifacts).length : 0);
  const runWorkers = $derived(run ? [...new Set(run.jobs.filter((j) => j.worker).map((j) => j.worker!))] : []);

  const STATUS_TEXT: Record<JobStatus, string> = {
    passed: 'Success',
    failed: 'Failure',
    running: 'In progress',
    pending: 'Queued',
  };

  function jobStatusLine(j: Job): string {
    if (j.status === 'passed') return `succeeded ${ago(j.finished_at, $now)} in ${jobDur(j)}`;
    if (j.status === 'failed') {
      return j.started_at
        ? `failed ${ago(j.finished_at, $now)} in ${jobDur(j)}`
        : 'skipped — a dependency failed';
    }
    if (j.status === 'running') return `running for ${jobDur(j)}`;
    return 'queued — waiting for a worker';
  }

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
    if (!r || stage === 'summary' || stage === 'yaml' || jobSel !== null) {
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
</script>

{#snippet logline(text: string, n: number, kind: LineKind | 'cmd')}
  <div class="lnrow" class:err={kind === 'err'} class:warn={kind === 'warn'} class:ok={kind === 'ok'} class:meta={kind === 'meta'} class:cmd={kind === 'cmd'}>
    <span class="lnum" aria-hidden="true">{n}</span>
    <span class="ltxt">{text || ' '}</span>
  </div>
{/snippet}

<header class="run-top">
  <div class="run-top-inner">
    {#if run}
      <button type="button" class="run-crumb" onclick={() => back(run?.repo ? `/repo/${run.repo}` : '/')}>
        ← {run.pipeline}
      </button>
      <div class="run-title-row">
        <span class="ghst {run.status}" aria-label={STATUS_TEXT[run.status]}>{GLYPH[run.status]}</span>
        <h1 class="run-title" title={run.commit?.message ?? ''}>
          {run.commit?.message ?? `Run #${run.id}`}
          <span class="rnum">#{run.id}</span>
        </h1>
        <span class="run-top-side mono">{MODE} · polling 3s</span>
      </div>
    {:else}
      <button type="button" class="run-crumb" onclick={() => back('/')}>← runs</button>
      <div class="run-title-row"><h1 class="run-title">Run #{id}</h1></div>
    {/if}
  </div>
</header>

<div class="run-shell">
  <nav class="side-nav" aria-label="Run sections">
    <button type="button" class="snav-item" class:active={view === 'summary'} onclick={() => { view = 'summary'; jobSel = null; }}>
      <span class="g" aria-hidden="true">◈</span><span class="nname">Summary</span>
    </button>
    <div class="snav-group">Jobs</div>
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
    <div class="snav-group">Run details</div>
    <button type="button" class="snav-item" class:active={view === 'yaml'} onclick={() => { view = 'yaml'; jobSel = null; }}>
      <span class="g" aria-hidden="true">☰</span><span class="nname">Workflow file</span>
      <span class="ndur">{run?.pipeline_file.split('/').pop() ?? ''}</span>
    </button>
  </nav>

  <div class="run-content">
    {#if error}<div class="err-banner">{error}</div>{/if}

    {#if run}
      {#if view === 'summary'}
        <div class="card sum-card">
          <div class="sum-trigger">
            <span class="sum-k">Triggered via {run.trigger === 'webhook' ? 'push' : run.trigger} {ago(run.started_at, $now)}</span>
            <span class="sum-who">
              {#if run.commit}
                <b>{run.commit.author}</b> pushed
                <span class="sha-chip2">{run.commit.sha}</span>
              {:else}
                started from the dashboard
              {/if}
              <span class="repo-tag2">{run.repo}</span>
            </span>
          </div>
          <div class="sum-cell">
            <span class="sum-k">Status</span>
            <span class="sum-v st-{run.status}">{STATUS_TEXT[run.status]}</span>
          </div>
          <div class="sum-cell">
            <span class="sum-k">Total duration</span>
            <span class="sum-v">{fmtDur(runEnd(run) - run.started_at)}</span>
          </div>
          <div class="sum-cell">
            <span class="sum-k">Jobs</span>
            <span class="sum-v">{passed}/{run.jobs.length} <span class="sum-sub">passed</span></span>
          </div>
          <div class="sum-cell">
            <span class="sum-k">Artifacts</span>
            <span class="sum-v">{artifactCount}</span>
          </div>
        </div>

        <div class="card flow-card-canvas">
          <div class="flow-head">
            <span class="fname">{run.pipeline_file.split('/').pop()}</span>
            <span class="fon">on: {run.trigger === 'webhook' ? 'push' : run.trigger}</span>
            <button
              type="button"
              class="fname-btn"
              title={run.repo ? `${run.pipeline_file} — view the file` : 'this run has no registered repo to fetch the file from'}
              disabled={!run.repo}
              onclick={() => (view = 'yaml')}
            >
              view file →
            </button>
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

        {#if failedJobs.length}
          <div class="card annots">
            <div class="annots-head">Annotations <span class="meta2">{failedJobs.length} error{failedJobs.length === 1 ? '' : 's'}</span></div>
            {#each failedJobs as j (j.id)}
              <button type="button" class="annot" onclick={() => selectJob(j)}>
                <span class="ann-ico" aria-hidden="true">✕</span>
                <span class="ann-body">
                  <b>{j.name}</b>
                  <span class="ann-msg">
                    {j.started_at
                      ? `Process completed with exit code ${j.exit_code ?? 1}.`
                      : 'Skipped — a dependency failed.'}
                  </span>
                </span>
              </button>
            {/each}
          </div>
        {/if}

        <DeviceMonitor {workers} series={statSeries} names={runWorkers} now={$now} />
      {:else if view === 'yaml'}
        <div class="card yaml-card">
          <div class="yaml-card-head">
            <span class="glabel">Workflow file for this run</span>
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
              <div class="card logcard logcard-xl" class:failed={selJob.status === 'failed'}>
                <div class="logcard-head">
                  <span class="ghst sm {selJob.status}" aria-hidden="true">{GLYPH[selJob.status]}</span>
                  <span class="lc-name" title={selJob.name}>{selJob.name}</span>
                  <span class="lc-meta">{jobStatusLine(selJob)}</span>
                  <span class="log-tools">
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
                  class="logpane xl"
                  class:wrapped={logWrap}
                  bind:this={logBody}
                  onscroll={onLogScroll}
                  role="log"
                  aria-label="Log for {selJob.name}"
                >
                  {#if !log.length}
                    <div class="lnrow">
                      <span class="lnum" aria-hidden="true"></span>
                      <span class="ltxt meta">queued — no output yet</span>
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
                <div class="logcard-foot">
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
              <div class="section-label">Job logs
                <span class="meta">full output of every job in {view}</span>
              </div>
              {#each jobs as j (j.id)}
                {@const jlog = stageLogs[j.id] ?? []}
                <div class="card logcard stage-term" class:failed={j.status === 'failed'}>
                  <div class="logcard-head">
                    <span class="ghst sm {j.status}" aria-hidden="true">{GLYPH[j.status]}</span>
                    <span class="lc-name" title={j.name}>{j.name}</span>
                    <span class="lc-meta">{j.worker ?? 'unassigned'}</span>
                  </div>
                  <div class="logpane stage-log" role="log" aria-label="Log for {j.name}">
                    {#each jlog as l, i (i)}
                      {@render logline(l.t, i + 1, rawKind(l.t))}
                    {:else}
                      <div class="lnrow">
                        <span class="lnum" aria-hidden="true"></span>
                        <span class="ltxt meta">
                          {j.started_at || j.finished_at ? 'no output yet' : 'queued — waiting for a worker'}
                        </span>
                      </div>
                    {/each}
                  </div>
                  <div class="logcard-foot">
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
