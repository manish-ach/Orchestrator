<script lang="ts">
  import { MODE } from '../api';
  import { lastFetch, now } from '../poll';
  import type { Overview } from '../types';
  import Palette from './Palette.svelte';

  let {
    active,
    overview = null,
    children,
  }: {
    active: 'control' | 'runs' | 'repos' | 'workers' | 'insights';
    overview?: Overview | null;
    children?: import('svelte').Snippet;
  } = $props();

  // Persisted so a collapsed sidebar stays collapsed across navigations. It is a
  // pure UI preference, so localStorage rather than a cookie — no reason for it
  // to ride along on every request to the coordinator.
  const KEY = 'orch.sidebar';
  let collapsed = $state(read());
  function read(): boolean {
    try {
      return localStorage.getItem(KEY) === 'collapsed';
    } catch {
      return false;
    }
  }
  function setCollapsed(next: boolean) {
    collapsed = next;
    try {
      localStorage.setItem(KEY, next ? 'collapsed' : 'open');
    } catch {
      /* private mode: the toggle still works, it just will not persist */
    }
    // charts and the DAG canvas size themselves from their box
    requestAnimationFrame(() => window.dispatchEvent(new Event('resize')));
  }

  let paletteOpen = $state(false);

  const online = $derived(overview?.workers.filter((w) => w.status === 'online').length ?? 0);
  const total = $derived(overview?.workers.length ?? 0);
  const jobs = $derived(overview?.runs.flatMap((r) => r.jobs) ?? []);
  const queued = $derived(jobs.filter((j) => j.status === 'pending' && j.ready_at !== null).length);
  const offline = $derived(total - online);
  const stale = $derived(Math.max(0, Math.round(($now - $lastFetch) / 1000)));
  // "healthy" has to mean something: it is false while any worker is missing or
  // the last poll failed, otherwise the rail would reassure during an outage.
  const healthy = $derived(overview !== null && offline === 0 && stale < 15);

  const NAV = [
    { key: 'control', label: 'Control center', href: '#/' },
    { key: 'runs', label: 'Runs', href: '#/runs' },
    { key: 'repos', label: 'Repositories', href: '#/repos' },
    { key: 'workers', label: 'Workers', href: '#/workers' },
    { key: 'insights', label: 'Insights', href: '#/insights', group: 'Analysis' },
  ] as const;

  const counts = $derived<Record<string, string>>({
    runs: overview ? String(overview.runs.length) : '',
    repos: overview ? String(new Set(overview.runs.map((r) => r.repo)).size) : '',
    workers: total ? `${online}/${total}` : '',
  });
</script>

<svelte:window
  onkeydown={(e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      paletteOpen = !paletteOpen;
    }
  }}
/>

<div class="app">
  <aside class="side" class:collapsed>
    <div class="brand">
      <button
        class="markbtn"
        title={collapsed ? 'Open sidebar' : 'Orchestrator'}
        aria-label="Toggle sidebar"
        onclick={() => collapsed && setCollapsed(false)}
      >
        <svg class="mark" viewBox="0 0 32 32" aria-hidden="true">
          <rect width="32" height="32" rx="7" fill="var(--lime)" />
          <path
            d="M11 16h4c2.5 0 2.5-5 5-5M11 16h4c2.5 0 2.5 5 5 5"
            fill="none"
            stroke="oklch(0.205 0.020 118)"
            stroke-width="2.2"
            stroke-linecap="round"
          />
          <circle cx="9.5" cy="16" r="3" fill="oklch(0.205 0.020 118)" />
          <circle cx="21.5" cy="11" r="3" fill="oklch(0.205 0.020 118)" />
          <circle cx="21.5" cy="21" r="3" fill="oklch(0.205 0.020 118)" />
        </svg>
        <svg class="panel-ico" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
          <rect x="2.5" y="3.5" width="15" height="13" rx="2" /><path d="M7.5 3.5v13" />
        </svg>
      </button>
      <span class="brand-words"><b>Orchestrator</b><span>ci · cd</span></span>
      <button class="collapse" title="Collapse sidebar" aria-label="Collapse sidebar" onclick={() => setCollapsed(true)}>
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6">
          <rect x="2.5" y="3.5" width="15" height="13" rx="2" /><path d="M7.5 3.5v13" />
        </svg>
      </button>
    </div>

    <nav aria-label="Pages">
      {#each NAV as item (item.key)}
        {#if 'group' in item}<div class="navlbl">{item.group}</div>{/if}
        <a class="navitem" class:on={active === item.key} href={item.href} title={item.label}>
          {#if item.key === 'control'}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="1.5" y="1.5" width="5.5" height="5.5" rx="1"/><rect x="9" y="1.5" width="5.5" height="5.5" rx="1"/><rect x="1.5" y="9" width="5.5" height="5.5" rx="1"/><rect x="9" y="9" width="5.5" height="5.5" rx="1"/></svg>
          {:else if item.key === 'runs'}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="6.2"/><path d="M6.4 5.5v5l4-2.5z" fill="currentColor" stroke="none"/></svg>
          {:else if item.key === 'repos'}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M5.5 5.5 3 8l2.5 2.5M10.5 5.5 13 8l-2.5 2.5"/></svg>
          {:else if item.key === 'workers'}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="2.5" width="12" height="4.5" rx="1"/><rect x="2" y="9" width="12" height="4.5" rx="1"/><path d="M4.5 4.75h.01M4.5 11.25h.01"/></svg>
          {:else}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 13.5h12"/><rect x="3" y="8" width="2.6" height="4"/><rect x="7" y="4.5" width="2.6" height="7.5"/><rect x="11" y="6.5" width="2.6" height="5.5"/></svg>
          {/if}
          <span class="txt">{item.label}</span>
          {#if counts[item.key]}<span class="cnt">{counts[item.key]}</span>{/if}
        </a>
      {/each}
    </nav>

    <div class="sfoot">
      <div class="row">
        <span class="dot-live" class:up={overview !== null} class:down={overview === null}></span>
        <span class="txt">{overview ? 'coordinator ok' : 'no data'}</span>
      </div>
      <div class="row row-2">
        <span class="txt">auto-refresh 3s</span>
        <span class="chip-mode" style="margin-left:auto">{MODE.toUpperCase()}</span>
      </div>
    </div>
  </aside>

  <div class="app-main">
    <div class="rail">
      <div class="grp">
        <span class="dot-live" class:up={healthy} class:down={!healthy}></span>
        <b>{healthy ? 'All systems healthy' : overview ? 'Degraded' : 'Waiting for data'}</b>
      </div>
      <div class="sep"></div>
      <div class="grp">
        <b class="mono">{online}</b><span style="color:var(--faint)">/{total} workers online</span>
      </div>
      <div class="sep"></div>
      <div class="grp">
        <span style="color:var(--faint)">queue</span><b class="mono">{queued}</b><span style="color:var(--faint)">pending</span>
      </div>
      <div class="sep"></div>
      <div class="grp">
        <span class="pulse"></span>
        <span style="color:var(--faint)">live · updated</span><b class="mono">{stale}s ago</b>
      </div>
      <button class="railsearch" onclick={() => (paletteOpen = true)}>
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6">
          <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5 14 14" />
        </svg>
        Search runs, repos, workers
        <span class="kbd">⌘K</span>
      </button>
    </div>

    {@render children?.()}
  </div>
</div>

<Palette bind:open={paletteOpen} {overview} />
