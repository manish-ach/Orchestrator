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
  repo: (name: string) => `/api/repos/${encodeURIComponent(name)}`, // DELETE = unregister
  repoPipeline: (name: string, file?: string) =>
    `/api/repos/${encodeURIComponent(name)}/pipeline${file ? `?file=${encodeURIComponent(file)}` : ''}`,
  calendar: '/api/activity/calendar',
} as const;

// ---- auth -------------------------------------------------------------------
// The coordinator requires a login when DASHBOARD_USERNAME/PASSWORD are set
// in its env. The session token lives in localStorage; every request sends
// it as a Bearer header. A 401 anywhere kicks the app back to the login
// screen via the `dash:unauthorized` event App.svelte listens for.

const TOKEN_KEY = 'dash.token';

export const getToken = (): string | null => localStorage.getItem(TOKEN_KEY);
export const clearToken = (): void => localStorage.removeItem(TOKEN_KEY);

function authHeaders(): Record<string, string> {
  const token = getToken();
  return token ? { Authorization: `Bearer ${token}` } : {};
}

function handleUnauthorized(res: Response): void {
  if (res.status === 401 && MODE === 'live') {
    clearToken();
    window.dispatchEvent(new CustomEvent('dash:unauthorized'));
  }
}

/** Does the coordinator want a login? Mock mode and dead coordinators don't. */
export async function authRequired(): Promise<boolean> {
  if (MODE !== 'live') return false;
  try {
    const res = await fetch(`${API_BASE}/api/auth/status`, { headers: { Accept: 'application/json' } });
    const body = (await res.json()) as { required?: boolean };
    return !!body.required;
  } catch {
    return false; // unreachable coordinator — let the pages show their error banner
  }
}

export async function login(username: string, password: string): Promise<void> {
  const res = await fetch(`${API_BASE}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) {
    const reason = await res.text().catch(() => '');
    throw new Error(reason || 'login failed');
  }
  const { token } = (await res.json()) as { token: string };
  localStorage.setItem(TOKEN_KEY, token);
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    headers: { Accept: 'application/json', ...authHeaders() },
  });
  if (!res.ok) {
    handleUnauthorized(res);
    throw new Error(`${path} -> ${res.status}`);
  }
  return res.json() as Promise<T>;
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    method: 'POST',
    headers: {
      ...authHeaders(),
      ...(body !== undefined && { 'Content-Type': 'application/json' }),
    },
    ...(body !== undefined && { body: JSON.stringify(body) }),
  });
  if (!res.ok) {
    handleUnauthorized(res);
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

  async deleteRepo(name: string): Promise<void> {
    const res = await fetch(`${API_BASE}${ENDPOINTS.repo(name)}`, {
      method: 'DELETE',
      headers: authHeaders(),
    });
    if (!res.ok) {
      handleUnauthorized(res);
      const reason = await res.text().catch(() => '');
      throw new Error(reason || `${ENDPOINTS.repo(name)} -> ${res.status}`);
    }
  },

  async pipelineFile(repo: string, file?: string): Promise<{ file: string; content: string }> {
    return get(ENDPOINTS.repoPipeline(repo, file));
  },

  async calendar(): Promise<CalendarDay[]> {
    return get<CalendarDay[]>(ENDPOINTS.calendar);
  },
};

export const api: Api = MODE === 'live' ? liveApi : mockApi;
