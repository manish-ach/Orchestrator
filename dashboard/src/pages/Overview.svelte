<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, MODE } from '../lib/api';
  import { activity, deviceStats } from '../lib/charts';
  import Avatar from '../lib/components/Avatar.svelte';
  import Strip from '../lib/components/Strip.svelte';
  import Topbar from '../lib/components/Topbar.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { now, startPolling } from '../lib/poll';
  import type { CalendarDay, Overview, Run } from '../lib/types';

  const WINDOW_MS = 15 * 60 * 1000;
  const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

  let overview = $state<Overview | null>(null);
  let calendar = $state<CalendarDay[]>([]);
  let error = $state('');
  let triggering = $state(false);

  const stop = startPolling(async () => {
    try {
      const [o, cal] = await Promise.all([api.overview(), api.calendar()]);
      overview = o;
      calendar = cal;
      error = '';
    } catch (e) {
      error = `Cannot reach the data source (${(e as Error).message}). Retrying on the next poll.`;
    }
  });
  onDestroy(stop);

  const runs = $derived(overview?.runs ?? []);
  const sorted = $derived([...runs].sort((a, b) => b.started_at - a.started_at));
  const finished = $derived(sorted.filter((r) => r.finished_at).slice(0, 10));
  const passedCount = $derived(finished.filter((r) => r.status === 'passed').length);
  const running = $derived(runs.flatMap((r) => r.jobs).filter((j) => j.status === 'running'));
  const activeRun = $derived(sorted.find((r) => r.status === 'running'));
  const queued = $derived(activeRun ? activeRun.jobs.filter((j) => j.status === 'pending') : []);
  const durs = $derived(finished.map((r) => runDuration(r)));
  const worst = $derived(Math.max(...durs, 1));
  const avgDur = $derived(durs.length ? durs.reduce((a, b) => a + b, 0) / durs.length : null);

  const act = $derived(overview ? activity(overview.runs, overview.workers, $now, WINDOW_MS) : null);

  function runDuration(r: Run): number {
    return (r.finished_at ?? Date.now()) - r.started_at;
  }

  // ---- contribution calendar ------------------------------------------------
  const calTotal = $derived(calendar.reduce((s, d) => s + d.count, 0));
  const weeks = $derived.by(() => {
    if (!calendar.length) return [] as (CalendarDay | null)[][];
    const first = new Date(calendar[0].date);
    const cells: (CalendarDay | null)[] = new Array(first.getDay()).fill(null).concat(calendar);
    const out: (CalendarDay | null)[][] = [];
    for (let i = 0; i < cells.length; i += 7) {
      const week = cells.slice(i, i + 7);
      while (week.length < 7) week.push(null);
      out.push(week);
    }
    return out;
  });
  const monthLabels = $derived.by(() => {
    const labels: { left: number; label: string }[] = [];
    let last = -1;
    weeks.forEach((week, wi) => {
      const firstDay = week.find(Boolean);
      if (!firstDay) return;
      const m = new Date(firstDay.date).getMonth();
      if (m !== last) {
        labels.push({ left: wi * 11, label: MONTHS[m] });
        last = m;
      }
    });
    return labels.slice(1);
  });
  function calLevel(count: number): string {
    if (count <= 0) return '';
    if (count === 1) return 'l1';
    if (count <= 3) return 'l2';
    if (count <= 5) return 'l3';
    return 'l4';
  }
  function calLabel(d: CalendarDay): string {
    const date = new Date(d.date);
    return `${d.count} run${d.count === 1 ? '' : 's'} on ${MONTHS[date.getMonth()]} ${date.getDate()}`;
  }

  // ---- updates feed (pipeline-file pushes only) -------------------------------
  const isYml = (f: string) => /\.ya?ml$/i.test(f);
  const feed = $derived(sorted.filter((r) => r.commit?.files?.some(isYml)).slice(0, 10));

  function sentence(r: Run): { actor: string; html: string } {
    const author = r.commit ? r.commit.author : 'someone';
    if (r.trigger === 'schedule') {
      return { actor: 'schedule', html: `<b>schedule</b> ran <b>${r.pipeline}</b> on ${r.repo}` };
    }
    if (r.trigger === 'manual') {
      return { actor: author, html: `<b>${author}</b> triggered <b>${r.pipeline}</b> manually` };
    }
    return {
      actor: author,
      html: `<b>${author}</b> pushed <code>${r.commit?.sha ?? '?'}</code> to <b>${r.repo}</b>`,
    };
  }

  function verb(r: Run): string {
    if (r.status === 'running') {
      const done = r.jobs.filter((j) => j.status === 'passed').length;
      return `is running · ${done}/${r.jobs.length} jobs done`;
    }
    if (r.status === 'failed') {
      const bad = r.jobs.find((j) => j.status === 'failed');
      return `<b class="bad">failed at ${bad?.name ?? '?'}</b> after ${fmtDur(runDuration(r))}`;
    }
    return `passed in <b>${fmtDur(runDuration(r))}</b>`;
  }

  async function trigger() {
    triggering = true;
    try {
      await api.trigger();
      overview = await api.overview();
    } finally {
      triggering = false;
    }
  }

  const repoCount = $derived(new Set(runs.map((r) => r.repo ?? r.pipeline)).size);
