<script lang="ts">
  import { route } from './lib/router';
  import Overview from './pages/Overview.svelte';
  import Repos from './pages/Repos.svelte';
  import RepoDetail from './pages/RepoDetail.svelte';
  import RunDetail from './pages/RunDetail.svelte';
  import Monitor from './pages/Monitor.svelte';

  const page = $derived($route.path[0] ?? '');
</script>

{#if page === 'repos'}
  <Repos />
{:else if page === 'repo'}
  {#key $route.path[1]}
    <RepoDetail name={decodeURIComponent($route.path[1] ?? '')} />
  {/key}
{:else if page === 'run'}
  {#key $route.path[1]}
    <RunDetail id={$route.path[1] ?? ''} initialJob={$route.query.get('job')} />
  {/key}
{:else if page === 'monitor'}
  <Monitor />
{:else}
  <Overview />
{/if}
