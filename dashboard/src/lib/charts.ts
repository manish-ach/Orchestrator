// Derived activity data + the two canvas charts still in use.
// Activity at any instant is derived from job started/finished timestamps,
// so history is exact and works after a reload — no sampling daemon.

import type { Job, Run, StatSample, Worker } from './types';

export interface BusyInterval {
  start: number;
  end: number;
  open: boolean;
  status: Job['status'];
  job: Job;
  run: Run;
}

export interface Activity {
  t0: number;
  now: number;
  windowMs: number;
  byWorker: Map<string, BusyInterval[]>;
}

/** Collect each worker's busy intervals inside [now-window, now]. */
export function activity(
  runs: Run[],
  workers: Pick<Worker, 'name'>[],
  now: number,
  windowMs: number,
): Activity {
  const t0 = now - windowMs;
  const byWorker = new Map<string, BusyInterval[]>(workers.map((w) => [w.name, []]));
  for (const run of runs) {
    for (const j of run.jobs) {
      if (!j.started_at || !j.worker) continue;
      const end = j.finished_at ?? now;
      if (end < t0 || j.started_at > now) continue;
      byWorker.get(j.worker)?.push({
        start: Math.max(j.started_at, t0),
        end: Math.min(end, now),
        open: !j.finished_at,
        status: j.status,
        job: j,
        run,
      });
    }
  }
  for (const list of byWorker.values()) list.sort((a, b) => a.start - b.start);
  return { t0, now, windowMs, byWorker };
}

export interface DeviceStats {
  jobs: number;
  passed: number;
  failed: number;
  running: number;
  util: number;
}

/** Per-device numbers over the window: job counts by outcome + utilization %. */
export function deviceStats(intervals: BusyInterval[], act: Activity): DeviceStats {
  let busyMs = 0;
  let passed = 0;
  let failed = 0;
  let running = 0;
  for (const iv of intervals) {
    busyMs += iv.end - iv.start;
    if (iv.status === 'failed') failed++;
    else if (iv.status === 'running') running++;
    else passed++;
  }
  return {
    jobs: intervals.length,
    passed,
    failed,
    running,
    util: Math.min(100, Math.round((busyMs / act.windowMs) * 100)),
  };
}

/** Fraction of each time bucket the given jobs kept a worker busy. */
export function utilizationSeries(jobs: Job[], t0: number, t1: number, buckets = 44): number[] {
  const total = Math.max(t1 - t0, 1000);
  const step = total / buckets;
  return Array.from({ length: buckets }, (_, i) => {
    const bs = t0 + i * step;
    const be = bs + step;
    let busy = 0;
    for (const j of jobs) {
      if (!j.started_at) continue;
      const s = Math.max(j.started_at, bs);
      const e = Math.min(j.finished_at ?? t1, be);
      if (e > s) busy += e - s;
    }
    return busy / step;
  });
}

const LIGHT = {
  text: 'oklch(0.47 0.014 110)',
  grid: 'oklch(0.945 0.004 110)',
  brand: 'oklch(0.52 0.105 112)',
  brandFill: 'oklch(0.52 0.105 112 / 0.18)',
  passed: 'oklch(0.55 0.13 145)',
  failed: 'oklch(0.53 0.19 27)',
};

function prep(canvas: HTMLCanvasElement) {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  if (canvas.width !== Math.round(w * dpr)) canvas.width = Math.round(w * dpr);
  if (canvas.height !== Math.round(h * dpr)) canvas.height = Math.round(h * dpr);
  const ctx = canvas.getContext('2d')!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  return { ctx, w, h };
}

/**
 * Utilization-over-time line chart (CPU-monitor style): values are 0..1 per
 * bucket; y axis 0–100% with gridlines, x axis labelled via opts.
 */
