<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, MODE } from '../lib/api';
  import { activity, deviceStats, type Activity, type BusyInterval } from '../lib/charts';
  import Topbar from '../lib/components/Topbar.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { now, startPolling } from '../lib/poll';
  import type { Overview, Worker } from '../lib/types';

  const WINDOW_MS = 15 * 60 * 1000;
  const BUCKETS = 36;

  let overview = $state<Overview | null>(null);
  let error = $state('');
  let apiMs = $state(0);

  const stop = startPolling(async () => {
    try {
      const t0 = performance.now();
      overview = await api.overview();
      apiMs = Math.max(1, Math.round(performance.now() - t0));
      error = '';
    } catch (e) {
      error = `Cannot reach the data source (${(e as Error).message}). Retrying on the next poll.`;
    }
  });
  onDestroy(stop);

  const act = $derived(overview ? activity(overview.runs, overview.workers, $now, WINDOW_MS) : null);
  const workers = $derived(overview?.workers ?? []);
  const total = $derived(Math.max(workers.length, 1));

  function busyAt(a: Activity, t: number): number {
    let n = 0;
    for (const list of a.byWorker.values()) {
      if (list.some((iv) => iv.start <= t && t <= iv.end)) n++;
    }
    return n;
  }

  const bars = $derived.by(() => {
    if (!act) return [];
    const step = act.windowMs / BUCKETS;
    return Array.from({ length: BUCKETS }, (_, i) => {
      const mid = act.t0 + (i + 0.5) * step;
      const busy = busyAt(act, mid);
      return { busy, when: Math.round((act.now - mid) / 60000) };
    });
  });
  const anyActivity = $derived(bars.some((b) => b.busy > 0));
  const busyNow = $derived(
    act ? [...act.byWorker.values()].filter((l) => l.some((iv) => iv.status === 'running')).length : 0,
  );
  const offlineCount = $derived(workers.filter((w) => w.status !== 'online').length);
  const queued = $derived.by(() => {
    const activeRun = overview?.runs.find((r) => r.status === 'running');
    return activeRun ? activeRun.jobs.filter((j) => j.status === 'pending').length : 0;
  });

  function runningIv(name: string): BusyInterval | undefined {
    return (act?.byWorker.get(name) ?? []).find((iv) => iv.status === 'running');
  }
  function lastFail(name: string): BusyInterval | undefined {
    return [...(act?.byWorker.get(name) ?? [])].reverse().find((iv) => iv.status === 'failed');
  }
  function segs(name: string) {
    if (!act) return [];
    return (act.byWorker.get(name) ?? []).map((iv) => ({
      iv,
      left: ((iv.start - act.t0) / act.windowMs) * 100,
      width: Math.max(((iv.end - iv.start) / act.windowMs) * 100, 0.8),
    }));
  }
  function stats(name: string) {
    return deviceStats(act?.byWorker.get(name) ?? [], act!);
  }
</script>

