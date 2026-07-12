// Tiny hash router: #/run/3?job=12 → { path: ['run', '3'], query: {job: '12'} }.
// Hash routing keeps the build a plain static bundle — any file server
// (python http.server, axum ServeDir) can host it with zero fallback config.

import { writable } from 'svelte/store';

export interface Route {
  path: string[];
  query: URLSearchParams;
}

function parse(): Route {
  const hash = location.hash.replace(/^#\/?/, '');
  const [p = '', q = ''] = hash.split('?');
  return { path: p.split('/').filter(Boolean), query: new URLSearchParams(q) };
}

export const route = writable<Route>(parse());

window.addEventListener('hashchange', () => route.set(parse()));

export function navigate(to: string): void {
  location.hash = to.startsWith('#') ? to : `#${to}`;
}

export function back(fallback: string): void {
  if (history.length > 1) history.back();
  else navigate(fallback);
}
