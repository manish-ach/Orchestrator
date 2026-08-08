<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api } from '../lib/api';
  import AppShell from '../lib/components/AppShell.svelte';
  import FleetChart from '../lib/components/FleetChart.svelte';
  import FlowCanvas from '../lib/components/FlowCanvas.svelte';
  import Sparkline from '../lib/components/Sparkline.svelte';
  import Strip from '../lib/components/Strip.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { overview as live } from '../lib/live';
  import { now, startPolling } from '../lib/poll';
  import type { Overview, Run, WorkerStatsSeries } from '../lib/types';

  // The control centre answers "is anything on fire, right now". Anything that
  // needs history, a full list or a comparison belongs to the page that owns it,
  // so there are deliberately no "view all" links here.
  const overview = $derived($live);
  let series = $state<WorkerStatsSeries[]>([]);
  let error = $state('');
  let picked = $state<number | null>(null);
  let pickerOpen = $state(false);

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

  const runs = $derived([...(overview?.runs ?? [])].sort((a, b) => b.started_at - a.started_at));
  const workers = $derived(overview?.workers ?? []);
  const allJobs = $derived(runs.flatMap((r) => r.jobs));
  const runDur = (r: Run) => (r.finished_at ?? $now) - r.started_at;

  // ---- KPI tiles --------------------------------------------------------
  const finished = $derived(runs.filter((r) => r.finished_at).slice(0, 20));
  const passed = $derived(finished.filter((r) => r.status === 'passed').length);
  const successPct = $derived(finished.length ? Math.round((passed / finished.length) * 100) : null);
  // running success rate across the window, so the sparkline shows a trend
  // instead of repeating the headline number
  const successTrend = $derived.by(() => {
    const seq = [...finished].reverse();
    return seq.map((_, i) => {
      const upto = seq.slice(0, i + 1);
      return (upto.filter((r) => r.status === 'passed').length / upto.length) * 100;
    });
  });
  const durations = $derived(finished.map(runDur).filter((d) => d > 0));
  const sorted = $derived([...durations].sort((a, b) => a - b));
  const median = $derived(sorted.length ? sorted[Math.floor(sorted.length / 2)] : null);
  const p90 = $derived(sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.9))] : null);
  const runningJobs = $derived(allJobs.filter((j) => j.status === 'running'));
  const startOfDay = $derived(new Date(new Date($now).setHours(0, 0, 0, 0)).getTime());
  const failedToday = $derived(runs.filter((r) => r.status === 'failed' && r.started_at >= startOfDay));

  // ---- live pipeline picker --------------------------------------------
  // Newest running by default, newest finished when the fleet is quiet — the
  // panel should never be blank just because nothing happens to be executing.
  const running = $derived(runs.filter((r) => r.status === 'running'));
  const candidates = $derived([...running, ...runs.filter((r) => r.status !== 'running')].slice(0, 12));
  const current = $derived(candidates.find((r) => r.id === picked) ?? running[0] ?? runs[0] ?? null);

  function statsFor(name: string, key: 'cpu' | 'mem'): number[] {
    return series.find((s) => s.name === name)?.samples.map((x) => x[key]) ?? [];
  }
  function jobOf(w: { job_id: number | null }) {
    if (w.job_id === null) return null;
    for (const r of runs) {
      const j = r.jobs.find((x) => x.id === w.job_id);
      if (j) return { job: j, run: r };
    }
    return null;
  }
  const recent = $derived(runs.slice(0, 4));
</script>

