<script lang="ts">
  import type { Job } from '../types';
  let { jobs }: { jobs: Job[] } = $props();
</script>

<span class="strip" role="img" aria-label="{jobs.length} jobs">
  {#each jobs as j (j.id)}
    <span class="seg {j.status}" title="{j.name}: {j.status}"></span>
  {/each}
</span>
