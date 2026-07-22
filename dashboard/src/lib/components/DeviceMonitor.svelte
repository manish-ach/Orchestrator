<script lang="ts">
  // Device monitor: real per-worker CPU/RAM graphs from heartbeat samples,
  // with a metric toggle. Used on the run screen.
  import { drawStatChart } from '../charts';
  import type { StatSample, Worker, WorkerStatsSeries } from '../types';

  let {
    workers,
    series,
    names = [],
    now,
  }: {
    workers: Worker[];
    series: WorkerStatsSeries[];
    /** device names to show; empty → every registered worker */
    names?: string[];
    now: number;
  } = $props();

  const WINDOW_MS = 5 * 60 * 1000;
  let metric = $state<'cpu' | 'mem'>('cpu');

  const COLORS = {
    cpu: { stroke: 'oklch(0.52 0.105 112)', fill: 'oklch(0.52 0.105 112 / 0.16)' },
    mem: { stroke: 'oklch(0.5 0.13 265)', fill: 'oklch(0.5 0.13 265 / 0.13)' },
  } as const;

  interface Device {
    name: string;
    worker: Worker | undefined;
    samples: StatSample[];
  }

  const devices = $derived.by<Device[]>(() => {
    const wanted = names.length ? names : workers.map((w) => w.name);
    return wanted.map((name) => ({
      name,
      worker: workers.find((w) => w.name === name),
      samples: series.find((s) => s.name === name)?.samples ?? [],
    }));
  });

  interface ChartParams {
    samples: StatSample[];
    t0: number;
    t1: number;
    metric: 'cpu' | 'mem';
  }
  function statChart(canvas: HTMLCanvasElement, params: ChartParams) {
    const draw = (p: ChartParams) =>
      drawStatChart(canvas, p.samples, { t0: p.t0, t1: p.t1, metric: p.metric, ...COLORS[p.metric] });
    draw(params);
    return {
      update(next: ChartParams) {
        draw(next);
      },
    };
  }

  function nowValue(d: Device): number | null {
    if (d.worker?.status !== 'online') return null;
    const st = d.worker?.stats;
    if (st) return metric === 'cpu' ? st.cpu_pct : st.mem_pct;
    const last = d.samples.at(-1);
    return last ? last[metric] : null;
  }

  function ramLabel(d: Device): string {
    const st = d.worker?.stats;
    if (!st || !st.mem_total_mb) return '';
    return `${(st.mem_used_mb / 1024).toFixed(1)} / ${(st.mem_total_mb / 1024).toFixed(0)} GB`;
  }
</script>

<div class="section-label devmon-label">
  Device monitor
  <span class="meta">{metric === 'cpu' ? 'CPU usage' : 'memory usage'} · reported by worker heartbeats · last 5 min</span>
  <span class="devmon-toggle" role="group" aria-label="Metric">
    <button type="button" class="toggle" aria-pressed={metric === 'cpu'} onclick={() => (metric = 'cpu')}>CPU</button>
    <button type="button" class="toggle" aria-pressed={metric === 'mem'} onclick={() => (metric = 'mem')}>RAM</button>
  </span>
</div>

<div class="devmon-grid">
  {#each devices as d (d.name)}
    {@const offline = d.worker?.status !== 'online'}
    {@const v = nowValue(d)}
    <div class="card devmon-card" class:offline>
      <div class="devmon-head">
        <span class="dot" aria-hidden="true"></span>
        <span class="dname">{d.name}</span>
        <span class="dval">
          {#if offline}
            offline
          {:else if v !== null}
            {v.toFixed(0)}<span class="u">%</span>
          {:else}
            —
          {/if}
        </span>
      </div>
      <canvas
        class="devmon-chart"
        use:statChart={{ samples: d.samples, t0: now - WINDOW_MS, t1: now, metric }}
        aria-label="{metric === 'cpu' ? 'CPU' : 'RAM'} usage of {d.name} over the last 5 minutes"
      ></canvas>
      <div class="graph-axis"><span>5m ago</span><span>now</span></div>
      <div class="devmon-foot">
        {#if d.worker?.stats}
          cpu <b>{d.worker.stats.cpu_pct.toFixed(0)}%</b> · ram <b>{d.worker.stats.mem_pct.toFixed(0)}%</b>
          {#if ramLabel(d)}<span class="dim">({ramLabel(d)})</span>{/if}
        {:else if d.samples.length}
          last sample {new Date(d.samples.at(-1)!.t).toLocaleTimeString()}
        {:else}
          no stats yet — worker predates stats reporting or has not heartbeat
        {/if}
      </div>
    </div>
  {:else}
    <div class="empty">No devices to monitor yet.</div>
  {/each}
</div>