<Topbar active="monitor" {overview}>
  <div class="wrap">
    <div class="page-head">
      <h1>Monitor</h1>
      <span class="meta">last 15 minutes</span>
    </div>

    <section class="graph-panel" aria-label="Fleet utilization over the last 15 minutes">
      <div class="graph-head">
        <span class="glabel">Fleet utilization</span>
        <span class="gsub">each bar = workers busy during a ~25s slice</span>
        <span class="gvalue">
          {#if busyNow}
            active {busyNow}/{total}<span class="live-dot" aria-hidden="true"></span>
          {:else}
            active 0/{total} · all idle<span class="live-dot quiet" aria-hidden="true"></span>
          {/if}
        </span>
      </div>
      <div class="fleet-plot">
        <div class="fleet-grid" aria-hidden="true">
          {#each Array.from({ length: total }, (_, i) => i + 1) as k (k)}
            <div class="fgl" style="bottom:{(k / total) * 100}%">
              {#if k === total}<span>all {total} busy</span>{:else if total <= 5}<span>{k}</span>{/if}
            </div>
          {/each}
        </div>
        <div class="fleet-bars" role="img" aria-label="Busy workers per interval">
          {#each bars as b, i (i)}
            <i
              class:zero={!b.busy}
              style="height:{b.busy ? Math.max(8, (b.busy / total) * 100) : 2}%"
              title="{b.busy} of {total} busy · ~{b.when}m ago"
            ></i>
          {/each}
        </div>
        {#if !anyActivity}
          <div class="fleet-idle">
            cluster idle — no jobs in the last 15 minutes ·
            {offlineCount ? `${offlineCount} worker${offlineCount === 1 ? '' : 's'} offline` : 'all workers online'}
          </div>
        {/if}
      </div>
      <div class="graph-axis"><span>15m ago</span><span>10m</span><span>5m</span><span>now</span></div>
    </section>
  </div>
</Topbar>

<main class="wrap">
  {#if error}<div class="err-banner">{error}</div>{/if}

  <div class="section-label">Devices
    <span class="meta">timeline: ✓ passed · ✕ failed · running extends to now</span>
  </div>
  <div class="devices">
    {#if act}
      {#each workers as w (w.name)}
        {@const st = stats(w.name)}
        {@const run = runningIv(w.name)}
        {@const fail = lastFail(w.name)}
        {@const offline = w.status !== 'online'}
        <div class="card wcard" class:offline>
          <div class="wc-head">
            <span class="wname">{w.name}</span>
            {#if offline}<span class="wc-badge off">⚠ offline</span>
            {:else if run}<span class="wc-badge busy">busy</span>
            {:else}<span class="wc-badge idle">idle</span>{/if}
          </div>
          <div class="wc-seen"><span class="dot" aria-hidden="true"></span>last seen {ago(w.last_heartbeat, $now)}</div>

          {#if offline}
            <div class="wc-well offwell">
              no heartbeat for {ago(w.last_heartbeat, $now).replace(' ago', '')} — marked offline by the reaper
            </div>
          {:else if run}
            <div class="wc-well job">
              <span class="wk">current job</span>
              <span class="wv"><a href="#/run/{run.run.id}?job={run.job.id}">{run.job.name}</a></span>
              <span class="wsub">run #{run.run.id} · {run.run.pipeline}</span>
            </div>
          {:else if fail}
            <div class="wc-well failwell">
              <span class="wk">recent failure</span>
              <span class="wv"><a href="#/run/{fail.run.id}?job={fail.job.id}">exit {fail.job.exit_code} in {fail.job.name}</a></span>
              <span class="wsub">run #{fail.run.id} · {ago(fail.end, $now)}</span>
            </div>
          {:else}
            <div class="wc-well standby">standing by — waiting for the coordinator queue…</div>
          {/if}

          <div class="wc-tl-label">timeline · 15m</div>
          <div class="wc-tl" role="img" aria-label="Activity in the last 15 minutes">
            {#each segs(w.name) as s (s.iv.job.id)}
              <i
                class={s.iv.status}
                style="left:{s.left}%;width:{s.width}%"
                title="{s.iv.job.name} · {s.iv.status} · {fmtDur(s.iv.end - s.iv.start)}"
              >{s.width > 6 ? GLYPH[s.iv.status] ?? '' : ''}</i>
            {/each}
          </div>
          <div class="wc-stats">
            <span class="wc-stat"><span class="k">jobs</span><span class="v">{st.jobs}</span></span>
            <span class="wc-stat"><span class="k">pass</span><span class="v">{st.passed}</span></span>
            <span class="wc-stat"><span class="k">fail</span><span class="v" class:bad={st.failed > 0}>{st.failed}</span></span>
            <span class="wc-stat util"><span class="k">util</span><span class="v">{st.util}%</span></span>
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <footer class="foot">api: ok ({apiMs}ms) · mode: {MODE} · queue: {queued} pending · auto-refresh: 3s</footer>
</main>
