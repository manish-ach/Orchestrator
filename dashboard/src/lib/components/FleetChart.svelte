<script lang="ts">
  import type { Job, Worker } from '../types';

  // Demand against capacity, as an exact step function.
  //
  // Two deliberate departures from a plain "workers busy" bar chart:
  //   * the series is *demand* (jobs wanting to run), which can exceed capacity.
  //     "Workers busy" mathematically cannot, so it can never show a shortage.
  //   * it is drawn as a step area off the real job intervals, with no bucketing.
  //     The underlying quantity holds a value until an event changes it; bars
  //     imply N independent samples and quantise into a picket fence.
  let {
    jobs = [],
    workers = [],
    now = Date.now(),
    windowMs = 15 * 60 * 1000,
  }: { jobs?: Job[]; workers?: Worker[]; now?: number; windowMs?: number } = $props();

  const t0 = $derived(now - windowMs);

  type Step = [t: number, v: number];

  /** Change points of "how many jobs wanted a worker" across the window. */
  const demand = $derived.by<Step[]>(() => {
    const deltas: [number, number][] = [];
    for (const j of jobs) {
      // a job is demanding a worker from the moment it became runnable until it
      // actually started; once running it is demanding one too, until it ends
      const from = j.ready_at ?? j.started_at;
      if (from === null || from === undefined) continue;
      const to = j.finished_at ?? now;
      if (to < t0 || from > now) continue;
      deltas.push([Math.max(from, t0), 1]);
      deltas.push([Math.min(to, now), -1]);
    }
    deltas.sort((a, b) => a[0] - b[0]);
    const out: Step[] = [[t0, 0]];
    let level = 0;
    for (const [t, d] of deltas) {
      level += d;
      if (out[out.length - 1][0] === t) out[out.length - 1][1] = level;
      else out.push([t, level]);
    }
    return out;
  });

  /** Online workers. Steps down when one is reaped, which is itself the story. */
  const capacity = $derived.by<Step[]>(() => {
    const online = workers.filter((w) => w.status === 'online').length;
    const lost = workers.filter((w) => w.status !== 'online' && w.last_heartbeat > t0);
    if (!lost.length) return [[t0, online]];
    const steps: Step[] = [[t0, online + lost.length]];
    for (const w of [...lost].sort((a, b) => a.last_heartbeat - b.last_heartbeat)) {
      const prev = steps[steps.length - 1][1];
      steps.push([w.last_heartbeat, prev - 1]);
    }
    return steps;
  });

  const at = (s: Step[], t: number) => {
    let v = s[0]?.[1] ?? 0;
    for (const [tt, vv] of s) { if (tt <= t) v = vv; else break; }
    return v;
  };

  const stats = $derived.by(() => {
    const marks = [...new Set([...demand.map((d) => d[0]), ...capacity.map((c) => c[0]), now])].sort((a, b) => a - b);
    let used = 0, cap = 0, saturated = 0, peak = 0, top = 0;
    const runs: [number, number][] = [];
    for (let i = 0; i < marks.length - 1; i++) {
      const t = marks[i], dt = marks[i + 1] - t;
      const d = at(demand, t), c = at(capacity, t);
      used += Math.min(d, c) * dt;
      cap += c * dt;
      peak = Math.max(peak, d);
      top = Math.max(top, d, c);
      if (c > 0 && d >= c) {
        const last = runs[runs.length - 1];
        if (last && last[1] === t) last[1] = marks[i + 1];
        else runs.push([t, marks[i + 1]]);
        saturated += dt;
      }
    }
    return { busyPct: cap ? Math.round((used / cap) * 100) : 0, saturated, peak, top: Math.max(1, top), runs };
  });

  let box = $state<HTMLElement | null>(null);
  let w = $state(600);
  let h = $state(150);
  $effect(() => {
    if (!box) return;
    const ro = new ResizeObserver(() => {
      w = Math.round(box!.clientWidth);
      h = Math.round(box!.clientHeight);
    });
    ro.observe(box);
    return () => ro.disconnect();
  });

  const pad = { l: 26, r: 4, t: 8, b: 4 };
  const X = $derived((t: number) => pad.l + ((t - t0) / windowMs) * (w - pad.l - pad.r));
  const Y = $derived((v: number) => pad.t + (h - pad.t - pad.b) - (v / stats.top) * (h - pad.t - pad.b));

  function stepPath(s: Step[]): string {
    if (!s.length) return '';
    let d = `M ${X(s[0][0])} ${Y(s[0][1])}`;
    for (let i = 0; i < s.length; i++) {
      const next = i + 1 < s.length ? s[i + 1][0] : now;
      d += ` H ${X(next)}`;
      if (i + 1 < s.length) d += ` V ${Y(s[i + 1][1])}`;
    }
    return d;
  }
  const areaPath = (s: Step[]) => `${stepPath(s)} V ${Y(0)} H ${X(t0)} Z`;
  const mmss = (ms: number) => {
    const s = Math.round(ms / 1000);
    return s >= 60 ? `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s` : `${s}s`;
  };
