<script lang="ts">
  import { MODE } from '../api';
  import { back } from '../router';

  let {
    fallback = '/',
    children,
    actions,
  }: {
    fallback?: string;
    children?: import('svelte').Snippet;
    actions?: import('svelte').Snippet;
  } = $props();
</script>

<header class="snackbar">
  <button type="button" class="backbtn" onclick={() => back(fallback)}>←<span>back</span></button>
  <a class="mark" href="#/" aria-label="Overview"></a>
  {@render children?.()}
  <span class="sactions">
    <span class="mode-chip" class:live={MODE === 'live'}>{MODE}</span>
    {@render actions?.()}
  </span>
</header>
