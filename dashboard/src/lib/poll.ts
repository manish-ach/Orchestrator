import { readable, writable } from 'svelte/store';

/** Wall clock updated every second — depend on $now to re-render live ages. */
export const now = readable(Date.now(), (set) => {
  const id = setInterval(() => set(Date.now()), 1000);
  return () => clearInterval(id);
});

/** Timestamp of the last successful fetch, for the "updated Ns ago" readout. */
export const lastFetch = writable(Date.now());

/**
 * Run `fn` immediately, then every `ms`, pausing while the tab is hidden.
 * Returns a stop function — call it from onDestroy.
 */
export function startPolling(fn: () => void | Promise<void>, ms = 3000): () => void {
  const run = () => {
    void Promise.resolve(fn()).then(() => lastFetch.set(Date.now()));
  };
  run();
  const id = setInterval(() => {
    if (!document.hidden) run();
  }, ms);
  const onVis = () => {
    if (!document.hidden) run();
  };
  document.addEventListener('visibilitychange', onVis);
  return () => {
    clearInterval(id);
    document.removeEventListener('visibilitychange', onVis);
  };
}
