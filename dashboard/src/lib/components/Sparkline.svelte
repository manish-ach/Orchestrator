<script lang="ts">
  // A bare trend line for KPI tiles. Scaled to its own min/max rather than to
  // zero: these sit behind a headline number that already carries the absolute
  // value, so the shape is the only thing worth reading here.
  let {
    values = [],
    tone = 'brand',
    width = 150,
    height = 34,
  }: {
    values?: number[];
    tone?: 'brand' | 'ok' | 'fail' | 'run';
    width?: number;
    height?: number;
  } = $props();

  const COLOUR = {
    brand: 'oklch(0.50 0.105 112)',
    ok: 'oklch(0.53 0.13 145)',
    fail: 'oklch(0.53 0.19 27)',
    run: 'oklch(0.63 0.13 78)',
  } as const;

  const pts = $derived.by(() => {
    if (values.length < 2) return [];
    const min = Math.min(...values), max = Math.max(...values);
    const span = max - min || 1;
    const pad = 3;
    return values.map((v, i) => [
      (i / (values.length - 1)) * width,
      height - pad - ((v - min) / span) * (height - pad * 2),
    ]);
  });
  const line = $derived(pts.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(' '));
</script>

{#if pts.length}
  <svg class="spark" {width} {height} aria-hidden="true">
    <polygon points="0,{height} {line} {width},{height}" fill={COLOUR[tone]} opacity="0.1" />
    <polyline points={line} fill="none" stroke={COLOUR[tone]} stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round" />
  </svg>
{/if}

<style>
  .spark { position: absolute; right: 0; bottom: 0; opacity: 0.9; pointer-events: none; }
</style>
