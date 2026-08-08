<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api } from '../lib/api';
  import { activity, deviceStats, drawStatChart, type Activity } from '../lib/charts';
  import AppShell from '../lib/components/AppShell.svelte';
  import Calendar from '../lib/components/Calendar.svelte';
  import FacetDropdown from '../lib/components/FacetDropdown.svelte';
  import OsLogo from '../lib/components/OsLogo.svelte';
  import { ago, fmtDur, fmtUptime, GLYPH } from '../lib/format';
  import { overview as live } from '../lib/live';
  import { now, startPolling } from '../lib/poll';
  import { route } from '../lib/router';
  import type { Overview, Worker, WorkerActivity, WorkerStatsSeries } from '../lib/types';

  // Two levels behind one nav item:
  //   #/workers          the fleet, as a list — one row per machine
  //   #/workers/<name>   that machine: what it is, and what it has run
  //
  // The split follows the same rule as the rest of the site. The list answers
  // "is the fleet healthy right now", so every column on it is live state. The
  // device page answers "what is this machine and what has it done", so the
  // hardware and the history live there and nowhere else — putting a CPU chart
  // per worker on the list would say the same thing five times over.

  const WINDOW_MS = 15 * 60 * 1000;

  const overview = $derived($live);
  let series = $state<WorkerStatsSeries[]>([]);
  let act = $state<WorkerActivity | null>(null);
  let error = $state('');

  let query = $state('');
  let status = $state<'all' | 'busy' | 'idle' | 'offline'>('all');
  let platform = $state('all');
  let tag = $state('all');

  const name = $derived($route.path[1] ? decodeURIComponent($route.path[1]) : null);

  const stop = startPolling(async () => {
    try {
      const s = await api.workerStats();
      series = s;
      error = '';
    } catch (e) {
      error = `Cannot reach the data source (${(e as Error).message}). Retrying on the next poll.`;
    }
  });
  onDestroy(stop);

  // History is a per-device concern and a full year of data, so it is fetched
  // only when a device page is open — and only when the name changes, not on
  // every 3s poll of live state.
  $effect(() => {
    const n = name;
    if (!n) {
      act = null;
      return;
    }
    // a device page opened from halfway down another one must not start
    // halfway down itself
    document.querySelector('.page-body.scrolly')?.scrollTo(0, 0);
    let stale = false;
    api
      .workerActivity(n)
      .then((a) => {
        if (!stale) act = a;
      })
      .catch(() => {});
    return () => {
      stale = true;
    };
  });

  const workers = $derived(overview?.workers ?? []);
  const acts = $derived<Activity | null>(overview ? activity(overview.runs, workers, $now, WINDOW_MS) : null);

  const runningJob = (w: Worker) =>
    overview?.runs.flatMap((r) => r.jobs.map((j) => ({ r, j }))).find(({ j }) => j.id === w.job_id) ?? null;

  function state_(w: Worker): 'busy' | 'idle' | 'offline' {
    if (w.status !== 'online') return 'offline';
    return w.job_id !== null ? 'busy' : 'idle';
  }
  const platformOf = (w: Worker) => (w.device ? `${w.device.os_id}/${w.device.arch}` : 'unknown');
  const latest = (w: Worker) => series.find((s) => s.name === w.name)?.samples.at(-1) ?? null;

  // ---- list --------------------------------------------------------------
  const counts = $derived({
    all: workers.length,
    busy: workers.filter((w) => state_(w) === 'busy').length,
    idle: workers.filter((w) => state_(w) === 'idle').length,
    offline: workers.filter((w) => state_(w) === 'offline').length,
  });
  const platformOptions = $derived(['all', ...new Set(workers.map(platformOf))]);
  const tagOptions = $derived(['all', ...new Set(workers.flatMap((w) => w.tags ?? []))]);

  const listed = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return workers.filter((w) => {
      if (status !== 'all' && state_(w) !== status) return false;
      if (platform !== 'all' && platformOf(w) !== platform) return false;
      if (tag !== 'all' && !(w.tags ?? []).includes(tag)) return false;
      if (!q) return true;
      return [w.name, w.device?.host, w.device?.os, w.device?.cpu, platformOf(w), ...(w.tags ?? [])]
        .filter(Boolean)
        .join(' ')
        .toLowerCase()
        .includes(q);
    });
  });

  const online = $derived(counts.all - counts.offline);
  const fleetJobs = $derived.by(() => {
    if (!acts) return { jobs: 0, passed: 0, failed: 0 };
    let jobs = 0,
      passed = 0,
      failed = 0;
    for (const l of acts.byWorker.values()) {
      const d = deviceStats(l, acts);
      jobs += d.jobs;
      passed += d.passed;
      failed += d.failed;
    }
    return { jobs, passed, failed };
  });
  // Share of online capacity that is executing right now. Offline machines are
  // excluded from the denominator — a fleet reading 60% busy because two boxes
  // are dead is describing an outage, not utilisation.
  const load = $derived(online ? Math.round((counts.busy / online) * 100) : 0);

  /** Tags no online worker can satisfy — jobs asking for them will never place. */
  const orphanTags = $derived.by(() => {
    const covered = new Set(workers.filter((w) => w.status === 'online').flatMap((w) => w.tags ?? []));
    const wanted = new Set(overview?.runs.flatMap((r) => r.jobs.flatMap((j) => j.tags ?? [])) ?? []);
    return [...wanted].filter((t) => !covered.has(t));
  });

  const open = (w: Worker) => (location.hash = `/workers/${encodeURIComponent(w.name)}`);

  // ---- device ------------------------------------------------------------
  const dev = $derived(workers.find((w) => w.name === name) ?? null);
  const devSamples = $derived(series.find((s) => s.name === name)?.samples ?? []);
  const devStats = $derived(acts && name ? deviceStats(acts.byWorker.get(name) ?? [], acts) : null);

  let cpuCanvas = $state<HTMLCanvasElement | null>(null);
  let memCanvas = $state<HTMLCanvasElement | null>(null);
  $effect(() => {
    const t1 = $now;
    const t0 = t1 - WINDOW_MS;
    const s = devSamples;
    if (cpuCanvas) {
      drawStatChart(cpuCanvas, s, {
        t0,
        t1,
        metric: 'cpu',
        stroke: 'oklch(0.50 0.105 112)',
        fill: 'oklch(0.50 0.105 112 / 0.14)',
      });
    }
    if (memCanvas) {
      drawStatChart(memCanvas, s, {
        t0,
        t1,
        metric: 'mem',
        stroke: 'oklch(0.55 0.09 245)',
        fill: 'oklch(0.55 0.09 245 / 0.13)',
      });
    }
  });

  /** The fastfetch-style block. Only rows the agent actually reported. */
  const spec = $derived.by(() => {
    const d = dev?.device;
    if (!d) return [] as { k: string; v: string }[];
    const gb = (mb: number) => `${(mb / 1024).toFixed(2)} GiB`;
    const rows: { k: string; v: string | null }[] = [
      { k: 'OS', v: d.os },
      { k: 'Host', v: d.host },
      { k: 'Kernel', v: d.kernel },
      { k: 'Arch', v: d.arch },
      {
        k: 'CPU',
        v: d.cpu
          ? `${d.cpu} (${d.cpu_cores}${d.cpu_physical && d.cpu_physical !== d.cpu_cores ? ` / ${d.cpu_physical}P` : ''})${
              d.cpu_mhz ? ` @ ${(d.cpu_mhz / 1000).toFixed(2)} GHz` : ''
            }`
          : null,
      },
      {
        k: 'Memory',
        v: d.mem_total_mb
          ? `${latest(dev!) ? `${gb((latest(dev!)!.mem / 100) * d.mem_total_mb)} / ` : ''}${gb(d.mem_total_mb)}${
              latest(dev!) ? ` (${Math.round(latest(dev!)!.mem)}%)` : ''
            }`
          : null,
      },
      {
        k: 'Disk',
        v: d.disk_total_gb
          ? `${d.disk_total_gb - d.disk_free_gb} / ${d.disk_total_gb} GiB (${Math.round(
              ((d.disk_total_gb - d.disk_free_gb) / d.disk_total_gb) * 100,
            )}%)`
          : null,
      },
      { k: 'Shell', v: d.shell },
      { k: 'Agent', v: `orchestrator ${d.agent}` },
      { k: 'Executor', v: d.executor },
    ];
    return rows.map((r) => ({ k: r.k, v: r.v ?? 'unknown' }));
  });

  const diskPct = $derived.by(() => {
    const d = dev?.device;
    if (!d?.disk_total_gb) return null;
    return Math.round(((d.disk_total_gb - d.disk_free_gb) / d.disk_total_gb) * 100);
  });

  /** Outcomes grouped by job name — what work this machine is actually given. */
  const byJob = $derived.by(() => {
    const m = new Map<string, { name: string; runs: number; failed: number }>();
    for (const j of act?.recent ?? []) {
      const e = m.get(j.name) ?? { name: j.name, runs: 0, failed: 0 };
      e.runs++;
      if (j.status === 'failed') e.failed++;
      m.set(j.name, e);
    }
    return [...m.values()].sort((a, b) => b.runs - a.runs);
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && name && document.activeElement?.tagName !== 'INPUT') location.hash = '/workers';
  }
