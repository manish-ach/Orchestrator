<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, MODE } from '../lib/api';
  import { drawSparkBars } from '../lib/charts';
  import StatusPill from '../lib/components/StatusPill.svelte';
  import Topbar from '../lib/components/Topbar.svelte';
  import { ago, GLYPH } from '../lib/format';
  import { now, startPolling } from '../lib/poll';
  import type { Overview, Repo, Run } from '../lib/types';

  const LANG_CLASS: Record<string, string> = { Rust: 'lang-rust', Python: 'lang-python' };

  let overview = $state<Overview | null>(null);
  let repos = $state<Repo[]>([]);
  let error = $state('');
  let query = $state('');

  let adding = $state(false);
  let remoteUrl = $state('');
  let addError = $state('');
  let addBusy = $state(false);

  async function addRepo(e: SubmitEvent) {
    e.preventDefault();
    const url = remoteUrl.trim();
    if (!url || addBusy) return;
    addBusy = true;
    addError = '';
    try {
      await api.addRepo(url);
      repos = await api.repos();
      adding = false;
      remoteUrl = '';
    } catch (err) {
      addError = (err as Error).message;
    } finally {
      addBusy = false;
    }
  }

  const stop = startPolling(async () => {
    try {
      const [r, o] = await Promise.all([api.repos(), api.overview()]);
      repos = r;
      overview = o;
      error = '';
    } catch (e) {
      error = `Cannot reach the data source (${(e as Error).message}). Retrying on the next poll.`;
    }
  });
  onDestroy(stop);

  const runs = $derived(overview?.runs ?? []);

  function repoRuns(repo: Repo): Run[] {
    return runs.filter((r) => (r.repo ?? r.pipeline) === repo.name);
  }

  function matches(repo: Repo, q: string): boolean {
    return [repo.name, repo.description, repo.language, repo.owner, ...repo.pipelines.map((p) => p.name)]
      .some((v) => v && String(v).toLowerCase().includes(q));
  }

  const q = $derived(query.trim().toLowerCase());
  const shown = $derived(q ? repos.filter((r) => matches(r, q)) : repos);

  function runDuration(r: Run): number {
    return (r.finished_at ?? Date.now()) - r.started_at;
  }

  function rate(repo: Repo): { pct: number | null } {
    const finished = repoRuns(repo).filter((r) => r.finished_at);
    if (!finished.length) return { pct: null };
    return { pct: Math.round((finished.filter((r) => r.status === 'passed').length / finished.length) * 100) };
  }

  function spark(canvas: HTMLCanvasElement, rr: Run[]) {
    const draw = (list: Run[]) => {
      const recent = list.slice(0, 6).reverse();
      drawSparkBars(
        canvas,
        recent.map((r) => ({
          count: Math.max(1, Math.round(runDuration(r) / 1000)),
          failed: r.status === 'failed',
        })),
      );
    };
    draw(rr);
    return {
      update(next: Run[]) {
        draw(next);
      },
    };
  }
</script>

<Topbar active="repos" {overview} />

<main class="wrap">
  {#if error}<div class="err-banner">{error}</div>{/if}

  <div class="repos-head">
    <h1 class="page-title2">Repositories</h1>
    <span class="meta2">
      {q ? `${shown.length} of ${repos.length} repos` : `${repos.length} repos · ${runs.length} runs`}
    </span>
    <input
      type="search"
      class="run-search"
      placeholder="search repositories…"
      autocomplete="off"
      spellcheck="false"
      aria-label="Search repositories"
      bind:value={query}
    />
    <button
      class="btn btn-primary"
      onclick={() => { adding = !adding; addError = ''; }}
    >{adding ? 'Close' : '+ Add repo'}</button>
  </div>

  {#if adding}
    <form class="repo-add" onsubmit={addRepo}>
      <input
        type="url"
        class="run-search repo-add-input"
        placeholder="https://git.manishacharya.name.np/owner/repo"
        autocomplete="off"
        spellcheck="false"
        aria-label="Repository URL"
        bind:value={remoteUrl}
      />
      <button class="btn btn-primary" disabled={addBusy || !remoteUrl.trim()}>
        {addBusy ? 'Fetching…' : 'Add'}
      </button>
    </form>
    {#if addError}<div class="err-banner">{addError}</div>{/if}
  {/if}

  <div class="repo-list">
    {#if !shown.length}
      <div class="empty" style="margin:16px 0">No repositories match “{query.trim()}”.</div>
    {/if}
    {#each shown as repo (repo.name)}
      {@const rr = repoRuns(repo)}
      {@const last = rr[0]}
      {@const rt = rate(repo)}
      <a class="repo-row" href="#/repo/{encodeURIComponent(repo.name)}">
        <span class="repo-main">
          <span class="repo-title">
            <span class="repo-name">{repo.name}</span>
            <span class="repo-chip">{repo.pipelines.length} pipeline{repo.pipelines.length > 1 ? 's' : ''}</span>
          </span>
          <span class="repo-desc">{repo.description}</span>
          <span class="repo-meta">
            <span><span class="lang-dot {LANG_CLASS[repo.language] ?? 'lang-none'}" aria-hidden="true"></span>{repo.language}</span>
            <span class="sep" aria-hidden="true">·</span>
            <span>@{repo.owner}</span>
            <span class="sep" aria-hidden="true">·</span>
            <span>
              {#if rt.pct !== null}
                <span class="rate" class:low={rt.pct < 80}>{rt.pct}% pass</span>
              {:else}no runs yet{/if}
            </span>
          </span>
          {#if last}
            <span class="repo-latest">
              <span class="g {last.status}" aria-hidden="true">{GLYPH[last.status]}</span>
              <span class="lpipe">{last.pipeline} #{last.id}</span>
              <span class="lmsg">{last.commit?.message ?? '—'}</span>
              <span class="lwhen">{ago(last.started_at, $now)}</span>
            </span>
          {:else}
            <span class="repo-latest"><span class="g pending">○</span><span class="lmsg">no runs yet</span></span>
          {/if}
        </span>
        <span class="repo-side">
          {#if last}<StatusPill status={last.status} />{:else}
            <span class="st st-pending"><span class="g">○</span>idle</span>
          {/if}
          <span class="spark-wrap">
            <canvas class="spark" use:spark={rr} aria-label="Durations of the latest runs for {repo.name}"></canvas>
            <span class="spark-cap">last {Math.min(rr.length, 6)} runs · duration</span>
          </span>
        </span>
      </a>
    {/each}
  </div>

  <footer class="foot">mode: {MODE} · polling every 3s · bars: latest runs, height = duration</footer>
</main>
