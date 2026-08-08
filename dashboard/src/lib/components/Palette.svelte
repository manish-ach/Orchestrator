<script lang="ts">
  import type { Overview } from '../types';

  let { open = $bindable(false), overview = null }: { open?: boolean; overview?: Overview | null } =
    $props();

  type Dest = { label: string; href: string; kind: string };

  // Pages are always offered; the rest is built from whatever the poll last
  // returned, so the palette never lists a repo or worker that does not exist.
  const PAGES: Dest[] = [
    { label: 'Control center', href: '#/', kind: 'page' },
    { label: 'Runs', href: '#/runs', kind: 'page' },
    { label: 'Repositories', href: '#/repos', kind: 'page' },
    { label: 'Workers', href: '#/workers', kind: 'page' },
    { label: 'Insights', href: '#/insights', kind: 'page' },
  ];

  const dests = $derived.by<Dest[]>(() => {
    if (!overview) return PAGES;
    const repos = [...new Set(overview.runs.map((r) => r.repo).filter(Boolean))].map((name) => ({
      label: name,
      href: `#/repo/${encodeURIComponent(name)}`,
      kind: 'repo',
    }));
    const workers = overview.workers.map((w) => ({
      label: w.name,
      href: `#/workers/${encodeURIComponent(w.name)}`,
      kind: 'worker',
    }));
    const runs = [...overview.runs]
      .sort((a, b) => b.started_at - a.started_at)
      .slice(0, 8)
      .map((r) => ({
        label: `#${r.id} ${r.commit?.message ?? r.pipeline}`,
        href: `#/run/${r.id}`,
        kind: 'run',
      }));
    return [...PAGES, ...repos, ...workers, ...runs];
  });

  let query = $state('');
  let sel = $state(0);
  let input = $state<HTMLInputElement | null>(null);
  let dlg = $state<HTMLDialogElement | null>(null);

  const hits = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const out = q ? dests.filter((d) => d.label.toLowerCase().includes(q) || d.kind.includes(q)) : dests;
    return out.slice(0, 40);
  });

  // A native <dialog> gives focus trapping, Esc and an inert backdrop for free —
  // all of which a div would have to reimplement, badly.
  $effect(() => {
    if (!dlg) return;
    if (open && !dlg.open) {
      query = '';
      sel = 0;
      dlg.showModal();
      queueMicrotask(() => input?.focus());
    } else if (!open && dlg.open) {
      dlg.close();
    }
  });

  function go(d: Dest | undefined) {
    if (!d) return;
    location.hash = d.href.replace(/^#/, '');
    open = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      sel = Math.min(hits.length - 1, sel + 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      sel = Math.max(0, sel - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      go(hits[sel]);
    }
  }
</script>

<dialog
  class="pal"
  bind:this={dlg}
  aria-label="Search"
  onclose={() => (open = false)}
  onkeydown={onKey}
  onmousedown={(e) => {
    // a mousedown landing on the dialog box itself is the backdrop, since the
    // panel below fills it entirely
    if (e.target === dlg) open = false;
  }}
>
  <div class="pal-in">
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6">
      <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5 14 14" />
    </svg>
    <input
      bind:this={input}
      bind:value={query}
      oninput={() => (sel = 0)}
      placeholder="Jump to a page, repo, worker or run…"
      aria-label="Search"
    />
  </div>
  <div class="pal-list">
    {#each hits as d, i (d.href + d.label)}
      <button class="pal-i" class:on={i === sel} onmouseenter={() => (sel = i)} onclick={() => go(d)}>
        <span class="nm">{d.label}</span><span class="kind">{d.kind}</span>
      </button>
    {:else}
      <div class="pal-none">Nothing matches that.</div>
    {/each}
  </div>
  <div class="pal-f"><span>↑↓ move</span><span>⏎ open</span><span>esc close</span></div>
</dialog>

<style>
  .pal {
    width: 520px;
    max-width: 92vw;
    padding: 0;
    border: 0;
    margin-top: 12vh;
    background: var(--card);
    color: var(--ink);
    font-family: var(--ui);
    border-radius: 12px;
    box-shadow: 0 24px 70px oklch(0.2 0.01 110 / 0.3);
    overflow: hidden;
  }
  .pal::backdrop { background: oklch(0.2 0.01 110 / 0.34); }
  .pal-in {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--line);
    color: var(--faint);
  }
  .pal-in input {
    flex: 1;
    border: 0;
    outline: 0;
    background: none;
    font: inherit;
    font-size: 14px;
    color: var(--ink);
  }
  .pal-list { max-height: 340px; overflow-y: auto; padding: 6px; }
  .pal-i {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border: 0;
    border-radius: 7px;
    background: none;
    font: inherit;
    font-size: 13px;
    color: var(--ink);
    cursor: pointer;
    text-align: left;
  }
  .pal-i.on { background: oklch(0.955 0.02 118); }
  .pal-i.on .nm { color: var(--brand); font-weight: 600; }
  .nm { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .kind {
    margin-left: auto;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--faint);
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 1px 6px;
    flex: none;
  }
  .pal-none { padding: 24px; text-align: center; color: var(--muted); font-size: 13px; }
  .pal-f {
    border-top: 1px solid var(--line);
    padding: 8px 14px;
    font-size: 11px;
    color: var(--faint);
    display: flex;
    gap: 14px;
    font-family: var(--mono);
  }
</style>