<AppShell active="control" {overview}>
  <div class="page-body">
    {#if error}<div class="err-banner">{error}</div>{/if}

    <div class="kpis">
      <div class="kpi">
        <span class="lbl">Success rate</span>
        <div class="v"><span class="num">{successPct ?? '–'}</span><span class="u">%</span></div>
        <div class="sub">{passed} of {finished.length} recent runs</div>
        <Sparkline values={successTrend} tone="ok" />
      </div>
      <div class="kpi">
        <span class="lbl">Median duration</span>
        <div class="v"><span class="num">{median ? fmtDur(median) : '–'}</span></div>
        <div class="sub">{p90 ? `p90 ${fmtDur(p90)}` : 'no finished runs yet'}</div>
        <Sparkline values={[...durations].reverse()} tone="brand" />
      </div>
      <div class="kpi">
        <span class="lbl">Running now</span>
        <div class="v"><span class="num">{runningJobs.length}</span><span class="u">jobs</span></div>
        <div class="sub">
          {runningJobs.length ? runningJobs.slice(0, 2).map((j) => j.name).join(' · ') : 'cluster idle'}
        </div>
      </div>
      <div class="kpi">
        <span class="lbl">Failed today</span>
        <div class="v"><span class="num">{failedToday.length}</span><span class="u">runs</span></div>
        <div class="sub">
          {failedToday.length
            ? [...new Set(failedToday.map((r) => r.repo))].slice(0, 2).join(' · ')
            : 'nothing failed today'}
        </div>
      </div>
    </div>

    <div class="grid">
      <section class="card flex">
        <div class="chead">
          {#if current}
            <span class="g {current.status}">{GLYPH[current.status] ?? ''}</span>
            <div class="picker">
              <button class="pk-btn" aria-haspopup="listbox" aria-expanded={pickerOpen} onclick={() => (pickerOpen = !pickerOpen)}>
                {current.repo} / {current.pipeline} <span class="id">#{current.id}</span>
              </button>
              {#if pickerOpen}
                <div class="pk-menu">
                  {#each ['running', 'finished'] as group (group)}
                    {@const rows = candidates.filter((r) => (group === 'running' ? r.status === 'running' : r.status !== 'running'))}
                    {#if rows.length}
                      <div class="pk-lbl">{group === 'running' ? 'Running now' : 'Recently finished'}</div>
                      {#each rows as r (r.id)}
                        <button class="pk-opt" class:sel={r.id === current.id} onclick={() => { picked = r.id; pickerOpen = false; }}>
                          <span class="g {r.status}">{GLYPH[r.status] ?? ''}</span>
                          <span class="pk-txt">
                            <span class="nm">{r.repo} / {r.pipeline} #{r.id}</span>
                            <span class="sub">{r.commit?.message ?? '—'}</span>
                          </span>
                          <span class="agecol">{ago(r.started_at, $now)}</span>
                        </button>
                      {/each}
                    {/if}
                  {/each}
                </div>
              {/if}
            </div>
            <span class="m">{current.commit?.message ?? ''}</span>
          {:else}
            <h2>No runs yet</h2>
          {/if}
        </div>

        {#if current}
          <FlowCanvas
            jobs={current.jobs}
            height={286}
            onselect={(j) => (location.hash = `/run/${current.id}?job=${j.id}`)}
          />
          <div class="wells">
            <div class="well">
              <span class="lbl">{current.status === 'running' ? 'Elapsed' : 'Duration'}</span>
              <span class="v">{fmtDur(runDur(current))}</span>
            </div>
            <div class="well">
              <span class="lbl">Jobs</span>
              <span class="v">
                {current.jobs.filter((j) => j.status === 'passed').length}<small
                  >/{current.jobs.length} done · {current.jobs.filter((j) => j.status === 'failed').length} failed</small>
              </span>
            </div>
            <div class="well">
              <span class="lbl">Workers</span>
              <span class="v">
                {new Set(current.jobs.map((j) => j.worker).filter(Boolean)).size}<small>&nbsp;on this run</small>
              </span>
            </div>
            <div class="well">
              <span class="lbl">Triggered by</span>
              <span class="v sm">
                {current.trigger === 'webhook' ? 'push' : current.trigger}<small
                  >{current.commit ? ` · ${current.commit.author}` : ''}</small>
              </span>
            </div>
          </div>
        {/if}
      </section>

      <section class="card">
        <div class="chead">
          <h2>Worker cluster</h2>
          <span class="m">
            {workers.filter((w) => w.status === 'online').length} online · {workers.filter((w) => w.status !== 'online').length} offline
          </span>
        </div>
        <div class="scroll">
          <table>
            <thead>
              <tr><th>Worker</th><th>Status</th><th>CPU</th><th>Memory</th><th>Current job</th></tr>
            </thead>
            <tbody>
              {#each workers as w (w.id ?? w.name)}
                {@const busy = jobOf(w)}
                <tr class:offrow={w.status !== 'online'}>
                  <td>
                    <a class="wname" href="#/workers/{encodeURIComponent(w.name)}">{w.name}</a>
                    <span class="whost">{(w.tags ?? []).join(' · ') || 'no tags'}</span>
                  </td>
                  <td>
                    <span class="pill" class:on={w.status === 'online' && !busy} class:busy={!!busy} class:off={w.status !== 'online'}>
                      <i></i>{w.status !== 'online' ? 'Offline' : busy ? 'Busy' : 'Idle'}
                    </span>
                  </td>
                  <td>
                    <span class="meter">
                      <span class="spark-slot"><Sparkline values={statsFor(w.name, 'cpu')} tone="brand" width={40} height={18} /></span>
                      <span class="num">{w.stats ? `${Math.round(w.stats.cpu_pct)}%` : '—'}</span>
                    </span>
                  </td>
                  <td>
                    <span class="meter">
                      <span class="spark-slot"><Sparkline values={statsFor(w.name, 'mem')} tone="brand" width={40} height={18} /></span>
                      <span class="num">{w.stats ? `${Math.round(w.stats.mem_pct)}%` : '—'}</span>
                    </span>
                  </td>
                  <td class="job">
                    {#if busy}
                      <a href="#/run/{busy.run.id}?job={busy.job.id}">{busy.job.name}</a>
                      <span class="s">{busy.run.pipeline} #{busy.run.id}</span>
                    {:else if w.status !== 'online'}
                      <span class="dead">no heartbeat for {ago(w.last_heartbeat, $now).replace(' ago', '')}</span>
                    {:else}
                      <span class="idle">standing by</span>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </section>
    </div>

    <div class="grid b">
      <section class="card">
        <div class="chead"><h2>Recent runs</h2><span class="m">latest activity</span></div>
        <div class="scroll">
          {#each recent as r (r.id)}
            <a class="run" href="#/run/{r.id}">
              <span class="g {r.status}">{GLYPH[r.status] ?? ''}</span>
              <span class="rbody">
                <span class="t">{r.commit?.message ?? `Run #${r.id}`}</span>
                <span class="s">{r.repo} #{r.id} · {r.commit?.sha ?? '—'} · {r.commit?.author ?? 'unknown'}</span>
              </span>
              <span class="rail2">
                <Strip jobs={r.jobs} />
                <span class="rt">
                  <span class="b">{fmtDur(runDur(r))}</span>
                  <span class="a">{r.status === 'running' ? 'running' : ago(r.started_at, $now)}</span>
                </span>
              </span>
            </a>
          {:else}
            <div class="empty">No runs yet — push to a registered repo and its pipeline appears here.</div>
          {/each}
        </div>
      </section>

      <section class="card flex">
        <div class="chead">
          <h2>Fleet utilization</h2>
          <div class="right">
            <div class="legend">
              <span><i style="background:oklch(0.66 0.09 112)"></i>in flight</span>
              <span><i style="background:oklch(0.935 0.004 110)"></i>headroom</span>
              <span><i style="background:oklch(0.74 0.15 62)"></i>queued</span>
              <span><i class="rail" style="background:oklch(0.74 0.15 62)"></i>at capacity</span>
            </div>
          </div>
        </div>
        <div class="chartbox">
          <FleetChart jobs={allJobs} {workers} now={$now} />
        </div>
      </section>
    </div>
  </div>
</AppShell>
