<script lang="ts">
  import { onMount } from 'svelte';
  import { authRequired, getToken } from './lib/api';
  import { route } from './lib/router';
  import Login from './pages/Login.svelte';
  import Monitor from './pages/Monitor.svelte';
  import Overview from './pages/Overview.svelte';
  import RepoDetail from './pages/RepoDetail.svelte';
  import RunDetail from './pages/RunDetail.svelte';
  import Repos from './pages/Repos.svelte';

  const page = $derived($route.path[0] ?? '');

  // 'checking' until /api/auth/status answers; 'login' gates every page
  // until a session token exists. A 401 from any call re-opens the gate.
  let auth = $state<'checking' | 'login' | 'ok'>('checking');

  onMount(() => {
    authRequired().then((required) => {
      auth = required && !getToken() ? 'login' : 'ok';
    });
    const onUnauthorized = () => (auth = 'login');
    window.addEventListener('dash:unauthorized', onUnauthorized);
    return () => window.removeEventListener('dash:unauthorized', onUnauthorized);
  });
</script>

{#if auth === 'login'}
  <Login onsuccess={() => (auth = 'ok')} />
{:else if auth === 'ok'}
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
{/if}