</script>

<svelte:window onkeydown={onKey} />

<AppShell active="workers" {overview}>
  {#if name}
    <!-- ===================== one device ===================== -->
    <div class="page-body scrolly">
      <div class="crumb">
        <button class="backb" title="Back to all workers" aria-label="Back to all workers" onclick={() => (location.hash = '/workers')}>←</button>
        <button class="crumb-link" onclick={() => (location.hash = '/workers')}>Workers</button>
        <span class="sep">/</span>
        <span class="cur">{name}</span>
      </div>

      {#if !dev}
        <div class="emptyq">
          <b>{overview ? `No worker named ${name}` : 'Loading…'}</b>
          {overview ? 'It may have been pruned after 24 hours of silence.' : ''}
        </div>
      {:else}
        {@const st = state_(dev)}
        {@const job = runningJob(dev)}
        <header class="devhead">
          <span class="devmark"><OsLogo id={dev.device?.os_id ?? ''} size={30} /></span>
          <div class="devid">
            <h1>{dev.name}</h1>
            <p class="mono">
              {platformOf(dev)}
              {#if dev.device?.host}<span class="d">·</span>{dev.device.host}{/if}
              {#if dev.registered_at}<span class="d">·</span>up {fmtUptime(dev.registered_at, $now)}{/if}
            </p>
          </div>
          <div class="devbadges">
            {#each dev.tags ?? [] as t (t)}<span class="tagchip">{t}</span>{/each}
            <span class="wstate {st}">{st}</span>
          </div>
        </header>

        {#if st === 'offline'}
          <div class="err-banner">
            No heartbeat for {ago(dev.last_heartbeat, $now).replace(' ago', '')} — the reaper marked this machine
            offline, so nothing new will be scheduled on it. Live readings below stop at its last heartbeat.
          </div>
        {/if}

        <!-- spec block and calendar share the row; both scroll inside
             themselves so neither can push the page down -->
        <div class="devtop">
          <section class="spec">
            <div class="spechead">
              <OsLogo id={dev.device?.os_id ?? ''} size={26} />
              <span class="mono"><b>{dev.name}</b>@{dev.device?.host ?? 'unknown'}</span>
            </div>
            <div class="specscroll">
              {#if spec.length}
                <dl class="specgrid">
                  {#each spec as r (r.k)}
                    <dt>{r.k}</dt>
                    <dd class:na={r.v === 'unknown'}>{r.v}</dd>
                  {/each}
                </dl>
                <div class="specfoot">
                  <span><i>Heartbeat</i> {ago(dev.last_heartbeat, $now)}</span>
                  <span><i>Tags</i> {dev.tags?.length ? dev.tags.join(', ') : 'none'}</span>
                </div>
              {:else}
                <p class="specna">
                  This worker did not report a device profile. Agents before 0.1.0 do not send one — restart it to fill
                  this in.
                </p>
              {/if}
            </div>
          </section>

          <section class="card calcard">
            <div class="chead">
              <h2>Days this machine ran jobs</h2>
              <span class="cmeta">last 12 months</span>
            </div>
            <Calendar days={act?.calendar ?? []} unit="jobs" />
          </section>
        </div>

        <div class="wells five">
          <div class="well">
            <span class="lbl">Jobs</span><span class="v">{act?.total_jobs ?? '—'}<small>&nbsp;all time</small></span>
          </div>
          <div class="well">
            <span class="lbl">Pass rate</span>
            <span class="v"
              >{act && act.total_jobs ? Math.round((act.passed / act.total_jobs) * 100) : '–'}<small
                >% · {act?.failed ?? 0} failed</small
              ></span
            >
          </div>
          <div class="well">
            <span class="lbl">Median job</span><span class="v">{act?.median_ms ? fmtDur(act.median_ms) : '—'}</span>
          </div>
          <div class="well">
            <span class="lbl">Busy · 15m</span><span class="v">{devStats?.util ?? 0}<small>%</small></span>
          </div>
          <div class="well">
            <span class="lbl">Disk used</span>
            <span class="v"
              >{diskPct ?? '—'}{#if diskPct !== null}<small
                  >% · {dev.device?.disk_free_gb} GiB free</small
                >{/if}</span
            >
          </div>
        </div>

        <div class="devcharts">
          <section class="card">
            <div class="chead">
              <h2>CPU</h2>
              <span class="cmeta">
                {#if devSamples.length}
                  now <b>{Math.round(devSamples.at(-1)!.cpu)}%</b> · last 15 min
                {:else}
                  no samples · heartbeat carries these
                {/if}
              </span>
            </div>
            <div class="plot"><canvas bind:this={cpuCanvas}></canvas></div>
          </section>
          <section class="card">
            <div class="chead">
              <h2>Memory</h2>
              <span class="cmeta">
                {#if devSamples.length}
                  now <b>{Math.round(devSamples.at(-1)!.mem)}%</b> · last 15 min
                {:else}
                  no samples · heartbeat carries these
                {/if}
              </span>
            </div>
            <div class="plot"><canvas bind:this={memCanvas}></canvas></div>
          </section>
        </div>

        <section class="card">
          <div class="chead">
            <h2>Job activity</h2>
            <span class="cmeta">last 15 min · ✓ passed · ✕ failed · running extends to now</span>
          </div>
          <div class="devtl" role="img" aria-label="Jobs executed in the last 15 minutes">
            {#each acts?.byWorker.get(dev.name) ?? [] as iv (iv.job.id)}
              {@const left = ((iv.start - acts!.t0) / acts!.windowMs) * 100}
              {@const width = Math.max(((iv.end - iv.start) / acts!.windowMs) * 100, 0.8)}
              <a
                class="tlseg {iv.status}"
                href="#/run/{iv.run.id}?job={iv.job.id}"
                style="left:{left}%;width:{width}%"
                title="{iv.job.name} · {iv.status} · {fmtDur(iv.end - iv.start)}"
              >{width > 6 ? (GLYPH[iv.status] ?? '') : ''}</a
              >
            {:else}
              <span class="tlnone">nothing ran on this machine in the last 15 minutes</span>
            {/each}
          </div>
          <div class="tlaxis"><span>15m ago</span><span>now</span></div>
        </section>

        <section class="card">
          <div class="chead">
            <h2>What this machine runs</h2>
            <span class="cmeta">by job · last {act?.recent.length ?? 0} executions</span>
          </div>
          <div class="scroll jobsplit">
            {#each byJob as j (j.name)}
              {@const pass = Math.round(((j.runs - j.failed) / j.runs) * 100)}
              <div class="jrow">
                <span class="jname">{j.name}</span>
                <span class="jbar"><i class="ok" style="width:{pass}%"></i><i class="bad" style="width:{100 - pass}%"></i></span>
                <span class="jn"><b>{j.runs}</b> runs</span>
                <span class="jp" class:bad={pass < 100}>{pass}%</span>
              </div>
            {:else}
              <div class="emptyq">
                <b>No executions recorded</b>
                This machine has registered but has not been given a job yet.
              </div>
            {/each}
          </div>
        </section>

        {#if job}
          <section class="card">
            <div class="chead"><h2>Running now</h2></div>
            <a class="curjob" href="#/run/{job.r.id}?job={job.j.id}">
              <span class="g running">{GLYPH.running}</span>
              <span class="cjb">
                <b>{job.j.name}</b>
                <span class="mono">{job.j.command}</span>
              </span>
              <span class="cjr">{job.r.pipeline} #{job.r.id}</span>
            </a>
          </section>
        {/if}
      {/if}
    </div>
  {:else}
    <!-- ===================== the fleet ===================== -->
    <div class="page-body">
      <div class="scopebar">
        <div class="chipset" role="group" aria-label="Filter by state">
          {#each ['all', 'busy', 'idle', 'offline'] as f (f)}
            <button class="schip" class:on={status === f} onclick={() => (status = f as typeof status)}>
              {f}<span class="n">{counts[f as keyof typeof counts]}</span>
            </button>
          {/each}
        </div>
        <FacetDropdown label="platform" bind:value={platform} options={platformOptions} />
        <FacetDropdown label="tag" bind:value={tag} options={tagOptions} />
        <div class="rsearch">
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6">
            <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5 14 14" />
          </svg>
          <input bind:value={query} type="search" placeholder="worker, host, cpu, tag…" aria-label="Search workers" />
        </div>
      </div>

      {#if error}<div class="err-banner">{error}</div>{/if}

      <div class="wells five">
        <div class="well"><span class="lbl">Capacity</span><span class="v">{online}<small>&nbsp;of {counts.all} online</small></span></div>
        <div class="well"><span class="lbl">Busy now</span><span class="v">{counts.busy}<small>&nbsp;· {counts.idle} idle</small></span></div>
        <div class="well">
          <span class="lbl">Jobs · 15m</span>
          <span class="v">{fleetJobs.jobs}<small>&nbsp;{fleetJobs.passed} passed · {fleetJobs.failed} failed</small></span>
        </div>
        <div class="well"><span class="lbl">Fleet load</span><span class="v">{load}<small>% of online capacity</small></span></div>
        <div class="well" class:warnwell={orphanTags.length}>
          <span class="lbl">Unschedulable</span>
          <span class="v sm">
            {#if orphanTags.length}
              {orphanTags.length} tag{orphanTags.length === 1 ? '' : 's'}<small
                >&nbsp;{orphanTags.join(', ')} — no online worker</small
              >
            {:else}
              none<small>&nbsp;every required tag is covered</small>
            {/if}
          </span>
        </div>
      </div>

      <section class="card">
        <div class="scroll">
          {#each listed as w (w.id ?? w.name)}
            {@const st = state_(w)}
            {@const job = runningJob(w)}
            {@const s = latest(w)}
            <div class="frow" role="button" tabindex="0" onclick={() => open(w)} onkeydown={(e) => e.key === 'Enter' && open(w)}>
              <span class="fmark"><OsLogo id={w.device?.os_id ?? ''} size={20} /></span>
              <span class="fid">
                <span class="fnm">{w.name}</span>
                <span class="fsub mono">
                  {platformOf(w)}
                  {#if w.device?.host}<span class="d">·</span>{w.device.host}{/if}
                  {#if w.registered_at}<span class="d">·</span>up {fmtUptime(w.registered_at, $now)}{/if}
                </span>
              </span>

              <span class="wstate {st}">{st}</span>

              <span class="fjob">
                {#if st === 'offline'}
                  <span class="ffail">no heartbeat for {ago(w.last_heartbeat, $now).replace(' ago', '')}</span>
                {:else if job}
                  <b>{job.j.name}</b>
                  <span class="fsub">{job.r.pipeline} #{job.r.id}</span>
                {:else}
                  <span class="fidle">standing by · {ago(w.last_heartbeat, $now)}</span>
                {/if}
              </span>

              <!-- live readings, not history: history is on the device page -->
              <span class="fmeter">
                {#if s}
                  <span class="mv">{Math.round(s.cpu)}%</span>
                  <span class="mtrack"><i style="width:{Math.min(100, s.cpu)}%"></i></span>
                  <span class="mk">cpu</span>
                {:else}
                  <span class="mv na">—</span>
                {/if}
              </span>
              <span class="fmeter">
                {#if s}
                  <span class="mv">{Math.round(s.mem)}%</span>
                  <span class="mtrack"><i class="mem" style="width:{Math.min(100, s.mem)}%"></i></span>
                  <span class="mk">mem</span>
                {:else}
                  <span class="mv na">—</span>
                {/if}
              </span>

              <span class="fchev" aria-hidden="true">›</span>
            </div>
          {:else}
            <div class="emptyq">
              <b>{workers.length ? 'No workers match this filter' : 'No workers registered'}</b>
              {#if workers.length}
                Try clearing the search, or pick a different state.
              {:else}
                Start one with <code>orchestrator worker --name w1 --coordinator http://&lt;host&gt;:8080</code>.
              {/if}
            </div>
          {/each}
        </div>
      </section>
    </div>
  {/if}
</AppShell>
