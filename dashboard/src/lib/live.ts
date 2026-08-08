// The one copy of live cluster state, shared by every page.
//
// Each page used to own a `let overview = $state(null)` and its own poller.
// Two things went wrong with that. Navigating reset the value to null, so the
// status rail read "Degraded" for the length of one request on every single
// route change — a red flash that made hash routing feel like a page load. And
// six pages each polling /api/runs + /api/workers meant the same two queries ran
// several times over per tick for no extra information.
//
// Hoisting it here fixes both: the value outlives any page, so a route change
// renders immediately from what is already known, and there is exactly one
// request cycle no matter how many components are reading it.

import { writable } from 'svelte/store';
import { api } from './api';
import { lastFetch } from './poll';
import type { Overview } from './types';

const POLL_MS = 3000;

const store = writable<Overview | null>(null);
const errorStore = writable('');

/** Live cluster state: workers + recent runs. Null only before the first load. */
export const overview = { subscribe: store.subscribe };
/** Last polling failure, or '' while healthy. */
export const liveError = { subscribe: errorStore.subscribe };

async function tick(): Promise<void> {
  try {
    store.set(await api.overview());
    errorStore.set('');
    lastFetch.set(Date.now());
  } catch (e) {
    // Keep the last good value on screen rather than blanking the UI: a failed
    // poll means we do not know the current state, not that the cluster is
    // empty. The rail goes stale on its own via `lastFetch`, which is honest.
    errorStore.set(`Cannot reach the data source (${(e as Error).message}). Retrying on the next poll.`);
  }
}

// One poller for the app's lifetime. Paused while the tab is hidden, and
// refreshed the moment it comes back so returning to the tab never shows a
// number that is minutes old.
void tick();
setInterval(() => {
  if (!document.hidden) void tick();
}, POLL_MS);
document.addEventListener('visibilitychange', () => {
  if (!document.hidden) void tick();
});

/** Force an immediate refresh — after a mutation, so the UI does not lag a poll. */
export function refreshNow(): Promise<void> {
  return tick();
}
