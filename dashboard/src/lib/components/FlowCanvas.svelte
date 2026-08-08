<script lang="ts">
  import { fmtDur, GLYPH } from '../format';
  import type { Job } from '../types';

  // Pannable, zoomable stage DAG. Shared by the control centre's live pipeline
  // card and the run detail page, so the graph behaves identically wherever you
  // meet it. Edges come from each job's `needs`, not from stage order — two jobs
  // in the same stage can have quite different dependencies.
  let {
    jobs = [],
    height = 300,
    onselect,
  }: {
    jobs?: Job[];
    height?: number;
    onselect?: (job: Job) => void;
  } = $props();

  const NW = 200, NH = 64, COL_GAP = 76, ROW_GAP = 14, PAD = 22, TOP = 34;

  const EDGE: Record<string, string> = {
    passed: 'oklch(0.72 0.10 145)',
    failed: 'oklch(0.72 0.15 27)',
    running: 'oklch(0.80 0.10 82)',
    pending: 'oklch(0.865 0.006 110)',
  };

  const stages = $derived([...new Set(jobs.map((j) => j.stage))]);
  const pos = $derived.by(() => {
    const m = new Map<string, { x: number; y: number }>();
    stages.forEach((s, ci) =>
      jobs
        .filter((j) => j.stage === s)
        .forEach((j, ri) => m.set(j.name, { x: PAD + ci * (NW + COL_GAP), y: TOP + ri * (NH + ROW_GAP) })),
    );
    return m;
  });
  const widest = $derived(Math.max(1, ...stages.map((s) => jobs.filter((j) => j.stage === s).length)));
  const W = $derived(PAD * 2 + Math.max(stages.length, 1) * (NW + COL_GAP) - COL_GAP);
  const H = $derived(TOP + widest * (NH + ROW_GAP) - ROW_GAP + PAD);

  const edges = $derived.by(() => {
    const byName = new Map(jobs.map((j) => [j.name, j]));
    const out: { d: string; stroke: string; live: boolean }[] = [];
    for (const j of jobs) {
      for (const dep of j.needs ?? []) {
        const a = pos.get(dep);
        const b = pos.get(j.name);
        const from = byName.get(dep);
        if (!a || !b || !from) continue;
        const x1 = a.x + NW, y1 = a.y + NH / 2, x2 = b.x, y2 = b.y + NH / 2;
        const dx = Math.max(36, (x2 - x1) / 2);
        out.push({
          d: `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`,
          stroke: EDGE[from.status] ?? EDGE.pending,
          live: j.status === 'running',
        });
      }
    }
    return out;
  });

  let view = $state<HTMLElement | null>(null);
  let pan = $state({ x: 0, y: 0 });
  let scale = $state(1);
  let drag: { x: number; y: number; moved: boolean } | null = null;

  function setScale(next: number, cx: number, cy: number) {
    const s = Math.min(2.5, Math.max(0.4, next));
    pan = { x: cx - ((cx - pan.x) / scale) * s, y: cy - ((cy - pan.y) / scale) * s };
    scale = s;
  }
  function fit() {
    if (!view) return;
    const s = Math.min(2.5, Math.max(0.4, Math.min(view.clientWidth / W, view.clientHeight / H)));
    scale = s;
    pan = { x: (view.clientWidth - W * s) / 2, y: (view.clientHeight - H * s) / 2 };
  }
  // Fitting a long pipeline makes it unreadable, so only fit when it stays
  // legible; otherwise open at 100% on the live stage, snapped to a stage
  // boundary so we never frame a half-sliced column.
  function frame() {
    if (!view || !jobs.length) return;
    const sw = view.clientWidth / W, sh = view.clientHeight / H;
    if (Math.min(sw, sh) >= 0.78) return fit();
    const live = jobs.find((j) => j.status === 'running') ?? jobs.find((j) => j.status === 'failed') ?? jobs[0];
    const col = Math.max(0, stages.indexOf(live.stage) - 1);
    scale = Math.max(0.4, Math.min(1, sh));
    pan = { x: -col * (NW + COL_GAP) * scale, y: 0 };
  }

  // re-frame when the graph itself changes, not on every unrelated update
  const shape = $derived(jobs.map((j) => `${j.name}:${j.status}`).join('|'));
  $effect(() => {
    void shape;
    requestAnimationFrame(frame);
  });

  function sub(j: Job): string {
    if (j.status === 'pending') {
      const waiting = (j.needs ?? []).filter((n) => jobs.find((x) => x.name === n)?.status !== 'passed');
      return waiting.length ? `waiting on ${waiting.join(', ')}` : 'queued · no free worker';
    }
    return j.worker ?? '—';
  }
  function dur(j: Job): string {
    if (j.started_at && j.finished_at) return fmtDur(j.finished_at - j.started_at);
    if (j.started_at) return fmtDur(Date.now() - j.started_at);
    return '—';
  }