export function drawUtilChart(
  canvas: HTMLCanvasElement,
  values: number[],
  opts: { totalLabel?: string; midLabel?: string } = {},
): void {
  const C = LIGHT;
  const { ctx, w, h } = prep(canvas);
  const padL = 36;
  const padT = 6;
  const padB = 16;
  const cw = w - padL - 8;
  const ch = h - padT - padB;
  ctx.font = '9.5px ui-monospace, Menlo, Consolas, monospace';
  ([[1, '100%'], [0.5, '50%'], [0, '0%']] as const).forEach(([f, label]) => {
    const y = Math.round(padT + (1 - f) * ch) + 0.5;
    ctx.strokeStyle = C.grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padL, y);
    ctx.lineTo(w - 8, y);
    ctx.stroke();
    ctx.fillStyle = C.text;
    ctx.fillText(label, 4, y + 3);
  });
  const step = cw / Math.max(values.length - 1, 1);
  ctx.beginPath();
  values.forEach((v, i) => {
    const x = padL + i * step;
    const y = padT + (1 - Math.min(v, 1)) * ch;
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  });
  ctx.strokeStyle = C.brand;
  ctx.lineWidth = 1.5;
  ctx.lineJoin = 'round';
  ctx.stroke();
  ctx.lineTo(padL + cw, padT + ch);
  ctx.lineTo(padL, padT + ch);
  ctx.closePath();
  ctx.fillStyle = C.brandFill;
  ctx.fill();
  ctx.fillStyle = C.text;
  ctx.fillText('0s', padL, h - 4);
  if (opts.totalLabel) {
    const tw = ctx.measureText(opts.totalLabel).width;
    ctx.fillText(opts.totalLabel, w - 8 - tw, h - 4);
  }
  if (opts.midLabel) {
    const mw = ctx.measureText(opts.midLabel).width;
    ctx.fillText(opts.midLabel, padL + cw / 2 - mw / 2, h - 4);
  }
}

/**
 * Device-monitor line chart over real heartbeat samples: x is wall time in
 * [t0, t1], y is 0–100%. Gaps longer than a few heartbeats break the line,
 * so an offline stretch reads as missing data rather than a flat line.
 */
export function drawStatChart(
  canvas: HTMLCanvasElement,
  samples: StatSample[],
  opts: { t0: number; t1: number; metric: 'cpu' | 'mem'; stroke: string; fill: string },
): void {
  const C = LIGHT;
  const { ctx, w, h } = prep(canvas);
  const padL = 34;
  const padT = 6;
  const padB = 6;
  const cw = w - padL - 8;
  const ch = h - padT - padB;
  ctx.font = '9.5px ui-monospace, Menlo, Consolas, monospace';
  ([[1, '100%'], [0.5, '50%'], [0, '0%']] as const).forEach(([f, label]) => {
    const y = Math.round(padT + (1 - f) * ch) + 0.5;
    ctx.strokeStyle = C.grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padL, y);
    ctx.lineTo(w - 8, y);
    ctx.stroke();
    ctx.fillStyle = C.text;
    ctx.fillText(label, 2, y + 3);
  });

  const span = Math.max(opts.t1 - opts.t0, 1);
  const pts = samples.filter((s) => s.t >= opts.t0 - 4000 && s.t <= opts.t1 + 1000);
  if (!pts.length) return;
  const GAP_MS = 8000;

  // one stroked+filled path per contiguous stretch of samples
  let seg: StatSample[] = [];
  const flush = () => {
    if (seg.length < 2) { seg = []; return; }
    const xy = seg.map((s) => ({
      x: padL + ((s.t - opts.t0) / span) * cw,
      y: padT + (1 - Math.min(s[opts.metric], 100) / 100) * ch,
    }));
    ctx.beginPath();
    xy.forEach((p, i) => (i === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y)));
    ctx.strokeStyle = opts.stroke;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = 'round';
    ctx.stroke();
    ctx.lineTo(xy[xy.length - 1].x, padT + ch);
    ctx.lineTo(xy[0].x, padT + ch);
    ctx.closePath();
    ctx.fillStyle = opts.fill;
    ctx.fill();
    seg = [];
  };
  for (const s of pts) {
    if (seg.length && s.t - seg[seg.length - 1].t > GAP_MS) flush();
    seg.push(s);
  }
  flush();
}

export interface SparkSlot {
  count: number;
  failed: boolean;
}

/** Mini bar chart: one column per slot, red where the slot saw a failure. */
export function drawSparkBars(canvas: HTMLCanvasElement, slots: SparkSlot[]): void {
  const C = LIGHT;
  const { ctx, w, h } = prep(canvas);
  const max = Math.max(...slots.map((s) => s.count), 1);
  const slot = w / Math.max(slots.length, 1);
  const barW = Math.min(18, Math.max(2, slot - 2.5));
  slots.forEach((s, i) => {
    if (!s.count) return;
    const bh = Math.max(3, (s.count / max) * (h - 3));
    ctx.fillStyle = s.failed ? C.failed : C.passed;
    ctx.beginPath();
    ctx.roundRect(i * slot + (slot - barW) / 2, h - bh, barW, bh, 1.5);
    ctx.fill();
  });
}
