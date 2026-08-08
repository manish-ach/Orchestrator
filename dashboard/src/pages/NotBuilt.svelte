<script lang="ts">
  import AppShell from '../lib/components/AppShell.svelte';

  // Reached when a hash route resolves to no page — a typo, a stale bookmark, or
  // a page the redesign has specified but not yet ported. Naming the route beats
  // silently redirecting to the overview, which hides the mistake.
  let { path = '' }: { path?: string } = $props();
</script>

<AppShell active="control">
  <div class="page-body nb">
    <div class="card">
      <svg viewBox="0 0 420 128" aria-label="A pipeline with a broken final edge">
        <rect class="n" x="6" y="46" width="96" height="36" rx="8" />
        <circle cx="24" cy="64" r="5" fill="oklch(0.66 0.14 145)" />
        <text class="lb" x="38" y="68">resolve</text>
        <path class="ed" d="M102 64 C126 64, 130 64, 154 64" />
        <rect class="n" x="154" y="46" width="96" height="36" rx="8" />
        <circle cx="172" cy="64" r="5" fill="oklch(0.66 0.14 145)" />
        <text class="lb" x="186" y="68">match</text>
        <path class="ed bad" d="M250 64 C274 64, 278 64, 302 64" />
        <rect class="nd" x="302" y="46" width="112" height="36" rx="8" />
        <text class="lbd" x="316" y="68">✕  no route</text>
      </svg>
      <div class="code">404</div>
      <div class="msg">This route has no page behind it</div>
      <div class="sub">The coordinator is fine. The address is not.</div>
      <div class="path">#/{path}</div>
      <div class="acts">
        <a class="btn lime" href="#/">Control center</a>
        <a class="btn ghost" href="#/runs">Runs</a>
        <a class="btn ghost" href="#/repos">Repositories</a>
        <a class="btn ghost" href="#/workers">Workers</a>
      </div>
    </div>
  </div>
</AppShell>

<style>
  .nb { display: grid; place-items: center; }
  .card {
    width: min(720px, 100%);
    background: oklch(0.185 0.008 110);
    border-radius: 14px;
    box-shadow: var(--shadow-hi);
    color: oklch(0.88 0.008 110);
    padding: 30px 34px 26px;
    text-align: center;
  }
  svg { width: 100%; max-width: 400px; height: auto; margin-bottom: 6px; }
  .n { fill: oklch(0.245 0.012 110); stroke: oklch(0.4 0.014 110); stroke-width: 1.5; }
  .nd { fill: oklch(0.205 0.012 110); stroke: oklch(0.45 0.14 27); stroke-width: 1.5; stroke-dasharray: 4 3; }
  .lb { font-family: var(--mono); font-size: 10px; fill: oklch(0.74 0.012 110); }
  .lbd { font-family: var(--mono); font-size: 10px; fill: oklch(0.68 0.15 27); }
  .ed { fill: none; stroke: oklch(0.55 0.1 145); stroke-width: 2; }
  .ed.bad { stroke: oklch(0.55 0.16 27); stroke-dasharray: 6 5; }
  .code {
    font-family: var(--mono);
    font-size: 52px;
    font-weight: 700;
    letter-spacing: -0.05em;
    color: var(--lime);
    line-height: 1;
  }
  .msg { font-size: 15px; font-weight: 600; color: #fff; margin-top: 12px; }
  .sub { font-size: 13px; color: oklch(0.72 0.01 110); margin-top: 6px; }
  .path {
    display: inline-block;
    font-family: var(--mono);
    font-size: 12.5px;
    color: var(--lime);
    background: oklch(0.245 0.012 110);
    border: 1px solid oklch(0.32 0.012 110);
    border-radius: 6px;
    padding: 3px 9px;
    margin-top: 14px;
  }
  .acts { display: flex; gap: 9px; justify-content: center; flex-wrap: wrap; margin-top: 22px; }
  .btn {
    display: inline-flex;
    align-items: center;
    height: 34px;
    padding: 0 14px;
    border-radius: 8px;
    font-size: 12.5px;
    font-weight: 600;
    border: 1px solid transparent;
    text-decoration: none;
  }
  .btn.lime { background: var(--lime); color: oklch(0.2 0.02 118); }
  .btn.ghost { border-color: oklch(0.34 0.012 110); color: oklch(0.82 0.01 110); }
  .btn.ghost:hover { border-color: var(--lime); color: var(--lime); text-decoration: none; }
</style>
