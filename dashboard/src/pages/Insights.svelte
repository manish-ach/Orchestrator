<script lang="ts">
  import { api } from '../lib/api';
  import AppShell from '../lib/components/AppShell.svelte';
  import Calendar from '../lib/components/Calendar.svelte';
  import { fmtDur } from '../lib/format';
  import { overview as live } from '../lib/live';
  import { route } from '../lib/router';
  import type { Insights, Overview } from '../lib/types';

  // Insights answers questions no other page can, because every panel here is
  // keyed by something other than a run id — a stage, a job name, a worker, a
  // day. Control center and Runs are keyed by run id and own that view; if a
  // card on this page could be identified by "which run?", it is on the wrong
  // page. Each panel therefore states its question in the header, and ends with
  // the one sentence the numbers support. No panel exists to be pretty.

  const RANGES = [
    { days: 7, label: 'last 7d' },
    { days: 30, label: 'last 30d' },
    { days: 90, label: 'last 90d' },
  ];

  const overview = $derived($live);
  let data = $state<Insights | null>(null);
  let error = $state('');
  let loading = $state(true);

  const range = $derived(Number($route.query.get('range')) || 30);

  function setRange(days: number) {
    location.hash = `/insights?range=${days}`;
  }

  // The shell's live counters come from the shared poller; the aggregates below
  // deliberately do not follow it. A 90-day rollup does not change every three
  // seconds, and re-fetching it on that cadence would be its most expensive
  // habit for no new information.
  $effect(() => {
    const days = range;
    let stale = false;
    loading = true;
    api
      .insights(days)
      .then((d) => {
        if (stale) return;
        data = d;
        error = '';
      })
      .catch((e) => {
        if (!stale) error = `Cannot load insights (${(e as Error).message}).`;
      })
      .finally(() => {
        if (!stale) loading = false;
      });
    return () => {
      stale = true;
    };
  });

  const passPct = $derived(data && data.runs ? Math.round((data.passed / data.runs) * 100) : null);

  // ---- run activity ------------------------------------------------------
  // Which hours the fleet is used is not in the calendar's daily buckets, so
  // the takeaway only claims what the data supports: weekday vs weekend.
  const weekendShare = $derived.by(() => {
    if (!data?.calendar.length) return null;
    let weekend = 0;
    let total = 0;
    for (const d of data.calendar) {
      const dow = new Date(`${d.date}T00:00:00`).getDay();
      total += d.count;
      if (dow === 0 || dow === 6) weekend += d.count;
    }
    return total ? Math.round((weekend / total) * 100) : null;
  });

  // ---- where the time goes -----------------------------------------------
  const stageMax = $derived(Math.max(1, ...(data?.stages ?? []).map((s) => s.p90_ms)));
  const runMedian = $derived(data?.median_ms ?? 0);
  const worstStage = $derived(data?.stages[0] ?? null);
  const stageShare = $derived(worstStage && runMedian ? Math.round((worstStage.median_ms / runMedian) * 100) : null);

  // ---- duration trend ----------------------------------------------------
  const W = 640;
  const H = 132;
  const trendMax = $derived(Math.max(1, ...(data?.trend ?? []).map((t) => t.p90_ms)));
  function line(key: 'p50_ms' | 'p90_ms'): string {
    const t = data?.trend ?? [];
    if (t.length < 2) return '';
    return t
      .map((p, i) => `${i === 0 ? 'M' : 'L'}${(i / (t.length - 1)) * W},${H - (p[key] / trendMax) * H}`)
      .join(' ');
  }
  /** Change across the window, first point to last, per series. */
  function delta(key: 'p50_ms' | 'p90_ms'): number | null {
    const t = data?.trend ?? [];
    return t.length >= 2 ? t[t.length - 1][key] - t[0][key] : null;
  }
  const signed = (ms: number) => `${ms >= 0 ? '+' : '−'}${fmtDur(Math.abs(ms))}`;

  // ---- queue wait --------------------------------------------------------
  const waitMax = $derived(Math.max(1, ...(data?.wait ?? []).map((w) => w.wait_ms + w.exec_ms)));
  const worstWait = $derived.by(() => {
    const w = data?.wait ?? [];
    if (!w.length) return null;
    return w.reduce((a, b) => (b.wait_ms / (b.wait_ms + b.exec_ms || 1) > a.wait_ms / (a.wait_ms + a.exec_ms || 1) ? b : a));
  });
  const worstWaitPct = $derived(
    worstWait ? Math.round((worstWait.wait_ms / (worstWait.wait_ms + worstWait.exec_ms || 1)) * 100) : null,
  );

  // ---- worker skew -------------------------------------------------------
  const skewMax = $derived(Math.max(1, ...(data?.skew?.workers ?? []).map((w) => w.median_ms)));
  const skewRatio = $derived.by(() => {
    const w = data?.skew?.workers ?? [];
    if (w.length < 2) return null;
    const fast = w[0].median_ms;
    const slow = w[w.length - 1].median_ms;
    return fast > 0 ? Math.round((slow / fast) * 10) / 10 : null;
  });

  // ---- failure causes ----------------------------------------------------
  const causeTotal = $derived((data?.causes ?? []).reduce((n, c) => n + c.count, 0));
  const topCauseShare = $derived(
    data?.causes.length && causeTotal ? Math.round((data.causes[0].count / causeTotal) * 100) : null,
  );

  const shortDate = (d: string) => new Date(`${d}T00:00:00`).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
