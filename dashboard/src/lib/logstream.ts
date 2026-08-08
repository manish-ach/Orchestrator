// Live job logs.
//
// The coordinator serves a job's log as server-sent events, each carrying only
// the bytes written since the previous one. That is the difference between a
// log that appears while a build runs and one that appears after it: polling
// `/logs` on the 3s page cadence shows a finished stage, not a running one.
//
// In mock mode there is no server to stream from, so this falls back to polling
// the mock API at the same effective cadence. The caller does not have to know
// which one it got.

import { API_BASE, ENDPOINTS, MODE, getToken, api } from './api';

export interface LogStream {
  /** stop streaming and release the connection */
  close(): void;
}

const MOCK_POLL_MS = 700;

/**
 * Stream `job`'s log, calling `onText` with each new chunk (never the whole log
 * again) and `onEnd` once the job is terminal.
 *
 * EventSource cannot send an Authorization header, so when a session token
 * exists it goes in the query string — the coordinator accepts it there for
 * this one endpoint. That keeps live logs live with dashboard auth turned on,
 * instead of silently degrading to polling.
 */
export function streamJobLog(
  runId: number,
  jobId: number,
  onText: (chunk: string) => void,
  onEnd?: () => void,
): LogStream {
  if (MODE === 'mock') return pollFallback(runId, jobId, onText, onEnd);

  const token = getToken();
  const url = `${API_BASE}${ENDPOINTS.jobLogStream(jobId)}${token ? `?token=${encodeURIComponent(token)}` : ''}`;

  let closed = false;
  const src = new EventSource(url);
  let fallback: LogStream | null = null;

  src.addEventListener('log', (e) => onText((e as MessageEvent<string>).data));
  src.addEventListener('end', () => {
    src.close();
    if (!closed) onEnd?.();
  });
  src.onerror = () => {
    // a coordinator too old for the endpoint, or a dropped connection: take the
    // slow path rather than leaving the pane empty
    src.close();
    if (!closed && !fallback) fallback = pollFallback(runId, jobId, onText, onEnd);
  };

  return {
    close() {
      closed = true;
      src.close();
      fallback?.close();
    },
  };
}

/** Re-reads the whole log and emits only what grew, so callers see one shape. */
function pollFallback(
  runId: number,
  jobId: number,
  onText: (chunk: string) => void,
  onEnd?: () => void,
): LogStream {
  let sent = 0;
  let stopped = false;
  const tick = async () => {
    if (stopped) return;
    try {
      const d = await api.job(runId, jobId);
      if (stopped || !d) return;
      const text = d.log.map((l) => l.t).join('\n');
      if (text.length > sent) {
        onText(text.slice(sent));
        sent = text.length;
      }
      if (d.job.status === 'passed' || d.job.status === 'failed') {
        stopped = true;
        onEnd?.();
        return;
      }
    } catch {
      /* the page's own error banner covers an unreachable coordinator */
    }
    if (!stopped) timer = window.setTimeout(tick, MOCK_POLL_MS);
  };
  let timer = window.setTimeout(tick, 0);
  return {
    close() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}
