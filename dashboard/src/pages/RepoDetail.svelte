<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, MODE } from '../lib/api';
  import Avatar from '../lib/components/Avatar.svelte';
  import Snackbar from '../lib/components/Snackbar.svelte';
  import StatusPill from '../lib/components/StatusPill.svelte';
  import StatusText from '../lib/components/StatusText.svelte';
  import Strip from '../lib/components/Strip.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { now, startPolling } from '../lib/poll';
  import { navigate } from '../lib/router';
  import type { Overview, Repo, Run } from '../lib/types';

  let { name }: { name: string } = $props();

  const LANG_COLORS: Record<string, string> = {
    Rust: 'oklch(0.75 0.06 55)',
    Python: 'oklch(0.52 0.09 250)',
    JavaScript: 'oklch(0.88 0.14 100)',
    CSS: 'oklch(0.45 0.15 300)',
    Shell: 'oklch(0.8 0.16 130)',
    Dockerfile: 'oklch(0.42 0.04 240)',
    Other: 'oklch(0.88 0.005 110)',
  };

  let overview = $state<Overview | null>(null);
  let repos = $state<Repo[]>([]);
  let error = $state('');
  let selectedPipeline = $state<string | null>(null);
  let query = $state('');
  let ddOpen = $state(false);

  // pipeline YAML viewer
  let showYaml = $state(false);
  let yaml = $state<{ file: string; content: string } | null>(null);
  let yamlError = $state('');

  async function toggleYaml() {
    showYaml = !showYaml;
    if (!showYaml) return;
    yaml = null;
    yamlError = '';
    try {
      // only pin the file when it's a real path (history-derived entries
      // can carry placeholders like "(default)")
      const file = pipeline?.file.match(/\.ya?ml$/) ? pipeline.file : undefined;
      yaml = await api.pipelineFile(name, file);
    } catch (e) {
      yamlError = (e as Error).message;
    }
  }

  // deleting the repo (runs stay in history)
  let deleting = $state(false);

  async function deleteRepo() {
    if (!window.confirm(`Unregister ${name} from the dashboard?\nIts run history stays; the repo is no longer polled or triggerable.`)) return;
    deleting = true;
    try {
      await api.deleteRepo(name);
      navigate('/repos');
    } catch (e) {
      error = (e as Error).message;
      deleting = false;
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

  const repo = $derived(repos.find((r) => r.name === name) ?? null);
  const repoRuns = $derived(
    (overview?.runs ?? []).filter((r) => (r.repo ?? r.pipeline) === name),
  );
  const last = $derived(repoRuns[0] ?? null);

  // pipelines the coordinator found on the Forgejo remote — or, before the
  // pipeline file is pushed there, derived from this repo's run history so
  // past runs are still browsable
  const pipelines = $derived.by(() => {
    if (!repo) return [];
    if (repo.pipelines.length) return repo.pipelines;
    const seen = new Map<string, string>();
    for (const r of repoRuns) if (!seen.has(r.pipeline)) seen.set(r.pipeline, r.pipeline_file);
    return [...seen].map(([name, file]) => ({ name, file }));
  });

  const pipeline = $derived.by(() => {
    if (!repo) return null;
    if (selectedPipeline && pipelines.some((p) => p.name === selectedPipeline)) {
      return pipelines.find((p) => p.name === selectedPipeline)!;
    }
    // default to the pipeline with the most recent run
    const byRecent = repoRuns[0] && pipelines.find((p) => p.name === repoRuns[0].pipeline);
    return byRecent ?? pipelines[0] ?? null;
  });

  const pipeRuns = $derived(pipeline ? repoRuns.filter((r) => r.pipeline === pipeline.name) : []);

  function runDuration(r: Run): number {
    return (r.finished_at ?? Date.now()) - r.started_at;
  }

  function matchesQuery(r: Run, q: string): boolean {
    return [r.commit?.message, r.commit?.sha, r.commit?.author, `#${r.id}`, String(r.id), r.trigger, r.status]
      .some((v) => v && String(v).toLowerCase().includes(q));
  }
  const q = $derived(query.trim().toLowerCase());
  const shownRuns = $derived(q ? pipeRuns.filter((r) => matchesQuery(r, q)).slice(0, 30) : pipeRuns.slice(0, 8));

  // hero insights
  const finished = $derived(repoRuns.filter((r) => r.finished_at));
  const heroRate = $derived(
    finished.length
      ? `${Math.round((finished.filter((r) => r.status === 'passed').length / finished.length) * 100)}%`
      : '—',
  );
  const heroDurs = $derived(finished.map((r) => runDuration(r)));
  const heroAvg = $derived(heroDurs.length ? fmtDur(heroDurs.reduce((a, b) => a + b, 0) / heroDurs.length) : '—');
  const heroWorst = $derived(Math.max(...heroDurs, 1));
  const heroBars = $derived([...finished].slice(0, 10).reverse());

  function pipeLast(p: string): Run | undefined {
    return repoRuns.find((r) => r.pipeline === p);
  }
  function pipeCount(p: string): number {
    return repoRuns.filter((r) => r.pipeline === p).length;
  }

  function closeDd(e: MouseEvent) {
    if (!(e.target as Element).closest('.dd')) ddOpen = false;
  }
</script>

<svelte:window onclick={closeDd} />

<Snackbar fallback="/repos">
  <span class="stitle">{repo?.name ?? name}</span>
  {#if last}<StatusPill status={last.status} />{/if}
</Snackbar>

<main class="wide-wrap">
  {#if error}<div class="err-banner">{error}</div>{/if}

  {#if repo}
    <section class="latest-hero" aria-label="Latest run">
      {#if last}
        <div class="lh-left">
          <span class="glabel">Latest run · {ago(last.started_at, $now)}</span>
          <h2 class="lh-title" title={last.commit?.message ?? ''}>{last.commit?.message ?? `Run #${last.id}`}</h2>
          <div class="lh-meta">
            {#if last.commit}
              <Avatar name={last.commit.author} /> {last.commit.author}
              <span class="sha-chip">{last.commit.sha}</span>
            {/if}
            <span>{last.pipeline} #{last.id}</span>
            <span>via {last.trigger}</span>
            <span><StatusPill status={last.status} /></span>
          </div>
          <div class="lh-strip">
            <Strip jobs={last.jobs} />
            <span class="ljobs">
              {last.jobs.filter((j) => j.status === 'passed').length}/{last.jobs.length} jobs passed
              · {fmtDur(runDuration(last))}
            </span>
          </div>
          <a class="btn btn-lime" href="#/run/{last.id}">view run →</a>
        </div>
        <div class="lh-insights">
          <div class="lh-stats">
            <span class="lh-stat"><span class="k">runs</span><span class="v">{repoRuns.length}</span></span>
            <span class="lh-stat"><span class="k">success</span><span class="v">{heroRate}</span></span>
            <span class="lh-stat"><span class="k">avg</span><span class="v">{heroAvg}</span></span>
          </div>
          <span class="sbars" role="img" aria-label="Durations of the last {heroBars.length} runs">
            {#each heroBars as r (r.id)}
              <i
                class:failed={r.status === 'failed'}
                style="height:{Math.max(10, Math.round((runDuration(r) / heroWorst) * 100))}%"
                title="#{r.id} · {fmtDur(runDuration(r))}"
              ></i>
            {/each}
          </span>
          <span class="lh-cap">last {heroBars.length} runs · duration</span>
        </div>
      {:else}
        <div>
          <span class="glabel">Latest run</span>
          <h2 class="lh-title">No runs yet</h2>
          <div class="lh-meta">trigger one from the overview, or push to {repo.branch}</div>
        </div>
      {/if}
    </section>

    <div class="repo-cols">
      <div>
        <section class="card panel-projects" style="grid-column:auto">
          <div class="pipes-head">
            <span>
              <details class="dd" open={ddOpen} ontoggle={(e) => (ddOpen = (e.target as HTMLDetailsElement).open)}>
                <summary aria-label="Switch pipeline">
                  <span class="br" aria-hidden="true">⎇</span>{pipeline?.name ?? '—'}<span class="caret" aria-hidden="true">▾</span>
                </summary>
                <div class="dd-menu">
                  <div class="dd-head">{pipelines.length} pipeline{pipelines.length === 1 ? '' : 's'} in {repo.name}</div>
                  {#each pipelines as p (p.name)}
                    {@const lastP = pipeLast(p.name)}
                    <button
                      type="button"
                      class="dd-item"
                      onclick={() => {
                        selectedPipeline = p.name;
                        ddOpen = false;
                      }}
                    >
                      <span class="check" aria-hidden="true">{p.name === pipeline?.name ? '✓' : ''}</span>
                      <span class="g {lastP?.status ?? 'pending'}" aria-hidden="true">{GLYPH[lastP?.status ?? 'pending']}</span>
                      <span class="dname">{p.name}</span>
                      <span class="dfile">{p.file.split('/').pop()} · {pipeCount(p.name)} runs</span>
                    </button>
                  {/each}
                </div>
              </details>
            </span>
            <span class="proj-meta">{pipelines.length} pipeline{pipelines.length === 1 ? '' : 's'}</span>
            <input
              type="search"
              class="run-search"
              placeholder="search runs…"
              autocomplete="off"
              spellcheck="false"
              aria-label="Search runs in this pipeline"
              bind:value={query}
            />
            <button class="btn" onclick={toggleYaml} disabled={!pipeline && !repo.remote}>
              {showYaml ? 'hide file' : 'view file'}
            </button>
          </div>

          {#if showYaml}
            <div class="yaml-view">
              {#if yamlError}
                <div class="empty">{yamlError}</div>
              {:else if !yaml}
                <div class="empty">fetching pipeline file…</div>
              {:else}
                <div class="yaml-head"><span class="mono">{yaml.file}</span><span class="dim">@ {repo.branch}</span></div>
                <pre class="yaml-pre">{yaml.content}</pre>
              {/if}
            </div>
          {/if}

          <div>
            {#if !shownRuns.length}
              <div class="empty" style="margin:16px 0">
                {q ? `No runs match “${query.trim()}”.` : 'No runs for this pipeline yet.'}
              </div>
            {/if}
            {#each shownRuns as r (r.id)}
              <a class="wrun" href="#/run/{r.id}">
                <span class="g {r.status}" aria-hidden="true">{GLYPH[r.status] ?? '·'}</span>
                <span class="wrun-body">
                  <span class="wrun-title">{r.commit?.message ?? '—'}</span>
                  <span class="wrun-sub">
                    {r.pipeline} #{r.id}: commit <span class="mono">{r.commit?.sha ?? '?'}</span>
                    by {r.commit?.author ?? '?'} · via {r.trigger}
                  </span>
                </span>
                <span class="branch-chip">{repo.branch}</span>
                <span class="wrun-meta">
                  <span>{ago(r.started_at, $now)}</span>
                  <span class="dur">{fmtDur(runDuration(r))}</span>
                </span>
              </a>
            {/each}
          </div>
        </section>
      </div>

      <aside class="repo-side-col" aria-label="Repository info">
        <section class="card side-sec">
          <span class="glabel">About</span>
          <p class="side-desc">{repo.description}</p>
          <div class="about-row">
            <span class="sik">hosted at</span>
            <span class="siv">
              {#if repo.remote}
                <a href={repo.remote} target="_blank" rel="noopener">{repo.remote.replace(/^https?:\/\//, '')} ↗</a>
              {:else}<span class="dim">not configured</span>{/if}
            </span>
          </div>
          <div class="about-row"><span class="sik">default branch</span><span class="siv">{repo.branch}</span></div>
          <div class="about-row"><span class="sik">owner</span><span class="siv">@{repo.owner}</span></div>
          <div class="repo-danger">
            <button class="btn btn-danger" onclick={deleteRepo} disabled={deleting}>
              {deleting ? 'removing…' : 'Delete repo'}
            </button>
            <span class="dim">runs stay in history</span>
          </div>
        </section>

        <section class="card side-sec">
          <span class="glabel">Contributors <span class="count-chip">{repo.contributors.length}</span></span>
          {#each repo.contributors as c (c.login)}
            <div class="contrib-row">
              <Avatar name={c.login} />
              <span class="clogin">{c.login}</span>
              <span class="cname">{c.name}</span>
            </div>
          {:else}
            <span class="dim" style="font-size:12.5px">unknown</span>
          {/each}
        </section>

        <section class="card side-sec">
          <span class="glabel">Languages</span>
          {#if repo.languages.length}
            <div class="lang-bar" role="img" aria-label="Language breakdown">
              {#each repo.languages as l (l.name)}
                <i style="width:{l.pct}%;background:{LANG_COLORS[l.name] ?? LANG_COLORS.Other}" title="{l.name} {l.pct}%"></i>
              {/each}
            </div>
            <div class="lang-legend">
              {#each repo.languages as l (l.name)}
                <span class="lang-item">
                  <span class="lang-dot2" style="background:{LANG_COLORS[l.name] ?? LANG_COLORS.Other}" aria-hidden="true"></span>
                  <span class="lname">{l.name}</span><span class="lpct">{l.pct}%</span>
                </span>
              {/each}
            </div>
          {:else}
            <span class="dim" style="font-size:12.5px">no data from the remote yet</span>
          {/if}
        </section>
      </aside>
    </div>
  {:else if repos.length}
    <div class="empty" style="margin:24px 0">Repo "{name}" not found.</div>
  {/if}

  <footer class="foot">mode: {MODE} · polling every 3s · click a run for its timeline</footer>
</main>