</script>

<div class="fleethead">
  <span><b>{stats.busyPct}%</b> busy</span>
  <span><b class:warn={stats.saturated > 0}>{mmss(stats.saturated)}</b> at capacity</span>
  <span>peak <b>{stats.peak}</b> in flight · <b>{at(capacity, now)}</b> workers</span>
</div>

<div class="plot" bind:this={box}>
  <svg viewBox="0 0 {w} {h}" width={w} height={h} aria-label="Fleet demand against capacity">
    <defs>
      <clipPath id="overcap">
        <path d="{stepPath(capacity)} L {X(now)} {pad.t} L {X(t0)} {pad.t} Z" />
      </clipPath>
    </defs>

    {#each Array.from({ length: stats.top + 1 }, (_, i) => i) as g (g)}
      <line x1={pad.l} y1={Y(g)} x2={w - pad.r} y2={Y(g)} stroke="oklch(0.925 0.005 110)" stroke-dasharray={g ? '2 4' : ''} />
      <text x={pad.l - 6} y={Y(g) + 3.4} text-anchor="end" font-family="var(--mono)" font-size="9.5" fill="oklch(0.70 0.01 110)">{g}</text>
    {/each}

    <!-- capacity as an area, so unused headroom is a visible quantity rather than a legend entry -->
    <path d={areaPath(capacity)} fill="oklch(0.935 0.004 110)" />
    <path d={areaPath(demand)} fill="oklch(0.50 0.105 112)" fill-opacity="0.30" />
    <!-- the part of demand above the ceiling: jobs that could not get a worker -->
    <path d={areaPath(demand)} fill="oklch(0.74 0.15 62)" fill-opacity="0.55" clip-path="url(#overcap)" />
    <path d={stepPath(demand)} fill="none" stroke="oklch(0.50 0.105 112)" stroke-width="1.6" stroke-linejoin="round" />
    <path d={stepPath(capacity)} fill="none" stroke="oklch(0.55 0.012 110)" stroke-width="1.5" stroke-dasharray="5 3" />

    <!-- exact ties leave no overshoot area to see, so zero-headroom spells itself out -->
    {#each stats.runs as [a, b], i (i)}
      <rect x={X(a)} y={h - 3} width={Math.max(2, X(b) - X(a))} height="3" rx="1.5" fill="oklch(0.74 0.15 62)" />
    {/each}
  </svg>
</div>

<div class="axis"><span>15m ago</span><span>10m</span><span>5m</span><span>now</span></div>

<style>
  .fleethead {
    display: flex;
    gap: 15px;
    align-items: baseline;
    padding-bottom: 9px;
    font-size: 11px;
    color: var(--faint);
    flex: none;
    white-space: nowrap;
  }
  .fleethead b {
    font-family: var(--mono);
    font-variant-numeric: tabular-nums;
    font-size: 14px;
    font-weight: 600;
    color: var(--ink);
    letter-spacing: -0.02em;
  }
  .fleethead b.warn { color: oklch(0.52 0.13 55); }
  .plot { flex: 1; min-height: 0; }
  .plot svg { display: block; }
  .axis {
    display: flex;
    justify-content: space-between;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--faint);
    margin-top: 6px;
    padding-left: 26px;
    flex: none;
  }
</style>
