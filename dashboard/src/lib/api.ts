// The ONLY file that knows about HTTP. Every coordinator URL lives in
// ENDPOINTS; every page imports `api` from here and never builds a URL or
// calls fetch() itself. Two modes:
//   live — the Rust coordinator's REST API (default)
//   mock — simulated coordinator, for demos without a backend
// Switch with ?mode=live / ?mode=mock on any page (persists in localStorage);
// change the coordinator address with
//   localStorage.setItem('dash.apiBase', 'http://vm:8080').

import { mockApi } from './mock';
import type { Api, CalendarDay, JobDetail, Overview, Repo, Run, Worker } from './types';

const qs = new URLSearchParams(location.search);
if (qs.get('mode')) localStorage.setItem('dash.mode', qs.get('mode')!);

export const MODE: 'mock' | 'live' =
  localStorage.getItem('dash.mode') === 'mock' ? 'mock' : 'live';
// Served by the coordinator itself → same origin. On the Vite dev server
// (port 4173) → the local coordinator.
export const API_BASE =
  localStorage.getItem('dash.apiBase') ??
  (location.port === '4173' ? 'http://127.0.0.1:8080' : '');

/**
 * Every coordinator URL the dashboard knows about, in one place.
 * The full request/response contract is documented in dashboard/README.md.
 */
export const ENDPOINTS = {
  health: '/api/health',
  workers: '/api/workers',
  jobs: '/api/jobs',
  trigger: '/api/pipelines/trigger', // POST, optional { repo } body
  runs: '/api/runs',
  run: (id: number | string) => `/api/runs/${id}`,
  jobLogs: (id: number | string) => `/api/jobs/${id}/logs`,
  repos: '/api/repos', // GET = list, POST { remote } = register a Forgejo repo
  calendar: '/api/activity/calendar',
} as const;

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, { headers: { Accept: 'application/json' } });
  if (!res.ok) throw new Error(`${path} -> ${res.status}`);
  return res.json() as Promise<T>;
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    method: 'POST',
    ...(body !== undefined && {
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }),
  });
  if (!res.ok) {
    // the coordinator answers 4xx with a human-readable reason
    const reason = await res.text().catch(() => '');
    throw new Error(reason || `${path} -> ${res.status}`);
  }
  return res.json() as Promise<T>;
}

// ---- live adapter -----------------------------------------------------------

const liveApi: Api = {
  async overview(): Promise<Overview> {
    const [workers, runs] = await Promise.all([
      get<Worker[]>(ENDPOINTS.workers),
      get<Run[]>(ENDPOINTS.runs),
    ]);
    return { workers, runs };
  },

  async run(id): Promise<Run | null> {
    try {
      return await get<Run>(ENDPOINTS.run(id));
    } catch {
      return null;
    }
  },

  async job(runId, jobId): Promise<JobDetail | null> {
    const run = await this.run(runId);
    const job = run?.jobs.find((j) => j.id === Number(jobId));
    if (!run || !job) return null;
    const { output } = await get<{ output: string }>(ENDPOINTS.jobLogs(jobId));
    const raw = output || '(no output captured for this job yet)';
    return { run, job, log: raw.split('\n').map((t) => ({ t, err: false, ok: false })) };
  },

  async trigger(repo?: string) {
    return post<{ id: number }>(ENDPOINTS.trigger, repo ? { repo } : undefined);
  },

  async repos(): Promise<Repo[]> {
    return get<Repo[]>(ENDPOINTS.repos);
  },

  async addRepo(remote: string): Promise<Repo> {
    return post<Repo>(ENDPOINTS.repos, { remote });
  },

  async calendar(): Promise<CalendarDay[]> {
    return get<CalendarDay[]>(ENDPOINTS.calendar);
  },
};

export const api: Api = MODE === 'live' ? liveApi : mockApi;