</script>

<div
  class="dagview"
  bind:this={view}
  role="application"
  aria-label="Pipeline stage graph"
  onpointerdown={(e) => {
    if ((e.target as HTMLElement).closest('.dagctl')) return;
    drag = { x: e.clientX - pan.x, y: e.clientY - pan.y, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }}
  onpointermove={(e) => {
    if (!drag) return;
    const nx = e.clientX - drag.x, ny = e.clientY - drag.y;
    if (Math.abs(nx - pan.x) > 3 || Math.abs(ny - pan.y) > 3) drag.moved = true;
    pan = { x: nx, y: ny };
  }}
  onpointerup={(e) => {
    const node = (e.target as HTMLElement).closest('[data-job]');
    // a drag that happens to end on a node must not count as a click
    if (node && drag && !drag.moved) {
      const j = jobs.find((x) => x.name === (node as HTMLElement).dataset.job);
      if (j) onselect?.(j);
    }
    drag = null;
  }}
  onwheel={(e) => {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setScale(scale * (e.deltaY < 0 ? 1.12 : 0.89), e.clientX - r.left, e.clientY - r.top);
  }}
  style="height:{height}px"
>
  <div class="dagstage" style="transform:translate({pan.x}px,{pan.y}px) scale({scale})">
    <svg width={W} height={H} aria-hidden="true">
      {#each edges as e, i (i)}
        <path class="edge" class:live={e.live} d={e.d} stroke={e.stroke} />
      {/each}
    </svg>

    {#each stages as s, ci (s)}
      {@const js = jobs.filter((j) => j.stage === s)}
      {@const done = js.filter((j) => j.status === 'passed').length}
      <div class="stagelbl" style="left:{PAD + ci * (NW + COL_GAP)}px;top:4px;width:{NW}px">
        <span class="lbl">{s}</span>
        <span class="cnt">
          {done}/{js.length} ·
          {js.some((j) => j.status === 'failed')
            ? 'failed'
            : js.some((j) => j.status === 'running')
              ? 'running'
              : done === js.length
                ? 'done'
                : 'queued'}
        </span>
      </div>
    {/each}

    {#each jobs as j (j.id)}
      {@const p = pos.get(j.name)}
      {#if p}
        <button
          class="node {j.status}"
          data-job={j.name}
          style="left:{p.x}px;top:{p.y}px;width:{NW}px;height:{NH}px"
        >
          <span class="n1">
            <span class="g {j.status}">{GLYPH[j.status] ?? ''}</span>
            <span class="name">{j.name}</span>
            <span class="dur">{dur(j)}</span>
          </span>
          <span class="n2">{sub(j)}</span>
        </button>
      {/if}
    {/each}
  </div>

  <div class="daghint">drag to pan · ⌘ + scroll to zoom</div>
  <div class="dagctl">
    <button title="Zoom out" onclick={() => view && setScale(scale / 1.2, view.clientWidth / 2, view.clientHeight / 2)}>−</button>
    <button class="pct" title="Reset to 100%" onclick={() => { scale = 1; pan = { x: 0, y: 0 }; }}>
      {Math.round(scale * 100)}%
    </button>
    <button title="Zoom in" onclick={() => view && setScale(scale * 1.2, view.clientWidth / 2, view.clientHeight / 2)}>+</button>
    <button title="Fit pipeline" onclick={fit}>⤢</button>
  </div>
</div>

<style>
  .dagview {
    position: relative;
    flex: none;
    overflow: hidden;
    cursor: grab;
    touch-action: none;
    background-image: radial-gradient(oklch(0.885 0.006 110) 1px, transparent 1px);
    background-size: 17px 17px;
    background-position: -1px -1px;
  }
  .dagview:active { cursor: grabbing; }
  .dagview::after {
    content: '';
    position: absolute;
    inset: 0 0 0 auto;
    width: 56px;
    pointer-events: none;
    background: linear-gradient(90deg, transparent, var(--card));
  }
  .dagstage { position: absolute; top: 0; left: 0; transform-origin: 0 0; will-change: transform; }
  .dagstage > svg { position: absolute; top: 0; left: 0; overflow: visible; pointer-events: none; }
  .stagelbl {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 7px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--line-2);
  }
  .lbl {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--ink-2);
  }
  .cnt { margin-left: auto; font-family: var(--mono); font-size: 10px; color: var(--faint); }
  .node {
    position: absolute;
    border: 1px solid var(--line-2);
    border-radius: 8px;
    padding: 9px 10px;
    background: var(--card);
    box-shadow: 0 1px 2px oklch(0.2 0.01 110 / 0.05);
    text-align: left;
    font: inherit;
    cursor: pointer;
    display: block;
  }
  .node:hover { border-color: var(--brand); }
  .node.passed { border-color: oklch(0.86 0.05 145); background: oklch(0.985 0.012 145); }
  .node.running {
    border-color: oklch(0.83 0.09 82);
    background: oklch(0.99 0.02 85);
    box-shadow: 0 0 0 3px oklch(0.83 0.09 82 / 0.14);
  }
  .node.failed { border-color: oklch(0.86 0.07 27); background: oklch(0.99 0.015 27); }
  .node.pending { background: var(--surface); border-style: dashed; }
  .n1 { display: flex; align-items: center; gap: 7px; }
  .name { font-size: 13px; font-weight: 600; letter-spacing: -0.01em; white-space: nowrap; }
  .dur { margin-left: auto; font-family: var(--mono); font-size: 10.5px; color: var(--muted); }
  .n2 {
    margin-top: 3px;
    font-size: 10.5px;
    color: var(--faint);
    font-family: var(--mono);
    line-height: 1.4;
    display: block;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .g {
    font-size: 11px;
    line-height: 1;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: #fff;
    flex: none;
  }
  .g.passed { background: var(--ok); }
  .g.failed { background: var(--fail); }
  .g.running { background: var(--run); animation: blink 1.6s infinite; }
  .g.pending { background: transparent; border: 1.5px solid oklch(0.62 0.01 110); color: transparent; }
  @keyframes blink { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

  :global(.edge) { fill: none; stroke-width: 1.6; }
  :global(.edge.live) { stroke-dasharray: 5 4; animation: march 700ms linear infinite; }
  @keyframes march { to { stroke-dashoffset: -9; } }
  @media (prefers-reduced-motion: reduce) {
    :global(.edge.live) { animation: none; }
    .g.running { animation: none; }
  }

  .daghint {
    position: absolute;
    left: 12px;
    bottom: 13px;
    font-size: 10.5px;
    color: var(--muted);
    font-family: var(--mono);
    pointer-events: none;
    background: var(--card);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 3px 8px;
    opacity: 0;
    transition: opacity 0.15s;
  }
  .dagview:hover .daghint { opacity: 1; }
  .dagctl {
    position: absolute;
    right: 12px;
    bottom: 12px;
    display: flex;
    align-items: center;
    gap: 2px;
    background: var(--card);
    border: 1px solid var(--line-2);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 3px;
  }
  .dagctl button {
    border: 0;
    background: none;
    font: inherit;
    font-size: 13px;
    color: var(--ink-2);
    width: 26px;
    height: 24px;
    border-radius: 5px;
    cursor: pointer;
    line-height: 1;
  }
  .dagctl button:hover { background: var(--surface); color: var(--ink); }
  .dagctl .pct { width: 46px; font-family: var(--mono); font-size: 11px; font-weight: 600; }
</style>
