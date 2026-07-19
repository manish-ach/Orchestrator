<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, MODE } from '../lib/api';
  import { drawUtilChart, utilizationSeries } from '../lib/charts';
  import Snackbar from '../lib/components/Snackbar.svelte';
  import StatusPill from '../lib/components/StatusPill.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
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
    await navigator.clipboard.writeText(log.map((l) => l.t).join('\n'));
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  // ---- pipeline YAML viewer -----------------------------------------------------
  let showYaml = $state(false);
  let yaml = $state<{ file: string; content: string } | null>(null);
  let yamlError = $state('');
  // string keys so the effect refires only when the run actually changes,
  // not on every poll's fresh run object
  const yamlRepo = $derived(run?.repo ?? null);
  const yamlFile = $derived(run && /\.ya?ml$/.test(run.pipeline_file) ? run.pipeline_file : null);

  $effect(() => {
    const repo = yamlRepo;
    const file = yamlFile;
    if (!showYaml || !repo) return;
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
    <div class="snav-group">Stages</div>
    {#each stages as stage (stage)}
      {@const jobs = stageJobs(stage)}
      <button
        type="button"
        class="snav-item"
        class:active={view === stage && jobSel === null}
        onclick={() => selectStage(stage)}
      >
        <span class="g {stageStatus(jobs)}" aria-hidden="true">{GLYPH[stageStatus(jobs)]}</span>
        <span class="nname">{stage}</span>
      </button>
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
            <span class="nname">{j.name}</span>
            <span class="ncmd">$ {j.command}</span>
          </span>
        </button>
      {/each}
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
              title={run.repo ? `${run.pipeline_file} — click to ${showYaml ? 'hide' : 'view'} the file` : 'this run has no registered repo to fetch the file from'}
              disabled={!run.repo}
              onclick={() => (showYaml = !showYaml)}
            >
              {run.pipeline_file.split('/').pop()}{showYaml ? ' ▴' : ' ▾'}
            </button>
            <span class="fon">on: {run.trigger === 'webhook' ? 'push' : run.trigger}</span>
            <span class="zoom-ctrls">
              <button class="zoombtn" onclick={() => zoomCenter(1 / 1.2)} aria-label="Zoom out">−</button>
              <span class="zoom-pct">{Math.round(scale * 100)}%</span>
              <button class="zoombtn" onclick={() => zoomCenter(1.2)} aria-label="Zoom in">+</button>
              <button class="zoombtn fit" onclick={() => { pan = { x: 0, y: 0 }; scale = 1; }}>reset</button>
            </span>
          </div>
          {#if showYaml}
            <div class="yaml-view">
              {#if yamlError}
                <div class="empty">{yamlError}</div>
              {:else if !yaml}
                <div class="empty">fetching pipeline file…</div>
              {:else}
                <div class="yaml-head">
                  <span class="mono">{yaml.file}</span>
                  <span class="dim">current version on the repo — the run may have used an older commit</span>
                </div>
                <pre class="yaml-pre">{yaml.content}</pre>
              {/if}
            </div>
          {/if}
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
              <div class="flow">
                {#each stages as stage, i (stage)}
                  {@const jobs = stageJobs(stage)}
                  {#if i > 0}<div class="fconn" aria-hidden="true"></div>{/if}
                  <div class="fcol-wrap">
                    <span class="fstage-row">
                      <span class="fstage-label">{stage}</span>
                      <span class="fst-ico {stageStatus(jobs)}" aria-label={stageStatus(jobs)}>{GLYPH[stageStatus(jobs)]}</span>
                    </span>
                    <div class="fcol">
                      {#each jobs as j (j.id)}
                        <button
                          type="button"
                          class="fnode"
                          class:is-failed={j.status === 'failed'}
                          class:is-pending={j.status === 'pending'}
                          onclick={() => selectJob(j)}
                        >
                          <span class="fcircle {j.status}" aria-hidden="true">{j.status === 'pending' ? '' : GLYPH[j.status]}</span>
                          <span class="fbody">
                            <span class="fname2">{j.name}</span>
                            {#if j.status === 'failed'}
                              <span class="fsub bad">failed at {fmtDur((j.finished_at ?? $now) - (j.started_at ?? $now))}</span>
                            {:else if j.status === 'pending'}
                              <span class="fsub">pending</span>
                            {:else if j.started_at}
                              <span class="fsub">{fmtDur((j.finished_at ?? $now) - j.started_at)}</span>
                            {/if}
                          </span>
                        </button>
                      {/each}
                    </div>
                  </div>
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
      {:else}
        {@const jobs = stageJobs(view)}
        {@const win = stageWindow(jobs)}
        {@const stWorkers = [...new Set(jobs.filter((j) => j.worker).map((j) => j.worker!))]}
        {@const failedJob = jobs.find((j) => j.status === 'failed')}
        <div class="stage-row">
          <div>
            {#if selJob}
              <div class="section-label">Command log
                <span class="meta">{view} / {selJob.name}</span>
              </div>
              <div class="log-controls">
                <button type="button" class="toggle" aria-pressed={logFollow}
                  onclick={() => { logFollow = !logFollow; if (logFollow && logBody) logBody.scrollTop = logBody.scrollHeight; }}>
                  follow tail
                </button>
                <button type="button" class="toggle" aria-pressed={logWrap} onclick={() => (logWrap = !logWrap)}>wrap lines</button>
                <span class="spacer"></span>
                <button type="button" class="btn" onclick={copyLog}>{copied ? 'Copied' : 'Copy log'}</button>
              </div>
              <div class="term" class:failed={selJob.status === 'failed'}>
                <div class="term-head">
                  <span class="dot" aria-hidden="true"></span>{selJob.worker ?? 'unassigned'}
                  <span class="tjob" title={selJob.name}>{selJob.status === 'failed' ? `${GLYPH.failed} ` : ''}{selJob.name}</span>
                </div>
                <div
                  class="term-body full"
                  class:wrapped={logWrap}
                  bind:this={logBody}
                  onscroll={onLogScroll}
                  role="log"
                  aria-label="Log for {selJob.name}"
                >
                  {#each log as l, i (i)}
                    <div class="lnrow" class:err={l.err} class:ok={l.ok}>
                      <span class="lnum" aria-hidden="true">{i + 1}</span>
                      <span class="ltxt">{l.t || ' '}</span>
                    </div>
                  {:else}
                    <div class="lnrow">
                      <span class="lnum" aria-hidden="true"></span>
                      <span class="ltxt" style="color:var(--term-muted)">queued — no output yet</span>
                    </div>
                  {/each}
                </div>
                <div class="term-foot">
                  <span class="tcmd" title="$ {selJob.command}">$ {selJob.command}</span>
                  {#if selJob.status === 'running'}
                    <span class="tlines streaming">streaming…</span>
                  {:else}
                    <span class="tlines">{log.length} lines</span>
                  {/if}
                </div>
              </div>
            {:else}
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
                      <div class="lnrow" class:err={l.err} class:ok={l.ok}>
                        <span class="lnum" aria-hidden="true">{i + 1}</span>
                        <span class="ltxt">{l.t || ' '}</span>
                      </div>
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
            {/if}
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

      <footer class="foot">
        mode: {MODE} · polling every 3s · ✓ {passed} passed · ✕ {failedCount} failed · ○ {pending} pending
      </footer>
    {:else if !error}
      <div class="empty" style="margin:24px 0">Run #{id} not found.</div>
    {/if}
  </div>
</div>
