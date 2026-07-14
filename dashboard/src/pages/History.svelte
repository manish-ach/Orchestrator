<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, MODE } from '../lib/api';
  import Strip from '../lib/components/Strip.svelte';
  import Topbar from '../lib/components/Topbar.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { now, startPolling } from '../lib/poll';
  import type { Overview, Run, RunStatus } from '../lib/types';

  const SHOW_MAX = 50;

  let overview = $state<Overview | null>(null);
  let error = $state('');
  // filter state lives outside the polled data so typing survives refreshes
  let query = $state('');
  let statusFilter = $state<'all' | RunStatus>('all');

  const stop = startPolling(async () => {
    try {
      overview = await api.overview();
      error = '';
    } catch (e) {
      error = `Cannot reach the data source (${(e as Error).message}). Retrying on the next poll.`;
    }
  });
  onDestroy(stop);

  const runs = $derived([...(overview?.runs ?? [])].sort((a, b) => b.started_at - a.started_at));
  const counts = $derived<Record<'all' | RunStatus, number>>({
    all: runs.length,
    passed: runs.filter((r) => r.status === 'passed').length,
    failed: runs.filter((r) => r.status === 'failed').length,
    running: runs.filter((r) => r.status === 'running').length,
    pending: runs.filter((r) => r.status === 'pending').length,
  });

  function matches(r: Run, q: string): boolean {
    const hay = [
      r.commit?.message,
      r.commit?.sha,
      r.commit?.author,
      r.repo,
      r.pipeline,
      `#${r.id}`,
      r.trigger,
      r.status,
    ]
      .filter(Boolean)
      .join(' ')
      .toLowerCase();
    return hay.includes(q);
  }

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return runs
      .filter((r) => statusFilter === 'all' || r.status === statusFilter)
      .filter((r) => !q || matches(r, q));
  });
  const shown = $derived(filtered.slice(0, SHOW_MAX));

  function duration(r: Run): number {
    return (r.finished_at ?? $now) - r.started_at;
  }

  const FILTERS: ('all' | RunStatus)[] = ['all', 'passed', 'failed', 'running'];
</script>

<Topbar active="runs" {overview}>
  <div class="wrap">
    <div class="page-head">
      <h1>Runs</h1>
      <span class="meta">
        every pipeline run, newest first · {counts.passed} passed · {counts.failed} failed
      </span>
    </div>
  </div>
</Topbar>

<main class="wrap">
  {#if error}<div class="err-banner">{error}</div>{/if}

  <div class="hist-controls">
    <div class="hist-chips" role="group" aria-label="Filter by status">
      {#each FILTERS as f (f)}
        <button
          type="button"
          class="hchip"
          class:active={statusFilter === f}
          onclick={() => (statusFilter = f)}
        >
          {#if f !== 'all'}<span class="g {f}" aria-hidden="true">{GLYPH[f]}</span>{/if}
          {f}
          <span class="hcount">{counts[f]}</span>
        </button>
      {/each}
    </div>
    <input
      class="hist-search"
      type="search"
      placeholder="Filter by commit, repo, author, #id, trigger…"
      aria-label="Search runs"
      bind:value={query}
    />
  </div>

  <section class="hist-list" aria-label="Run history">
    {#if overview && !shown.length}
      <div class="empty">
        {#if runs.length}
          No runs match — clear the search or pick another status.
        {:else}
          No runs yet — push to a registered repo and the history fills in here.
        {/if}
      </div>
    {/if}
    {#each shown as r (r.id)}
      <a class="hrow" href="#/run/{r.id}">
        <span class="g {r.status}" aria-hidden="true">{GLYPH[r.status] ?? '·'}</span>
        <span class="hbody">
          <span class="htitle">{r.commit?.message ?? `Run #${r.id}`}</span>
          <span class="hsub">
            <b>{r.pipeline}</b> #{r.id}
            {#if r.commit}
              · <span class="sha-chip">{r.commit.sha}</span> by {r.commit.author}
            {/if}
            · via {r.trigger}
          </span>
        </span>
        <span class="hside">
          <span class="hmeta-col">
            <span class="repo-tag">{r.repo ?? r.pipeline}</span>
            <Strip jobs={r.jobs} />
          </span>
          <span class="htimes">
            <span class="hdur" title="duration">{fmtDur(duration(r))}</span>
            <span class="hage">{ago(r.started_at, $now)}</span>
          </span>
        </span>
      </a>
    {/each}
  </section>

  <footer class="foot">
    showing {shown.length} of {filtered.length} matching · {runs.length} total · mode: {MODE} · polling every 3s
  </footer>
</main>
