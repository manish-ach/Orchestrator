<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api } from '../lib/api';
  import AppShell from '../lib/components/AppShell.svelte';
  import FlowCanvas from '../lib/components/FlowCanvas.svelte';
  import OsLogo from '../lib/components/OsLogo.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { classify, parseSteps, type LineKind } from '../lib/logsteps';
  import { streamJobLog, type LogStream } from '../lib/logstream';
  import { overview as live } from '../lib/live';
  import { now, startPolling } from '../lib/poll';
  import { highlightYaml } from '../lib/yamlhl';
  import type { Job, JobStatus, Overview, Run } from '../lib/types';

  // One run, in three panes.
  //
  //   left   what the pipeline is — its file, its stages, the command each job
  //          runs. Static structure, always visible, so you never lose your
  //          place while a log scrolls.
  //   centre what is happening — the DAG when nothing is selected, that job's
  //          live log when something is.
  //   right  the selected job's facts, and its siblings, so moving between jobs
  //          in a stage does not mean going back to the tree.
  //
  // Logs stream rather than poll: a stage's output has to appear while the
  // stage runs, not after it. See lib/logstream.ts.

  let { id, initialJob = null }: { id: string; initialJob?: string | null } = $props();

  let run = $state<Run | null>(null);
  const overview = $derived($live);
  let error = $state('');

  /** 'flow' | 'yaml' | a job id */
  let sel = $state<'flow' | 'yaml' | number>('flow');
  // svelte-ignore state_referenced_locally — the deep link is consumed once, on purpose
  let deepLink = initialJob !== null ? Number(initialJob) : null;

  let collapsed = $state<Set<string>>(new Set());
  let follow = $state(true);
  let wrap = $state(false);
  let copied = $state(false);

  const stop = startPolling(async () => {
    try {
      const r = await api.run(id);
      run = r;
      error = '';
      if (r && deepLink !== null) {
        const job = r.jobs.find((j) => j.id === deepLink);
        deepLink = null;
        if (job) sel = job.id;
      }
    } catch (e) {
      error = `Cannot reach the data source (${(e as Error).message}). Retrying on the next poll.`;
    }
  });
  onDestroy(stop);

  // ---- shape -------------------------------------------------------------
  const jobs = $derived(run?.jobs ?? []);
  const stages = $derived([...new Set(jobs.map((j) => j.stage))]);
  const stageJobs = (s: string) => jobs.filter((j) => j.stage === s);

  function stageStatus(list: Job[]): JobStatus {
    if (list.some((j) => j.status === 'failed')) return 'failed';
    if (list.some((j) => j.status === 'running')) return 'running';
    if (list.length && list.every((j) => j.status === 'passed')) return 'passed';
    if (list.some((j) => j.status === 'passed')) return 'running';
    return 'pending';
  }

  const jobDur = (j: Job) => (j.started_at ? fmtDur((j.finished_at ?? $now) - j.started_at) : null);
  const elapsed = $derived(run ? (run.finished_at ?? $now) - run.started_at : 0);

  const done = $derived(jobs.filter((j) => j.status === 'passed' || j.status === 'failed').length);
  const failed = $derived(jobs.filter((j) => j.status === 'failed').length);
  const stagesDone = $derived(stages.filter((s) => stageStatus(stageJobs(s)) === 'passed').length);
  const runWorkerNames = $derived([...new Set(jobs.map((j) => j.worker).filter(Boolean) as string[])]);
  const runWorkers = $derived((overview?.workers ?? []).filter((w) => runWorkerNames.includes(w.name)));
  // A worker that ran a job here but has since been pruned from the registry
  // still did the work, so it is named rather than silently dropped.
  const missingWorkers = $derived(runWorkerNames.filter((n) => !runWorkers.some((w) => w.name === n)));

  const selJob = $derived(typeof sel === 'number' ? (jobs.find((j) => j.id === sel) ?? null) : null);
  const selSiblings = $derived(selJob ? stageJobs(selJob.stage) : []);

  function pick(target: 'flow' | 'yaml' | number) {
    sel = target;
    follow = true;
  }

  function toggleStage(s: string) {
    const next = new Set(collapsed);
    if (next.has(s)) next.delete(s);
    else next.add(s);
    collapsed = next;
  }

  // ---- live log ----------------------------------------------------------
  let logText = $state('');
  let streaming = $state(false);
  let logBody = $state<HTMLElement | null>(null);

  // The ids are derived as primitives ON PURPOSE. Reading `run?.id` inside the
  // effect would subscribe it to `run` itself, and the 3s poll replaces that
  // object every time — so the effect tore down, cleared logText and reopened
  // the stream three times a minute, which is exactly what made the terminal
  // flash. A derived primitive only notifies when the number actually changes.
  const streamRunId = $derived(run?.id ?? null);
  const streamJobId = $derived(typeof sel === 'number' ? sel : null);

  $effect(() => {
    const r = streamRunId;
    const j = streamJobId;
    if (r === null || j === null) {
      logText = '';
      streaming = false;
      return;
    }
    logText = '';
    streaming = true;
    let s: LogStream | null = streamJobLog(r, j, (chunk) => (logText += chunk), () => (streaming = false));
    return () => {
      s?.close();
      s = null;
    };
  });

  // Autoscroll only while following. Reading `logText` is what makes this run
  // on every chunk; scrolling on a timer would fight the user's own scrolling.
  $effect(() => {
    void logText;
    if (follow && logBody) logBody.scrollTop = logBody.scrollHeight;
  });

  function onLogScroll() {
    if (!logBody || !follow) return;
    const atBottom = logBody.scrollHeight - logBody.scrollTop - logBody.clientHeight < 24;
    if (!atBottom) follow = false;
  }

  const lines = $derived(logText ? logText.split('\n') : []);
  const steps = $derived(parseSteps(lines));
  const hasSteps = $derived(steps.some((s) => s.cmd !== null));
  let closedSteps = $state<Set<number>>(new Set());
  function toggleStep(i: number) {
    const next = new Set(closedSteps);
    if (next.has(i)) next.delete(i);
    else next.add(i);
    closedSteps = next;
  }
  const kind = (t: string): LineKind => classify(t);

  async function copyLog() {
    await navigator.clipboard.writeText(logText);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  // ---- pipeline file -----------------------------------------------------
  let yaml = $state<{ file: string; content: string } | null>(null);
  let yamlError = $state('');
  const repoName = $derived(run?.repo ?? null);
  const pipeFile = $derived(run && /\.ya?ml$/.test(run.pipeline_file) ? run.pipeline_file : null);
  const yamlLines = $derived(yaml ? highlightYaml(yaml.content) : []);

  $effect(() => {
    const repo = repoName;
    const file = pipeFile;
    if (sel !== 'yaml' || !repo) return;
    yaml = null;
    yamlError = '';
    let cancelled = false;
    api.pipelineFile(repo, file ?? undefined).then(
      (d) => !cancelled && (yaml = d),
      (e) => !cancelled && (yamlError = (e as Error).message),
    );
    return () => {
      cancelled = true;
    };
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && document.activeElement?.tagName !== 'INPUT') {
      if (sel !== 'flow') sel = 'flow';
      else location.hash = '/runs';
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<AppShell active="runs" {overview}>
  <div class="page-body runpage">
    <div class="crumb">
      <button class="backb" title="Back to runs" aria-label="Back to runs" onclick={() => (location.hash = '/runs')}
        >←</button
      >
      <button class="crumb-link" onclick={() => (location.hash = '/runs')}>Runs</button>
      {#if run}
        <span class="sep">/</span>
        <button class="crumb-link" onclick={() => (location.hash = `/repos/${encodeURIComponent(run!.repo)}`)}>
          {run.repo}
        </button>
        <span class="sep">/</span>
        <button
          class="crumb-link"
          onclick={() => (location.hash = `/repos/${encodeURIComponent(run!.repo)}/${encodeURIComponent(run!.pipeline)}`)}
        >
          {run.pipeline}
        </button>
        <span class="sep">/</span>
        <span class="cur">#{run.id}</span>
      {/if}
    </div>

    {#if error}<div class="err-banner">{error}</div>{/if}

    {#if !run}
      <div class="emptyq">
        <b>{error ? 'Run unavailable' : 'Loading run…'}</b>
        {error ? '' : `Fetching #${id}.`}
      </div>
    {:else}
      <header class="runhead">
        <div class="runid">
          <h1>{run.commit?.message ?? `Run #${run.id}`}</h1>
          <p class="mono">
            {#if run.commit}<span class="sha">{run.commit.sha}</span><span class="d">·</span>{/if}
            {run.trigger === 'webhook'
              ? `pushed by ${run.commit?.author ?? 'unknown'}`
              : run.trigger === 'schedule'
                ? 'scheduled'
                : `run by ${run.commit?.author ?? 'unknown'}`}
            <span class="d">·</span>{run.pipeline}
            {#if run.branch}<span class="d">·</span><span class="branch">⎇ {run.branch}</span>{/if}
          </p>
        </div>
        <div class="runstatus">
          <span class="runpill {run.status}"><i class="g {run.status}">{GLYPH[run.status] ?? ''}</i>{run.status}</span>
          <span class="relapsed mono">{fmtDur(elapsed)} elapsed</span>
        </div>
      </header>

      <div class="wells five">
        <div class="well"><span class="lbl">Elapsed</span><span class="v">{fmtDur(elapsed)}</span></div>
        <div class="well">
          <span class="lbl">Jobs</span>
          <span class="v">{done}<small>&nbsp;/{jobs.length} done · {failed} failed</small></span>
        </div>
        <div class="well">
          <span class="lbl">Stages</span><span class="v">{stagesDone}<small>&nbsp;/{stages.length} complete</small></span>
        </div>
        <div class="well">
          <span class="lbl">Workers</span><span class="v">{runWorkerNames.length}<small>&nbsp;took part</small></span>
        </div>
        <div class="well">
          <span class="lbl">Triggered</span>
          <span class="v sm">{run.trigger}<small>&nbsp;· {run.commit?.author ?? 'unknown'}</small></span>
        </div>
      </div>

      <div class="runbody" class:withside={selJob !== null}>
        <!-- ---- left: what the pipeline is ---- -->
        <nav class="railcard">
          <div class="scroll">
            <button class="railitem" class:on={sel === 'flow'} onclick={() => pick('flow')}>
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
                <rect x="2" y="2" width="5" height="5" rx="1" /><rect x="9" y="2" width="5" height="5" rx="1" />
                <rect x="2" y="9" width="5" height="5" rx="1" /><rect x="9" y="9" width="5" height="5" rx="1" />
              </svg>
              Overview
            </button>
            <button class="railitem" class:on={sel === 'yaml'} onclick={() => pick('yaml')}>
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M3.5 1.5h6l3 3v10h-9z" /><path d="M9.5 1.5v3h3" />
              </svg>
              Pipeline file
              <span class="railmeta mono">{run.pipeline_file.split('/').pop()}</span>
            </button>

            <p class="raillbl">Stages</p>
            {#each stages as s (s)}
              {@const list = stageJobs(s)}
              {@const st = stageStatus(list)}
              {@const ok = list.filter((j) => j.status === 'passed').length}
              <button class="stagehead" onclick={() => toggleStage(s)} aria-expanded={!collapsed.has(s)}>
                <span class="chev" class:closed={collapsed.has(s)}>▾</span>
                <span class="g {st}">{GLYPH[st] ?? ''}</span>
                <span class="sn">{s}</span>
                <span class="sc mono">{ok}/{list.length}</span>
              </button>
              {#if !collapsed.has(s)}
                <div class="stagejobs">
                  {#each list as j (j.id)}
                    <button class="jobitem" class:on={sel === j.id} onclick={() => pick(j.id)}>
                      <span class="g {j.status}">{GLYPH[j.status] ?? ''}</span>
                      <span class="jn">{j.name}</span>
                      <span class="jd mono">{jobDur(j) ?? '—'}</span>
                      <!-- the command is the job; hiding it behind a click
                           would make the tree a list of names -->
                      <span class="jc mono">$ {j.command}</span>
                    </button>
                  {/each}
                </div>
              {/if}
            {/each}
          </div>
        </nav>

        <!-- ---- centre ---- -->
        {#if sel === 'flow'}
          <div class="runmain">
            <section class="card">
              <div class="chead">
                <h2>Stage flow</h2>
                <span class="cmeta">drag to pan · ⌘ + scroll to zoom · click a job for its log</span>
              </div>
              <FlowCanvas {jobs} height={340} onselect={(j) => pick(j.id)} />
            </section>

            <section class="card">
              <div class="chead">
                <h2>Workers on this run</h2>
                <span class="cmeta">{runWorkerNames.length} of {overview?.workers.length ?? 0} took part</span>
              </div>
              <div class="wgrid">
                {#each runWorkers as w (w.id ?? w.name)}
                  {@const jr = jobs.find((j) => j.worker === w.name && j.status === 'running')}
                  {@const ranHere = jobs.filter((j) => j.worker === w.name)}
                  <a class="wcard2" href="#/workers/{encodeURIComponent(w.name)}">
                    <span class="wtop">
                      <OsLogo id={w.device?.os_id ?? ''} size={16} />
                      <b>{w.name}</b>
                      <span class="wstate {w.status !== 'online' ? 'offline' : jr ? 'busy' : 'idle'}">
                        {w.status !== 'online' ? 'offline' : jr ? 'busy' : 'idle'}
                      </span>
                    </span>
                    <span class="wmeta mono">
                      {w.device ? `${w.device.os_id}/${w.device.arch}` : 'unknown platform'}
                      {#if w.device?.host}<span class="d">·</span>{w.device.host}{/if}
                    </span>
                    <span class="wdid">
                      {#if jr}
                        running <b>{jr.name}</b>
                      {:else}
                        ran {ranHere.length}
                        {ranHere.length === 1 ? 'job' : 'jobs'} on this run
                      {/if}
                    </span>
                    {#if w.stats}
                      <span class="wgauges">
                        <span class="gg">
                          <span class="gk">cpu</span>
                          <span class="mtrack"><i style="width:{Math.min(100, w.stats.cpu_pct)}%"></i></span>
                          <span class="gv mono">{Math.round(w.stats.cpu_pct)}%</span>
                        </span>
                        <span class="gg">
                          <span class="gk">mem</span>
                          <span class="mtrack"><i class="mem" style="width:{Math.min(100, w.stats.mem_pct)}%"></i></span>
                          <span class="gv mono">{Math.round(w.stats.mem_pct)}%</span>
                        </span>
                      </span>
                    {/if}
                  </a>
                {:else}
                  <p class="none">No job on this run has been placed on a worker yet.</p>
                {/each}
                {#each missingWorkers as n (n)}
                  <div class="wcard2 gone">
                    <span class="wtop"><b>{n}</b><span class="wstate offline">gone</span></span>
                    <span class="wdid">Ran jobs here, but is no longer in the registry.</span>
                  </div>
                {/each}
              </div>
            </section>
          </div>
        {:else if sel === 'yaml'}
          <section class="card runmain">
            <div class="chead">
              <h2>Pipeline file</h2>
              <span class="cmeta">{yaml?.file ?? run.pipeline_file}</span>
            </div>
            <div class="scroll yamlbody">
              {#if yamlError}
                <p class="none">Could not read the pipeline file from the coordinator ({yamlError}).</p>
              {:else if !yaml}
                <p class="none">Reading {run.pipeline_file}…</p>
              {:else}
<!-- One span per token, not {@html}: highlightYaml returns token objects,
                   and interpolating the array stringified it to "[object Object]".
                   Rendering them as elements also means file contents are never
                   treated as markup. -->
                <pre class="yaml">{#each yamlLines as toks, i (i)}<span class="yl"
                      ><i class="ln">{i + 1}</i><span class="yc"
                        >{#each toks as t, k (k)}<span class={t.cls}>{t.text}</span>{/each}</span
                      ></span
                    >{/each}</pre>
              {/if}
            </div>
          </section>
        {:else if selJob}
          <section class="card runmain logcard">
            <div class="loghead">
              <span class="g {selJob.status}">{GLYPH[selJob.status] ?? ''}</span>
              <b>{selJob.name}</b>
              <span class="lmeta mono">
                {selJob.worker ?? 'unassigned'}
                {#if jobDur(selJob)}<span class="d">·</span>{jobDur(selJob)}{/if}
              </span>
              <span class="logbtns">
                {#if hasSteps}
                  <button class="lbtn" onclick={() => (closedSteps = new Set(steps.map((_, i) => i)))}>
                    collapse all
                  </button>
                {/if}
                <button class="lbtn" class:on={wrap} onclick={() => (wrap = !wrap)}>wrap</button>
                <button class="lbtn" class:on={follow} onclick={() => (follow = !follow)}>follow</button>
                <button class="lbtn" onclick={copyLog}>{copied ? 'copied' : 'copy'}</button>
              </span>
            </div>
            <div class="logbody" class:wrap bind:this={logBody} onscroll={onLogScroll}>
              {#if !lines.length}
                <p class="logempty">
                  {#if selJob.status === 'pending'}
                    Queued — no output until a worker picks this up.
                  {:else if !selJob.started_at}
                    Skipped: a job it needed failed, so it never ran.
                  {:else}
                    Waiting for the first output…
                  {/if}
                </p>
              {:else if hasSteps}
                {#each steps as st, i (i)}
                  {#if st.cmd !== null}
                    <button class="steprow" onclick={() => toggleStep(i)} aria-expanded={!closedSteps.has(i)}>
                      <span class="chev" class:closed={closedSteps.has(i)}>▾</span>
                      <span class="stepcmd">$ {st.cmd}</span>
                      <span class="stepn">{st.lines.length} lines</span>
                    </button>
                  {/if}
                  {#if !closedSteps.has(i)}
                    {#each st.lines as t, k (k)}
                      <div class="lline {kind(t)}"><i class="ln">{st.start + k}</i><span>{t}</span></div>
                    {/each}
                  {/if}
                {/each}
              {:else}
                {#each lines as t, i (i)}
                  <div class="lline {kind(t)}"><i class="ln">{i + 1}</i><span>{t}</span></div>
                {/each}
              {/if}
              {#if streaming}
                <div class="lline live"><i class="ln"></i><span class="cursor">▌</span></div>
              {/if}
            </div>
            <div class="logfoot">
              <span class="mono">{lines.length} lines</span>
              <span class="lstream" class:live={streaming}>
                {streaming ? 'streaming live' : 'log complete'}
              </span>
            </div>
          </section>

          <!-- ---- right: the selected job's facts + its siblings ---- -->
          <aside class="runside">
            <section class="card">
              <div class="chead"><h2>Job</h2><span class="cmeta">{selJob.status}</span></div>
              <dl class="kv2">
                <dt>Stage</dt>
                <dd>{selJob.stage}</dd>
                <dt>Worker</dt>
                <dd>
                  {#if selJob.worker}
                    <a href="#/workers/{encodeURIComponent(selJob.worker)}">{selJob.worker}</a>
                  {:else}unassigned{/if}
                </dd>
                <dt>Duration</dt>
                <dd>{jobDur(selJob) ?? '—'}</dd>
                {#if selJob.ready_at && selJob.started_at}
                  <dt>Queue wait</dt>
                  <dd>{fmtDur(selJob.started_at - selJob.ready_at)}</dd>
                {/if}
                <dt>Needs</dt>
                <dd>{selJob.needs?.length ? selJob.needs.join(', ') : 'nothing'}</dd>
                {#if selJob.tags?.length}
                  <dt>Tags</dt>
                  <dd>{selJob.tags.join(', ')}</dd>
                {/if}
                <dt>Exit code</dt>
                <dd>{selJob.exit_code ?? '—'}</dd>
                {#if selJob.requeue_count > 0}
                  <dt>Requeued</dt>
                  <dd>{selJob.requeue_count}× after a worker died</dd>
                {/if}
                {#if selJob.finished_at}
                  <dt>Finished</dt>
                  <dd>{ago(selJob.finished_at, $now)}</dd>
                {/if}
              </dl>
              <div class="cmdbox">
                <span class="cmdlbl">Command</span>
                <code>{selJob.command}</code>
              </div>
            </section>

            <section class="card">
              <div class="chead">
                <h2>Stage: {selJob.stage}</h2>
                <span class="cmeta">{selSiblings.length} {selSiblings.length === 1 ? 'job' : 'jobs'}</span>
              </div>
              <div class="scroll">
                {#each selSiblings as j (j.id)}
                  <button class="sibrow" class:on={j.id === selJob.id} onclick={() => pick(j.id)}>
                    <span class="g {j.status}">{GLYPH[j.status] ?? ''}</span>
                    <span class="jn">{j.name}</span>
                    <span class="jd mono">{jobDur(j) ?? '—'}</span>
                  </button>
                {/each}
              </div>
            </section>
          </aside>
        {/if}
      </div>
    {/if}
  </div>
</AppShell>
