<script lang="ts">
  // A scope control that actually filters. Options are passed in from whatever
  // the page has loaded, so it can never offer a repo or branch that does not
  // exist in the data — an empty result from a stale option is worse than no
  // control at all.
  let {
    label,
    value = $bindable('all'),
    options = [],
  }: { label: string; value?: string; options?: string[] } = $props();

  let open = $state(false);
  let root = $state<HTMLElement | null>(null);

  $effect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    document.addEventListener('click', close);
    return () => document.removeEventListener('click', close);
  });
</script>

<div class="fdrop-wrap" bind:this={root}>
  <button
    class="fdrop"
    aria-haspopup="listbox"
    aria-expanded={open}
    disabled={options.length <= 1}
    onclick={(e) => { e.stopPropagation(); open = !open; }}
  >
    <span class="k">{label}</span>
    {value}
  </button>
  {#if open}
    <div class="fmenu" role="listbox" tabindex="-1">
      {#each options as o (o)}
        <button class="fopt" class:sel={o === value} role="option" aria-selected={o === value} onclick={() => { value = o; open = false; }}>
          <span class="tick">{o === value ? '✓' : ''}</span>{o}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .fdrop-wrap { position: relative; flex: none; }
  .fdrop {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--line-2);
    border-radius: 8px;
    background: var(--card);
    font: inherit;
    font-size: 12.5px;
    color: var(--ink-2);
    cursor: pointer;
    white-space: nowrap;
  }
  .fdrop:hover:not(:disabled) { border-color: oklch(0.78 0.008 110); color: var(--ink); }
  /* nothing to choose between: shown, but visibly inert rather than fake */
  .fdrop:disabled { opacity: 0.55; cursor: default; }
  .fdrop .k { color: var(--faint); }
  .fdrop::after { content: '▾'; color: var(--faint); font-size: 10px; }
  .fmenu {
    position: absolute;
    top: calc(100% + 5px);
    left: 0;
    z-index: 40;
    min-width: 180px;
    max-height: 300px;
    overflow-y: auto;
    background: var(--card);
    border: 1px solid var(--line-2);
    border-radius: 9px;
    box-shadow: var(--shadow-hi);
    padding: 5px;
  }
  .fopt {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 9px;
    border: 0;
    border-radius: 6px;
    background: none;
    font: inherit;
    font-size: 12.5px;
    color: var(--ink-2);
    cursor: pointer;
    white-space: nowrap;
    text-align: left;
  }
  .fopt:hover { background: var(--surface); color: var(--ink); }
  .fopt.sel { color: var(--brand); font-weight: 600; }
  .tick { width: 12px; font-size: 11px; }
</style>
