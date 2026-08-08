// Derived activity data, plus the one canvas chart left.
//
// Activity at any instant is derived from job started/finished timestamps, so
// history is exact and survives a reload — there is no sampling daemon to miss
// anything. `drawStatChart` is the exception: CPU/RAM cannot be derived from
// timestamps, so those come from heartbeat samples the coordinator stores.

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

const LIGHT = {
  text: 'oklch(0.47 0.014 110)',
  grid: 'oklch(0.945 0.004 110)',
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
 * One heartbeat metric over time, as a filled line on a 0–100% axis.
 *
 * Draws a separate path per contiguous stretch of samples rather than one
 * continuous line: a worker that went offline for two minutes should leave a
 * gap, not a straight line implying it was idling through the outage.
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