</script>

<AppShell active="insights" {overview}>
  <div class="page-body scrolly">
    <div class="scopebar">
      <div class="chipset" role="group" aria-label="Time range">
        {#each RANGES as r (r.days)}
          <button class="schip" class:on={range === r.days} onclick={() => setRange(r.days)}>{r.label}</button>
        {/each}
      </div>
      <span class="rangenote">
        {#if loading && !data}
          reading history…
        {:else if data}
          derived from {data.runs} runs
        {/if}
      </span>
    </div>

    {#if error}<div class="err-banner">{error}</div>{/if}

    {#if data}
      <!-- Every panel below states its own window, because they differ: these
           six and most panels follow the range chips, while Run activity is
           always a year. Leaving that implicit invites reading one number as
           context for another. -->
      <div class="windowlbl">
        Last {data.range_days} days
        <span class="d">·</span>{data.runs} runs
      </div>
      <div class="wells six">
        <div class="well"><span class="lbl">Runs</span><span class="v">{data.runs}</span></div>
        <div class="well">
          <span class="lbl">Passed</span><span class="v">{passPct ?? '–'}<small>% · {data.failed} failed</small></span>
        </div>
        <div class="well"><span class="lbl">Median</span><span class="v">{fmtDur(data.median_ms)}</span></div>
        <div class="well"><span class="lbl">p90</span><span class="v">{fmtDur(data.p90_ms)}</span></div>
        <div class="well">
          <span class="lbl">Time to recovery</span>
          <span class="v sm">{data.recovery_ms ? fmtDur(data.recovery_ms) : 'never went red'}</span>
        </div>
        <div class="well">
          <span class="lbl">Longest red streak</span>
          <span class="v sm">{data.longest_red_ms ? fmtDur(data.longest_red_ms) : 'none'}</span>
        </div>
      </div>

      <!-- Run activity leads: everything below is a claim about a window, and
           this is the panel that shows what the window contains. -->
      <section class="card">
        <div class="chead col">
          <h2>Run activity</h2>
          <p class="ask">
            When does this fleet actually get used?
            <span class="scope">last 12 months — not the {data.range_days}-day window above</span>
          </p>
        </div>
        <Calendar days={data.calendar} unit="runs" />
        {#if weekendShare !== null}
          <p class="takeaway">
            Weekends carry <b>{weekendShare}%</b> of all runs in the last year.
            {weekendShare < 15
              ? 'Overnight and weekend capacity is sitting idle — that is where a slow nightly job belongs.'
              : 'Load is spread through the week, so there is no quiet window to hide long jobs in.'}
          </p>
        {/if}
      </section>

      <div class="two">
        <section class="card">
          <div class="chead col">
            <h2>Where the time goes</h2>
            <p class="ask">Which stage should we attack to make runs faster?</p>
          </div>
          <div class="pad">
            {#each data.stages as s (s.stage)}
              <div class="srow">
                <span class="sname">
                  {s.stage}
                  <small>{s.jobs} {s.jobs === 1 ? 'job' : 'jobs'}</small>
                </span>
                <span class="sbar">
                  <i class="p90" style="width:{(s.p90_ms / stageMax) * 100}%"></i>
                  <i class="p50" style="width:{(s.median_ms / stageMax) * 100}%"></i>
                </span>
                <span class="sval">{fmtDur(s.median_ms)}<small>&nbsp;/ {fmtDur(s.p90_ms)}</small></span>
              </div>
            {:else}
              <p class="none">No finished stages in this window.</p>
            {/each}
            {#if data.stages.length}
              <div class="legend2">
                <span><i class="k p50"></i>median</span><span><i class="k p90"></i>p90 tail</span>
              </div>
            {/if}
          </div>
          {#if worstStage && stageShare}
            <p class="takeaway">
              <b>{worstStage.stage}</b> is {stageShare}% of a median run, and its p90 runs
              <b>{Math.round((worstStage.p90_ms / Math.max(1, worstStage.median_ms)) * 10) / 10}×</b> its median. The tail
              is what people feel, not the average.
            </p>
          {/if}
        </section>

        <section class="card">
          <div class="chead col">
            <h2>Flaky jobs</h2>
            <p class="ask">Which jobs pass and fail on the <i>same</i> commit?</p>
          </div>
          <div class="pad">
            {#each data.flaky as f (f.name)}
              <div class="frow2">
                <span class="fname">
                  {f.name}
                  <small>{f.repo}</small>
                </span>
                <span class="fflips"><b>{f.flips}</b> on same sha</span>
                <span class="fstrip">
                  {#each f.recent.split('') as c, i (i)}
                    <i class={c === 'f' ? 'bad' : 'ok'}></i>
                  {/each}
                </span>
              </div>
            {:else}
              <p class="none">
                No job has disagreed with itself on the same commit in this window. Every failure here was the code.
              </p>
            {/each}
          </div>
          {#if data.flaky.length}
            <p class="takeaway">
              <b>{data.flaky[0].name}</b> flipped on {data.flaky[0].flips} identical
              {data.flaky[0].flips === 1 ? 'commit' : 'commits'}. Re-running it does not tell you which answer was right.
            </p>
          {/if}
        </section>
      </div>

      <div class="two">
        <section class="card">
          <div class="chead col">
            <h2>Duration trend</h2>
            <p class="ask">
              Are runs getting slower, and is it the median or the tail?
              {#if data.trend.length}<span class="scope">{data.trend.length} days had a finished run</span>{/if}
            </p>
          </div>
          <div class="pad">
            {#if data.trend.length >= 2}
              <!-- the plot stretches, so the scale is labelled outside it in
                   normal type rather than scaled-up SVG text -->
              <div class="plotrow">
                <div class="yaxis">
                  <span>{fmtDur(trendMax)}</span>
                  <span>{fmtDur(trendMax / 2)}</span>
                  <span>0</span>
                </div>
                <svg
                  class="trend"
                  viewBox="0 0 {W} {H}"
                  preserveAspectRatio="none"
                  role="img"
                  aria-label="p50 and p90 run duration over time"
                >
                  {#each [0, 0.5, 1] as f (f)}
                    <line x1="0" x2={W} y1={H - f * H} y2={H - f * H} class="grid" />
                  {/each}
                  <path d={line('p90_ms')} class="l90" />
                  <path d={line('p50_ms')} class="l50" />
                </svg>
              </div>
              <div class="axis2">
                <span>{shortDate(data.trend[0].date)}</span>
                <span class="legend2">
                  <span><i class="k p50"></i>p50</span><span><i class="k p90"></i>p90</span>
                </span>
                <span>{shortDate(data.trend[data.trend.length - 1].date)}</span>
              </div>
            {:else}
              <p class="none">Not enough finished runs in this window to draw a trend.</p>
            {/if}
          </div>
          {#if delta('p50_ms') !== null}
            <p class="takeaway">
              p50 {delta('p50_ms')! <= 0 ? 'improved' : 'grew'} <b>{signed(delta('p50_ms')!)}</b>, p90
              {delta('p90_ms')! <= 0 ? 'improved' : 'grew'} <b>{signed(delta('p90_ms')!)}</b>.
              {delta('p90_ms')! > 0 && delta('p50_ms')! <= 0
                ? 'A slow minority is getting slower while the average looks fine.'
                : 'Both ends are moving together, so this is the whole pipeline, not one bad job.'}
            </p>
          {/if}
        </section>

        <section class="card">
          <div class="chead col">
            <h2>Queue wait vs execution</h2>
            <p class="ask">
              Do we need more workers, or faster ones?
              {#if data.wait.length}<span class="scope">last {data.wait.length} days with jobs</span>{/if}
            </p>
          </div>
          <div class="pad">
            {#if data.wait.length}
              <div class="wbars">
                {#each data.wait as w (w.date)}
                  {@const total = w.wait_ms + w.exec_ms}
                  <span
                    class="wcol"
                    title="{shortDate(w.date)} · waiting {fmtDur(w.wait_ms)} · executing {fmtDur(w.exec_ms)}"
                  >
                    <i class="wait" style="height:{(w.wait_ms / waitMax) * 100}%"></i>
                    <i class="exec" style="height:{(w.exec_ms / waitMax) * 100}%"></i>
                    <em>{Math.round((w.wait_ms / (total || 1)) * 100)}%</em>
                  </span>
                {/each}
              </div>
              <div class="axis2">
                <span>{shortDate(data.wait[0].date)}</span>
                <span class="legend2">
                  <span><i class="k wait"></i>waiting for a worker</span><span><i class="k exec"></i>executing</span>
                </span>
                <span>{shortDate(data.wait[data.wait.length - 1].date)}</span>
              </div>
            {:else}
              <p class="none">No jobs started in this window.</p>
            {/if}
          </div>
          {#if worstWait && worstWaitPct !== null}
            <p class="takeaway">
              On {shortDate(worstWait.date)} waiting was <b>{worstWaitPct}%</b> of total job time.
              {worstWaitPct >= 25
                ? 'More workers would help those days; faster ones would not.'
                : 'Jobs are spending their time running, not queuing — capacity is not the bottleneck.'}
            </p>
          {/if}
        </section>
      </div>

      <div class="two">
        <section class="card">
          <div class="chead col">
            <h2>Worker skew{#if data.skew}&nbsp;— {data.skew.job}{/if}</h2>
            <p class="ask">Does the same job behave differently per machine?</p>
          </div>
          <div class="pad">
            {#if data.skew}
              {#each data.skew.workers as w (w.worker)}
                <div class="krow">
                  <span class="kname">
                    {w.worker}
                    <small>{w.runs} runs</small>
                  </span>
                  <span class="kbar"><i style="width:{(w.median_ms / skewMax) * 100}%"></i></span>
                  <span class="kval">
                    {fmtDur(w.median_ms)}
                    <small>&nbsp;· {w.pass_pct ?? '–'}%</small>
                  </span>
                </div>
              {/each}
            {:else}
              <p class="none">
                No job in this window ran on more than one machine, so there is nothing to compare.
              </p>
            {/if}
          </div>
          {#if data.skew && skewRatio && skewRatio > 1.2}
            <p class="takeaway">
              <b>{data.skew.job}</b> is {skewRatio}× faster on {data.skew.workers[0].worker}. Pinning it with a
              <code>tags:</code> label would cut roughly
              {fmtDur(data.skew.workers[data.skew.workers.length - 1].median_ms - data.skew.workers[0].median_ms)} off every
              run that lands elsewhere.
            </p>
          {/if}
        </section>

        <section class="card">
          <div class="chead col">
            <h2>Why runs fail</h2>
            <p class="ask">What are the actual causes, ranked?</p>
          </div>
          <div class="pad">
            {#each data.causes as c (c.cause + c.job)}
              <div class="crow">
                <span class="cwhat">
                  <span class="cmsg">{c.cause}</span>
                  <small>exit {c.exit_code ?? '?'} · {c.job}</small>
                </span>
                <span class="cbar"><i style="width:{(c.count / data.causes[0].count) * 100}%"></i></span>
                <span class="ccount">{c.count}</span>
              </div>
            {:else}
              <p class="none">Nothing failed in this window.</p>
            {/each}
          </div>
          {#if data.causes.length && topCauseShare !== null}
            <p class="takeaway">
              One cause accounts for <b>{topCauseShare}%</b> of failures in this window. Fixing it is worth more than
              any number of retries.
            </p>
          {/if}
        </section>
      </div>
    {:else if !loading && !error}
      <div class="emptyq">
        <b>Nothing to analyse yet</b>
        Insights needs finished runs. Trigger a pipeline and come back.
      </div>
    {/if}
  </div>
</AppShell>