</script>

<Topbar active="runs" {overview}>
  <div class="wrap">
    <div class="page-head">
      <h1>Overview</h1>
      <span class="meta">
        {repoCount} repos · {runs.length} runs · {runs.filter((r) => r.status === 'passed').length} passed
      </span>
      <span class="actions">
        <button class="btn btn-lime" onclick={trigger} disabled={triggering}>Trigger run</button>
      </span>
    </div>

    <div class="band-grid">
      <div class="tile">
        <span class="tlabel">Success rate · 10 runs</span>
        <span class="tval">
          {#if finished.length}{Math.round((passedCount / finished.length) * 100)}<span class="u">%</span>{:else}–{/if}
        </span>
        <span class="tsub">{passedCount} passed · {finished.length - passedCount} failed</span>
        <span class="tviz sq-row" role="img" aria-label="Last 10 run outcomes">
          {#each [...finished].reverse() as r (r.id)}
            <span class="sq {r.status}" title="#{r.id} {r.status}"></span>
          {/each}
        </span>
      </div>
      <div class="tile">
        <span class="tlabel">Active now</span>
        <span class="tval">{running.length}<span class="u">running</span></span>
        <span class="tsub">
          {running.length ? running.slice(0, 2).map((j) => `${j.name}@${j.worker}`).join(' · ') : 'cluster idle'}
        </span>
        <span class="tviz sq-row" role="img" aria-label="Jobs running now">
          {#each running as j (j.id)}<span class="sq running" title={j.name}></span>{/each}
          {#if !running.length}<span class="sq"></span>{/if}
        </span>
      </div>
      <div class="tile">
        <span class="tlabel">Avg run duration</span>
        <span class="tval">{avgDur ? fmtDur(avgDur) : '–'}</span>
        <span class="tsub">
          {durs.length ? `fastest ${fmtDur(Math.min(...durs))} · slowest ${fmtDur(worst)}` : ''}
        </span>
        <span class="tviz sbars" role="img" aria-label="Recent run durations">
          {#each [...finished].reverse() as r (r.id)}
            <i
              class:failed={r.status === 'failed'}
              style="height:{Math.max(10, Math.round((runDuration(r) / worst) * 100))}%"
              title="#{r.id} {fmtDur(runDuration(r))}"
            ></i>
          {/each}
        </span>
      </div>
      <div class="tile">
        <span class="tlabel">Queue</span>
        <span class="tval">{queued.length}<span class="u">jobs</span></span>
        <span class="tsub">{activeRun ? `waiting in run #${activeRun.id}` : 'nothing waiting'}</span>
        <span class="tviz sq-row" role="img" aria-label="Queued jobs">
          {#each queued as j (j.id)}<span class="sq" title={j.name}></span>{/each}
          {#if !queued.length}<span class="sq passed" title="queue empty"></span>{/if}
        </span>
      </div>
    </div>
  </div>
</Topbar>

<main class="wrap">
  {#if error}<div class="err-banner">{error}</div>{/if}

  <div class="dash-grid">
    <section class="panel-graph" aria-label="Run activity, past year">
      <div class="graph-head">
        <span class="glabel">Run activity · past year</span>
        <span class="gvalue">{calTotal} runs</span>
      </div>
      <div class="cal-scroll">
        <div class="cal">
          <div class="cal-months">
            {#each monthLabels as m (m.left)}<span style="left:{m.left}px">{m.label}</span>{/each}
          </div>
          <div class="cal-body">
            <div class="cal-dow"><span></span><span>mon</span><span></span><span>wed</span><span></span><span>fri</span><span></span></div>
            <div class="cal-grid">
              {#each weeks as week, wi (wi)}
                <div class="cal-week">
                  {#each week as d, di (di)}
                    {#if d}
                      <span class="cal-cell {calLevel(d.count)}" title={calLabel(d)} aria-label={calLabel(d)}></span>
                    {:else}
                      <span class="cal-cell future" aria-hidden="true"></span>
                    {/if}
                  {/each}
                </div>
              {/each}
            </div>
          </div>
        </div>
      </div>
      <div class="cal-legend">
        <span>less</span>
        <span class="cal-cell"></span><span class="cal-cell l1"></span><span class="cal-cell l2"></span><span class="cal-cell l3"></span><span class="cal-cell l4"></span>
        <span>more</span>
      </div>
    </section>

    <section class="panel-workers" aria-label="Workers">
      <span class="glabel">Workers · 15m utilization</span>
      <div>
        {#if overview && act}
          {#each overview.workers as w (w.name)}
            {@const st = deviceStats(act.byWorker.get(w.name) ?? [], act)}
            {@const runningIv = (act.byWorker.get(w.name) ?? []).find((iv) => iv.status === 'running')}
            <div class="wrow" class:offline={w.status !== 'online'}>
              <span class="dot" aria-hidden="true"></span>
              <span class="wname">{w.name}</span>
              <span class="ubar"><i style="width:{st.util}%"></i></span>
              {#if w.status !== 'online'}
                <span class="upct">off {ago(w.last_heartbeat, $now).replace(' ago', '')}</span>
              {:else if runningIv}
                <span class="wjob"><a href="#/run/{runningIv.run.id}?job={runningIv.job.id}">{runningIv.job.name}</a></span>
              {:else}
                <span class="upct">{st.util}%</span>
              {/if}
            </div>
          {/each}
        {/if}
      </div>
    </section>
  </div>

  <div class="section-label">Updates <span class="meta">pushes that touched a pipeline file (.yml / .yaml)</span></div>
  <section class="feed" aria-label="Updates">
    {#if !feed.length}
      <div class="empty">
        No pipeline-file pushes yet — updates appear when a commit touches a
        <code>.yml</code> / <code>.yaml</code> pipeline definition.
      </div>
    {/if}
    {#each feed as r (r.id)}
      {@const s = sentence(r)}
      {@const ymlFile = r.commit?.files.find(isYml) ?? ''}
      <article class="fcard">
        <div class="fcard-head">
          <Avatar name={s.actor} />
          <span class="sentence">{@html s.html}</span>
          <span class="when">{ago(r.started_at, $now)}</span>
        </div>
        <a class="fobj" href="#/run/{r.id}">
          <span class="g {r.status}" aria-hidden="true">{GLYPH[r.status] ?? '·'}</span>
          <span class="fobj-body">
            <span class="fobj-l1">
              <span class="rref">{r.pipeline} #{r.id}</span>
              <span class="verb">{@html verb(r)}</span>
            </span>
            <span class="fobj-l2">{r.commit?.message ?? '—'}</span>
          </span>
          <span class="fobj-side">
            <span class="file-chip" title={ymlFile}>{ymlFile.split('/').pop()}</span>
            <span class="repo-tag">{r.repo ?? r.pipeline}</span>
            <Strip jobs={r.jobs} />
          </span>
        </a>
      </article>
    {/each}
  </section>

  <footer class="foot">mode: {MODE} · polling every 3s · coordinator http://127.0.0.1:8080</footer>
</main>
