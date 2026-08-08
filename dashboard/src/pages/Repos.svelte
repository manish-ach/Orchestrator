<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api } from '../lib/api';
  import AppShell from '../lib/components/AppShell.svelte';
  import FacetDropdown from '../lib/components/FacetDropdown.svelte';
  import Strip from '../lib/components/Strip.svelte';
  import { ago, fmtDur, GLYPH } from '../lib/format';
  import { now, startPolling } from '../lib/poll';
  import { route } from '../lib/router';
  import type { Overview, Repo, Run } from '../lib/types';

  // Three levels behind one nav item:
  //   #/repos                     every repository
  //   #/repos/<repo>              its pipelines
  //   #/repos/<repo>/<pipeline>   that pipeline's own runs
  // Pipelines are not a top-level page because a pipeline only means anything
  // inside the repo that defines it.
  let repos = $state<Repo[]>([]);
  let overview = $state<Overview | null>(null);
  let error = $state('');
  let query = $state('');
  let language = $state('all');
  let sort = $state('last run');
  let adding = $state(false);
  let removeTarget = $state<Repo | null>(null);
  let url = $state('');
  let detecting = $state(false);
  let detected = $state<{ owner: string; name: string } | null>(null);
  let addError = $state('');

  // showModal() rather than the `open` attribute: only the modal form gets a
  // ::backdrop, a focus trap and Esc-to-close from the platform.
  let addDlg = $state<HTMLDialogElement | null>(null);
  let rmDlg = $state<HTMLDialogElement | null>(null);
  $effect(() => {
    if (!addDlg) return;
    if (adding && !addDlg.open) addDlg.showModal();
    else if (!adding && addDlg.open) addDlg.close();
  });
  $effect(() => {
    if (!rmDlg) return;
    if (removeTarget && !rmDlg.open) rmDlg.showModal();
    else if (!removeTarget && rmDlg.open) rmDlg.close();
  });

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

  const runs = $derived([...(overview?.runs ?? [])].sort((a, b) => b.started_at - a.started_at));
  const runsOf = (repo: string) => runs.filter((r) => r.repo === repo);
  const runsOfPipeline = (pipeline: string) => runs.filter((r) => r.pipeline === pipeline);
  const runDur = (r: Run) => (r.finished_at ?? $now) - r.started_at;

  // ---- route ------------------------------------------------------------
  const repoName = $derived($route.path[1] ? decodeURIComponent($route.path[1]) : null);
  const pipeName = $derived($route.path[2] ? decodeURIComponent($route.path[2]) : null);
  const repo = $derived(repos.find((r) => r.name === repoName) ?? null);
  const go = (path = '') => (location.hash = `/repos${path ? `/${path}` : ''}`);

  // Escape climbs a level, matching the back button.
  function onKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || document.activeElement?.tagName === 'INPUT') return;
    // a modal is dismissing itself on this same Escape — do not also navigate
    if (adding || removeTarget) return;
    if (pipeName) go(encodeURIComponent(repoName!));
    else if (repoName) go();
  }

  // ---- level 1 ----------------------------------------------------------
  const LANGS = $derived(['all', ...new Set(repos.flatMap((r) => r.languages.map((l) => l.name)))]);
  const SORTS = ['last run', 'name', 'pass rate', 'runs'];

  function stats(name: string) {
    const rs = runsOf(name);
    const done = rs.filter((r) => r.status !== 'running');
    const ok = done.filter((r) => r.status === 'passed').length;
    return { total: rs.length, pass: done.length ? Math.round((ok / done.length) * 100) : null, latest: rs[0] ?? null };
  }

  const listed = $derived.by(() => {
    const q = query.trim().toLowerCase();
    let out = repos.filter(
      (r) =>
        (language === 'all' || r.languages.some((l) => l.name === language)) &&
        (!q || [r.name, r.description, r.owner, r.language].join(' ').toLowerCase().includes(q)),
    );
    if (sort === 'name') out = [...out].sort((a, b) => a.name.localeCompare(b.name));
    else if (sort === 'runs') out = [...out].sort((a, b) => stats(b.name).total - stats(a.name).total);
    else if (sort === 'pass rate') out = [...out].sort((a, b) => (stats(b.name).pass ?? -1) - (stats(a.name).pass ?? -1));
    else out = [...out].sort((a, b) => (stats(b.name).latest?.started_at ?? 0) - (stats(a.name).latest?.started_at ?? 0));
    return out;
  });

  const LANG_COLOUR: Record<string, string> = {
    Rust: '#dea584', Python: '#3572A5', TypeScript: '#3178c6', JavaScript: '#f1e05a',
    CSS: '#563d7c', HTML: '#e34c26', Shell: '#89e051', Dockerfile: '#384d54', Other: '#9aa08e',
  };
  const colour = (n: string) => LANG_COLOUR[n] ?? '#9aa08e';

  // ---- level 2: a pipeline's shape comes from its most recent run --------
  // A pipeline that has never run has no jobs, so it has no stages to draw.
  // Saying so is better than inventing a shape.
  function pipelineView(name: string) {
    const rs = runsOfPipeline(name);
    const latest = rs[0] ?? null;
    const done = rs.filter((r) => r.status !== 'running');
    const ok = done.filter((r) => r.status === 'passed').length;
    const durs = done.map(runDur).sort((a, b) => a - b);
    const stages = latest ? [...new Set(latest.jobs.map((j) => j.stage))] : [];
    return {
      latest,
      runs: rs,
      total: rs.length,
      passed: ok,
      pass: done.length ? Math.round((ok / done.length) * 100) : null,
      median: durs.length ? durs[Math.floor(durs.length / 2)] : null,
      p90: durs.length ? durs[Math.min(durs.length - 1, Math.floor(durs.length * 0.9))] : null,
      stages: stages.map((s) => ({ name: s, jobs: latest!.jobs.filter((j) => j.stage === s) })),
      health: rs.slice(0, 20).reverse(),
    };
  }

  // ---- level 3 ----------------------------------------------------------
  const pipe = $derived(pipeName ? pipelineView(pipeName) : null);
  const worst = $derived(pipe ? Math.max(1, ...pipe.runs.map(runDur)) : 1);

  // ---- add / remove -----------------------------------------------------
  const URL_RE = /^https?:\/\/[^/]+\/([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/;
  let debounce: ReturnType<typeof setTimeout> | undefined;
  function inspect() {
    clearTimeout(debounce);
    detected = null;
    addError = '';
    const m = url.trim().match(URL_RE);
    if (!m) return;
    detecting = true;
    debounce = setTimeout(() => {
      detecting = false;
      detected = { owner: m[1], name: m[2] };
    }, 400);
  }
  async function confirmAdd() {
    if (!detected) return;
    try {
      await api.addRepo(url.trim());
      adding = false;
      url = '';
      detected = null;
      repos = await api.repos();
    } catch (e) {
      addError = (e as Error).message;
    }
  }
  async function confirmRemove() {
    if (!removeTarget) return;
    try {
      await api.deleteRepo(removeTarget.name);
      removeTarget = null;
      repos = await api.repos();
    } catch (e) {
      error = `Could not remove: ${(e as Error).message}`;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<AppShell active="repos" {overview}>
  <div class="page-body">
    {#if error}<div class="err-banner">{error}</div>{/if}

    {#if !repoName}
      <!-- ===== level 1: every repository ===== -->
      <div class="scopebar">
        <FacetDropdown label="language" bind:value={language} options={LANGS} />
        <FacetDropdown label="sort" bind:value={sort} options={SORTS} />
        <div class="rsearch">
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6">
            <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5 14 14" />
          </svg>
          <input bind:value={query} type="search" placeholder="name, description, owner…" aria-label="Search repositories" />
        </div>
        <button class="btn primary" onclick={() => { adding = true; url = ''; detected = null; addError = ''; }}>
          + Add repository
        </button>
      </div>

      <div class="summary">
        <span><b>{listed.length}</b> repositories</span>
        <span><b>{listed.reduce((n, r) => n + r.pipelines.length, 0)}</b> pipelines</span>
        <span><b>{listed.reduce((n, r) => n + stats(r.name).total, 0)}</b> runs</span>
      </div>

      <div class="pscroll">
        <div class="rgrid">
          {#each listed as r (r.name)}
            {@const s = stats(r.name)}
            {@const total = r.languages.reduce((n, l) => n + l.pct, 0) || 1}
            <!-- a real link stretched over the card: whole-card click without
                 nesting a button inside a button, and keyboard-reachable -->
            <article class="rcard">
              <button class="kebab" title="Remove repository" onclick={() => (removeTarget = r)}>⋯</button>
              <div class="rhead">
                <div style="min-width:0">
                  <a class="rname stretch" href="#/repos/{encodeURIComponent(r.name)}">{r.name}</a>
                  <span class="rdesc">{r.description || 'No description'}</span>
                </div>
                <div class="rstate">
                  {#if s.latest}
                    <span class="w {s.latest.status}"><span class="g {s.latest.status}">{GLYPH[s.latest.status] ?? ''}</span>{s.latest.status}</span>
                    <span class="agev">#{s.latest.id} · {ago(s.latest.started_at, $now)}</span>
                  {:else}
                    <span class="w none">no runs yet</span>
                  {/if}
                </div>
              </div>

              {#if r.languages.length}
                <div>
                  <div class="langbar">
                    {#each r.languages as l (l.name)}
                      <i style="width:{(l.pct / total) * 100}%;background:{colour(l.name)}" title="{l.name} {l.pct}%"></i>
                    {/each}
                  </div>
                  <div class="langleg">
                    {#each r.languages.slice(0, 4) as l (l.name)}
                      <span><i style="background:{colour(l.name)}"></i>{l.name} <b>{l.pct}%</b></span>
                    {/each}
                  </div>
                </div>
              {/if}

              <div class="rmeta">
                ⎇ {r.branch}<span class="d">·</span>{r.owner}<span class="d">·</span>
                <span class="k">{r.pipelines.length} pipeline{r.pipelines.length === 1 ? '' : 's'}</span>
                <span class="d">·</span>{s.total} runs
              </div>

              <!-- The point of this line is to tell "quiet" apart from
                   "broken". A repo nobody has pushed to and one whose webhook
                   URL is wrong are indistinguishable without it. -->
              <div class="hook {r.webhook ? r.webhook.status : 'never'}">
                {#if !r.webhook}
                  <span class="hdot"></span>
                  <span>No push has ever reached the coordinator — check the repository's webhook.</span>
                {:else if r.webhook.status === 'accepted'}
                  <span class="hdot"></span>
                  <span>Webhook delivering · last push {ago(r.webhook.last_at, $now)}</span>
                {:else}
                  <span class="hdot"></span>
                  <span>
                    Webhook reaching us but <b>rejected</b> {ago(r.webhook.last_at, $now)}
                    {#if r.webhook.detail}<span class="hwhy">{r.webhook.detail}</span>{/if}
                  </span>
                {/if}
              </div>

              <div class="rfoot">
                <span class="avs">
                  {#each r.contributors.slice(0, 4) as c, i (c.login)}
                    <i style="background:{['oklch(0.55 0.13 145)','oklch(0.52 0.15 250)','oklch(0.55 0.15 25)','oklch(0.52 0.12 300)'][i % 4]}" title={c.name}>
                      {c.login[0].toUpperCase()}
                    </i>
                  {/each}
                </span>
                <span class="cn">{r.contributors.length} contributors</span>
                <span class="pass">
                  {s.pass === null ? 'waiting for the first push' : `${s.pass}% pass rate · ${s.total} runs`}
                </span>
              </div>
            </article>
          {:else}
            <div class="emptyq">
              <b>{repos.length ? 'No repositories match' : 'No repositories registered'}</b>
              {repos.length ? 'Clear the search, or pick another language.' : 'Add one with the button above.'}
            </div>
          {/each}
        </div>
      </div>

    {:else if !pipeName}
      <!-- ===== level 2: one repository, its pipelines ===== -->
      <div class="crumb">
        <button class="backb" title="Back to all repositories" aria-label="Back to all repositories" onclick={() => go()}>←</button>
        <button class="crumb-link" onclick={() => go()}>Repositories</button>
        <span class="sep">/</span><span class="cur">{repoName}</span>
      </div>

      {#if repo}
        {@const s = stats(repo.name)}
        {@const total = repo.languages.reduce((n, l) => n + l.pct, 0) || 1}
        <div class="dhead">
          <div style="min-width:0">
            <span class="nm">{repo.name}</span>
            <span class="sb">{repo.description || 'No description'}</span>
            {#if repo.languages.length}
              <div style="margin-top:9px;max-width:340px">
                <div class="langbar">
                  {#each repo.languages as l (l.name)}
                    <i style="width:{(l.pct / total) * 100}%;background:{colour(l.name)}"></i>
                  {/each}
                </div>
              </div>
            {/if}
          </div>
          <div class="rt">
            <span class="tag-c">⎇ {repo.branch}</span>
            <span class="tag-c">{repo.owner}</span>
            {#if repo.remote}<a class="tag-c" href={repo.remote} target="_blank" rel="noreferrer">open ↗</a>{/if}
            {#if s.latest}
              <span class="w {s.latest.status}"><span class="g {s.latest.status}">{GLYPH[s.latest.status] ?? ''}</span>{s.latest.status}</span>
            {/if}
          </div>
        </div>

        <div class="pscroll">
          <div class="plist">
            <div class="lbl" style="padding:2px 2px 0">
              {repo.pipelines.length} pipeline{repo.pipelines.length === 1 ? '' : 's'} in this repository
            </div>
            {#each repo.pipelines as p (p.name)}
              {@const v = pipelineView(p.name)}
              <article class="pcard">
                <div class="phead">
                  <div style="min-width:0">
                    <a class="pname stretch" href="#/repos/{encodeURIComponent(repo.name)}/{encodeURIComponent(p.name)}">{p.name}</a>
                    <span class="psub">{p.file}</span>
                  </div>
                  <div class="rstate">
                    {#if v.latest}
                      <span class="w {v.latest.status}"><span class="g {v.latest.status}">{GLYPH[v.latest.status] ?? ''}</span>{v.latest.status}</span>
                      <span class="agev">#{v.latest.id} · {ago(v.latest.started_at, $now)}</span>
                    {:else}
                      <span class="w none">never run</span>
                    {/if}
                  </div>
                </div>

                {#if v.stages.length}
                  <!-- shape from the most recent run: real outcomes per job -->
                  <div class="chain">
                    {#each v.stages as st, i (st.name)}
                      {#if i}<span class="lnk">›</span>{/if}
                      <div class="stg">
                        <span class="sn">{st.name}<i>{st.jobs.length}</i></span>
                        <span class="dots">
                          {#each st.jobs as j (j.id)}<i class={j.status}></i>{/each}
                        </span>
                      </div>
                    {/each}
                  </div>
                  <div class="pmeta">
                    {v.stages.reduce((n, s2) => n + s2.jobs.length, 0)} jobs in {v.stages.length} stages
                    {#if v.median}<span class="d">·</span>median {fmtDur(v.median)}{/if}
                    {#if p.schedule}<span class="d">·</span><span class="cron">⏱ {p.schedule}</span>{/if}
                  </div>
                {:else if p.parse_error}
                  <!-- a pipeline that cannot be planned cannot run: say which,
                       and why, rather than showing an empty shape -->
                  <div class="perr">
                    <b>{p.file} does not parse</b>
                    {p.parse_error}
                  </div>
                {:else if p.stages.length}
                  <!-- never run, but the coordinator indexed its definition, so
                       the declared shape is known — drawn hollow, because no
                       job here has an outcome yet -->
                  <div class="chain declared">
                    {#each p.stages as st, i (st)}
                      {@const stageJobs = p.jobs.filter((j) => j.stage === st)}
                      {#if i}<span class="lnk">›</span>{/if}
                      <div class="stg">
                        <span class="sn">{st}<i>{stageJobs.length}</i></span>
                        <span class="dots">
                          {#each stageJobs as j (j.name)}<i class="pending"></i>{/each}
                        </span>
                      </div>
                    {/each}
                  </div>
                  <div class="pmeta">
                    {p.jobs.length} jobs in {p.stages.length} stages, as declared
                    {#if p.schedule}<span class="d">·</span><span class="cron">⏱ {p.schedule}</span>{/if}
                  </div>
                {:else}
                  <div class="pmeta">Shape appears after its first run.</div>
                {/if}

                <div class="phealth">
                  <span class="hbar">
                    {#each v.health as r (r.id)}<i class={r.status === 'failed' ? 'failed' : ''}></i>{:else}<i class="none"></i>{/each}
                  </span>
                  <span class="pstat">
                    {v.total ? `${v.passed}/${v.total} passed` : 'never run — push, or trigger via the API'}
                  </span>
                </div>
              </article>
            {:else}
              <div class="emptyq"><b>No pipelines found</b>Add a <code>.orchestrator/*.yml</code> to this repository.</div>
            {/each}
          </div>
        </div>
      {:else}
        <div class="emptyq"><b>No such repository</b>{repoName} is not registered.</div>
      {/if}

    {:else if pipe}
      <!-- ===== level 3: one pipeline's own runs ===== -->
      <div class="crumb">
        <button class="backb" title="Back to {repoName}" aria-label="Back to {repoName}" onclick={() => go(encodeURIComponent(repoName!))}>←</button>
        <button class="crumb-link" onclick={() => go()}>Repositories</button>
        <span class="sep">/</span>
        <button class="crumb-link" onclick={() => go(encodeURIComponent(repoName!))}>{repoName}</button>
        <span class="sep">/</span><span class="cur">{pipeName}</span>
      </div>

      <div class="dhead">
        <div style="min-width:0">
          <span class="nm">{pipeName}</span>
          <span class="sb">{repoName}{pipe.latest ? ` · ${pipe.latest.pipeline_file}` : ''}</span>
        </div>
      </div>

      <div class="wells five">
        <div class="well"><span class="lbl">Runs</span><span class="v">{pipe.total}<small>&nbsp;total</small></span></div>
        <div class="well">
          <span class="lbl">Pass rate</span>
          <span class="v">{pipe.pass ?? '–'}<small>% · {pipe.total - pipe.passed} failed</small></span>
        </div>
        <div class="well"><span class="lbl">Median</span><span class="v">{pipe.median ? fmtDur(pipe.median) : '—'}</span></div>
        <div class="well"><span class="lbl">p90</span><span class="v">{pipe.p90 ? fmtDur(pipe.p90) : '—'}</span></div>
        <div class="well">
          <span class="lbl">Last run</span>
          <span class="v sm">{pipe.latest ? `#${pipe.latest.id}` : '—'}<small>{pipe.latest ? ` ${ago(pipe.latest.started_at, $now)}` : ''}</small></span>
        </div>
      </div>

      <section class="card">
        <div class="scroll">
          {#if pipe.runs.length}
            <!-- Repo and pipeline are constant here, so the columns that vary on
                 the global Runs page are dropped and the space goes to duration
                 against this pipeline's own median. -->
            <div class="dlabel">
              <span></span><span>Commit</span><span>Stages</span>
              <span>Duration vs median</span><span class="r">Duration</span><span class="r">When</span>
            </div>
            {#each pipe.runs as r (r.id)}
              {@const d = runDur(r)}
              {@const delta = pipe.median ? Math.round(((d - pipe.median) / pipe.median) * 100) : 0}
              <a class="prow" href="#/run/{r.id}">
                <span class="g {r.status}">{GLYPH[r.status] ?? ''}</span>
                <span style="min-width:0">
                  <span class="t">{r.commit?.message ?? `Run #${r.id}`}</span>
                  <span class="s">
                    {r.commit?.sha ?? '—'} · {r.trigger === 'webhook' ? `pushed by ${r.commit?.author ?? 'unknown'}` : r.trigger}
                  </span>
                </span>
                <span class="stripcol"><Strip jobs={r.jobs} /></span>
                <span class="dbar" title="median {pipe.median ? fmtDur(pipe.median) : '—'}">
                  <i class:slow={delta > 25} class:fail={r.status === 'failed'} style="width:{(d / worst) * 100}%"></i>
                  {#if pipe.median}<u style="left:{(pipe.median / worst) * 100}%"></u>{/if}
                </span>
                <span class="du">
                  {fmtDur(d)}
                  {#if pipe.median && r.finished_at}<small class:over={delta > 25}>{delta > 0 ? '+' : ''}{delta}%</small>{/if}
                </span>
                <span class="agev r">{r.status === 'running' ? 'running' : ago(r.started_at, $now)}</span>
              </a>
            {/each}
          {:else}
            <div class="emptyq">
              <b>This pipeline has never run</b>
              Push to <code>{repo?.branch ?? 'the default branch'}</code>, or trigger it with the API.
            </div>
          {/if}
        </div>
      </section>
    {/if}
  </div>
</AppShell>

<!-- ===== add repository ===== -->
<dialog class="dlg" bind:this={addDlg} onclose={() => (adding = false)}>
  <div class="dlg-h">
    <h3>Add a repository</h3>
    <p>Orchestrator reads its pipeline files and registers a push webhook.</p>
  </div>
  <div class="dlg-b">
    <div class="field">
      <label for="repo-url">Repository URL</label>
      <input id="repo-url" bind:value={url} oninput={inspect} type="url" placeholder="https://git.example.com/you/service" autocomplete="off" />
      <p class="hint">
        Any Forgejo or Gitea repository the coordinator can reach. Pipelines are discovered from
        <code>.orchestrator/*.yml</code> on the default branch.
      </p>
    </div>
    {#if detecting || detected}
      <div class="detect">
        <div class="dh">{detecting ? 'Reading the URL…' : '✓ Looks like a repository'}</div>
        {#if detected}
          <div class="drow"><span class="dk">Name</span><span class="dv">{detected.name}</span></div>
          <div class="drow"><span class="dk">Owner</span><span class="dv">{detected.owner}</span></div>
          <!-- Deliberately not claiming what will be found: the coordinator
               clones and parses on add, and only then knows. -->
          <div class="drow"><span class="dk">Pipelines</span><span class="dv na">discovered on add</span></div>
        {/if}
      </div>
    {/if}
    {#if addError}<div class="err-banner" style="margin-top:12px">{addError}</div>{/if}
  </div>
  <div class="dlg-f">
    <button class="btn quiet" onclick={() => (adding = false)}>Cancel</button>
    <button class="btn primary" disabled={!detected} onclick={confirmAdd}>Add repository</button>
  </div>
</dialog>

<!-- ===== remove repository ===== -->
<dialog class="dlg" bind:this={rmDlg} onclose={() => (removeTarget = null)}>
  <div class="dlg-h">
    <h3>Remove {removeTarget?.name}?</h3>
    <p>The repository is unregistered. Its existing runs stay in history.</p>
  </div>
  <div class="dlg-f">
    <button class="btn quiet" onclick={() => (removeTarget = null)}>Cancel</button>
    <button class="btn danger" onclick={confirmRemove}>Remove repository</button>
  </div>
</dialog>
