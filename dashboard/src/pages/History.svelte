<script lang="ts">
  import AppShell from '../lib/components/AppShell.svelte';
  import FacetDropdown from '../lib/components/FacetDropdown.svelte';
  import Strip from '../lib/components/Strip.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { liveError, overview as live } from '../lib/live';
  import { now } from '../lib/poll';
  import { route } from '../lib/router';
  import type { Job, Overview, Run, RunStatus } from '../lib/types';

  // The event log: every run, newest first. Identity varies row to row — a
  // different repo, pipeline and author each time — so those columns earn their
  // place here in a way they would not inside one pipeline's own history.
  // Shared across pages so a route change renders from what is already known
  // instead of blanking and re-fetching. `overview` stays a plain name so every
  // reference below reads the same as before.
  const overview = $derived($live);
  const error = $derived($liveError);
  let shown = $state(30);
  let sel = $state(0);
  let listEl = $state<HTMLElement | null>(null);
  let searchEl = $state<HTMLInputElement | null>(null);

  // Filter state lives outside the polled data so typing survives a refresh, and
  // mirrors into the URL so any view can be linked or bookmarked.
  let query = $state('');
  let status = $state<'all' | RunStatus>('all');
  let repo = $state('all');
  let branch = $state('all');
  let trigger = $state('all');
  let when = $state('last 7 days');

  const WHEN = ['last 24 hours', 'last 7 days', 'last 30 days', 'everything'];

  $effect(() => {
    const q = $route.query;
    status = (q.get('status') as RunStatus) ?? 'all';
    query = q.get('q') ?? '';
    repo = q.get('repo') ?? 'all';
    branch = q.get('branch') ?? 'all';
    trigger = q.get('trigger') ?? 'all';
  });

  function syncUrl() {
    const p = new URLSearchParams();
    if (status !== 'all') p.set('status', status);
    if (query) p.set('q', query);
    if (repo !== 'all') p.set('repo', repo);
    if (branch !== 'all') p.set('branch', branch);
    if (trigger !== 'all') p.set('trigger', trigger);
    const s = p.toString();
    history.replaceState(null, '', `#/runs${s ? `?${s}` : ''}`);
  }

  const runs = $derived([...(overview?.runs ?? [])].sort((a, b) => b.started_at - a.started_at));
  const repoOptions = $derived(['all', ...new Set(runs.map((r) => r.repo).filter(Boolean))]);
  const triggerOptions = $derived(['all', ...new Set(runs.map((r) => r.trigger))]);
  // Runs from before the branch column exists have none; they are grouped
  // under "unknown" rather than silently dropped from every branch filter.
  const branchOptions = $derived(['all', ...new Set(runs.map((r) => r.branch ?? 'unknown'))]);

  // Everything EXCEPT the status filter. Faceted-search rule: a facet's counts
  // show what you would get by picking it while the other facets stay put — so
  // clicking "failed 7" always yields exactly 7 rows. Counting all runs here
  // instead (the old behaviour) made the chips disagree with both the summary
  // line and the list whenever the window or repo filter was narrowing.
  const scoped = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return runs.filter(
      (r) =>
        (repo === 'all' || r.repo === repo) &&
        (branch === 'all' || (r.branch ?? 'unknown') === branch) &&
        (trigger === 'all' || r.trigger === trigger) &&
        r.started_at >= cutoff &&
        (!q || matches(r, q)),
    );
  });

  const counts = $derived({
    all: scoped.length,
    passed: scoped.filter((r) => r.status === 'passed').length,
    failed: scoped.filter((r) => r.status === 'failed').length,
    running: scoped.filter((r) => r.status === 'running').length,
  });

  const runDur = (r: Run) => (r.finished_at ?? $now) - r.started_at;

  function matches(r: Run, q: string): boolean {
    return [r.commit?.message, r.commit?.sha, r.commit?.author, r.repo, r.pipeline, r.branch, `#${r.id}`, r.trigger, r.status]
      .filter(Boolean)
      .join(' ')
      .toLowerCase()
      .includes(q);
  }
  const cutoff = $derived(
    when === 'last 24 hours'
      ? $now - 864e5
      : when === 'last 7 days'
        ? $now - 6048e5
        : when === 'last 30 days'
          ? $now - 2592e6
          : 0,
  );

  const filtered = $derived(scoped.filter((r) => status === 'all' || r.status === status));
  const visible = $derived(filtered.slice(0, shown));

  const medianDur = $derived.by(() => {
    const d = filtered.filter((r) => r.finished_at).map(runDur).sort((a, b) => a - b);
    return d.length ? d[Math.floor(d.length / 2)] : null;
  });

  // ---- day grouping ------------------------------------------------------
  const dayKey = (ts: number) => {
    const d = new Date(ts);
    return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
  };
  function dayLabel(ts: number): string {
    const k = dayKey(ts);
    if (k === dayKey($now)) return 'Today';
    if (k === dayKey($now - 864e5)) return 'Yesterday';
    return new Date(ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  }
  const groups = $derived.by(() => {
    const out: { label: string; runs: Run[] }[] = [];
    for (const r of visible) {
      const label = dayLabel(r.started_at);
      const last = out[out.length - 1];
      if (last && last.label === label) last.runs.push(r);
      else out.push({ label, runs: [r] });
    }
    return out;
  });

  // ---- why a run failed --------------------------------------------------
  // The coordinator distils this on report, so list responses can explain a
  // failure without shipping whole build logs. The client-side scan stays as a
  // fallback for runs that finished before the column existed and were not
  // caught by the backfill.
  const ERROR_RE = /(error|panicked|fatal|failed|exception|traceback)/i;
  function failure(r: Run): { job: Job; line: string } | null {
    const job = r.jobs.find((j) => j.status === 'failed');
    if (!job) return null;
    if (job.first_error) return { job, line: job.first_error };
    const lines = (job.output ?? '')
      .split('\n')
      .map((l: string) => l.trim())
      .filter(Boolean);
    const line = lines.find((l: string) => ERROR_RE.test(l)) ?? lines[lines.length - 1] ?? 'no output captured';
    return { job, line: line.length > 160 ? `${line.slice(0, 157)}…` : line };
  }

  /** Which attempt this is at the same commit, counting older runs of the pipeline. */
  function attempt(r: Run): number {
    if (!r.commit) return 1;
    const same = runs.filter((x) => x.pipeline === r.pipeline && x.commit?.sha === r.commit!.sha);
    return same.length - same.findIndex((x) => x.id === r.id);
  }
  const requeued = (r: Run) => r.jobs.some((j) => j.requeue_count > 0);

  const open = (r: Run) => (location.hash = `/run/${r.id}`);

  function onKey(e: KeyboardEvent) {
    const typing = document.activeElement?.tagName === 'INPUT';
    if (e.key === '/' && !typing) {
      e.preventDefault();
      searchEl?.focus();
      return;
    }
    if (typing) return;
    if (e.key === 'ArrowDown' || e.key === 'j') {
      e.preventDefault();
      sel = Math.min(visible.length - 1, sel + 1);
    } else if (e.key === 'ArrowUp' || e.key === 'k') {
      e.preventDefault();
      sel = Math.max(0, sel - 1);
    } else if (e.key === 'Enter' && visible[sel]) {
      open(visible[sel]);
      return;
    } else {
      return;
    }
    listEl?.querySelectorAll('.lrow')[sel]?.scrollIntoView({ block: 'nearest' });
  }
</script>

<svelte:window onkeydown={onKey} />

<AppShell active="runs" {overview}>
  <div class="page-body">
    <div class="scopebar">
      <div class="chipset" role="group" aria-label="Filter by status">
        {#each ['all', 'passed', 'failed', 'running'] as f (f)}
          <button
            class="schip"
            class:on={status === f}
            onclick={() => {
              status = f as typeof status;
              sel = 0;
              syncUrl();
            }}
          >
            {#if f !== 'all'}<span class="g {f}">{GLYPH[f] ?? ''}</span>{/if}
            {f}<span class="n">{counts[f as keyof typeof counts]}</span>
          </button>
        {/each}
      </div>

      <FacetDropdown label="repo" bind:value={repo} options={repoOptions} />
      <FacetDropdown label="branch" bind:value={branch} options={branchOptions} />
      <FacetDropdown label="trigger" bind:value={trigger} options={triggerOptions} />
      <FacetDropdown label="when" bind:value={when} options={WHEN} />

      <div class="rsearch">
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6">
          <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5 14 14" />
        </svg>
        <input
          bind:this={searchEl}
          bind:value={query}
          oninput={() => {
            sel = 0;
            syncUrl();
          }}
          type="search"
          placeholder="commit, sha, author, #id…"
          aria-label="Search runs"
        />
      </div>
    </div>

    <!-- describes the current query, not a trend: it moves when you filter -->
    <div class="summary">
      <span><b>{filtered.length}</b> runs</span>
      <span><b>{filtered.filter((r) => r.status === 'passed').length}</b> passed</span>
      <span><b>{filtered.filter((r) => r.status === 'failed').length}</b> failed</span>
      <span>median <b>{medianDur ? fmtDur(medianDur) : '—'}</b></span>
      <span>across <b>{new Set(filtered.map((r) => r.repo)).size}</b> repos</span>
    </div>

    {#if error}<div class="err-banner">{error}</div>{/if}

    <section class="card">
      <div class="scroll" bind:this={listEl}>
        {#each groups as g (g.label)}
          {@const bad = g.runs.filter((r) => r.status === 'failed').length}
          <div class="dayhead">
            {g.label}
            <span class="dm">
              {g.runs.length} runs{#if bad}
                · <b>{bad} failed</b>
              {/if}
            </span>
          </div>
          {#each g.runs as r (r.id)}
            {@const i = visible.indexOf(r)}
            {@const fail = failure(r)}
            {@const att = attempt(r)}
            <div
              class="lrow"
              class:selrow={i === sel}
              role="button"
              tabindex="0"
              onclick={() => open(r)}
              onkeydown={(e) => e.key === 'Enter' && open(r)}
            >
              <span class="g {r.status}">{GLYPH[r.status] ?? ''}</span>
              <span class="lb">
                <span class="ltitle">{r.commit?.message ?? `Run #${r.id}`}</span>
                <span class="lsub">
                  <span class="repo">{r.repo}</span>
                  <span class="rid">#{r.id}</span>
                  <span class="d">·</span>{r.pipeline}
                  {#if r.branch}<span class="d">·</span><span class="branch">⎇ {r.branch}</span>{/if}
                  {#if r.commit}<span class="d">·</span><span class="sha">{r.commit.sha}</span>{/if}
                  <span class="d">·</span>{r.trigger === 'webhook'
                    ? `pushed by ${r.commit?.author ?? 'unknown'}`
                    : r.trigger === 'schedule'
                      ? 'scheduled'
                      : `run by ${r.commit?.author ?? 'unknown'}`}
                  {#if att > 1}<span class="d">·</span>↻ attempt {att} on this commit{/if}
                  {#if requeued(r)}<span class="d">·</span>↻ requeued after a worker died{/if}
                </span>
                {#if fail}
                  <span class="cause">
                    <span class="jb">{fail.job.name}</span>
                    <span class="er">exit {fail.job.exit_code ?? '?'} · {fail.line}</span>
                    <a class="go" href="#/run/{r.id}?job={fail.job.id}" onclick={(e) => e.stopPropagation()}>open log →</a>
                  </span>
                {/if}
              </span>
              <span class="lside">
                <span class="stripcol"><Strip jobs={r.jobs} /></span>
                <span class="ltimes">
                  <span class="du">{fmtDur(runDur(r))}</span>
                  <span class="agev">{r.status === 'running' ? 'running' : ago(r.started_at, $now)}</span>
                </span>
              </span>
            </div>
          {/each}
        {:else}
          <div class="emptyq">
            <b>{runs.length ? 'No runs match this filter' : 'No runs yet'}</b>
            {runs.length
              ? 'Try clearing the search, or pick a different status.'
              : 'Push to a registered repo and its pipeline appears here.'}
          </div>
        {/each}
      </div>

      <div class="lfoot">
        <span>
          Showing {visible.length} of {filtered.length} matching · {runs.length}{runs.length >= 200 ? '+' : ''} loaded
        </span>
        {#if visible.length < filtered.length}
          <button class="more" onclick={() => (shown += 30)}>Load 30 more →</button>
        {/if}
        <span class="keys">
          <kbd>↑</kbd><kbd>↓</kbd> navigate <kbd>⏎</kbd> open <kbd>/</kbd> search
        </span>
      </div>
    </section>
  </div>
</AppShell>
