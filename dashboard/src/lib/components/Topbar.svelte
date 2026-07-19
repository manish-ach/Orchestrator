<script lang="ts">
  import { MODE } from '../api';
  import { lastFetch, now } from '../poll';
  import type { Overview } from '../types';

  let {
    active,
    overview = null,
    children,
  }: {
    active: string;
    overview?: Overview | null;
    children?: import('svelte').Snippet;
  } = $props();

  const readout = $derived.by(() => {
    if (!overview) return '';
    const online = overview.workers.filter((w) => w.status === 'online').length;
    const jobs = overview.runs.flatMap((r) => r.jobs);
    const running = jobs.filter((j) => j.status === 'running').length;
    const activeRun = overview.runs.find((r) => r.status === 'running');
    const queued = jobs.filter(
      (j) => j.status === 'pending' && j.run_id === activeRun?.id,
    ).length;
    return `${online}/${overview.workers.length} workers · ${running} running · queue ${queued}`;
  });

  const updated = $derived(`updated ${Math.max(0, Math.round(($now - $lastFetch) / 1000))}s ago`);
</script>

<div class="band">
  <header class="topbar">
    <div class="wrap topbar-inner">
      <a class="wordmark" href="#/">orchestrator</a>
      <span class="crumb"></span>
      <span class="topbar-status">
        <span class="readout">{readout}</span>
        <span class="mode-chip" class:live={MODE === 'live'}>{MODE}</span>
        <span class="readout">{updated}</span>
      </span>
    </div>
    <nav class="wrap topnav" aria-label="Pages">
      <a href="#/" aria-current={active === 'overview' ? 'page' : undefined}>overview</a>
      <a href="#/runs" aria-current={active === 'runs' ? 'page' : undefined}>runs</a>
      <a href="#/repos" aria-current={active === 'repos' ? 'page' : undefined}>repos</a>
      <a href="#/monitor" aria-current={active === 'monitor' ? 'page' : undefined}>monitor</a>
    </nav>
  </header>
  {@render children?.()}
</div>
