<script lang="ts">
  import { onMount } from 'svelte';
  import { authRequired, getToken } from './lib/api';
  import { route } from './lib/router';
  import History from './pages/History.svelte';
  import Login from './pages/Login.svelte';
  import Overview from './pages/Overview.svelte';
  import RunDetail from './pages/RunDetail.svelte';
  import NotBuilt from './pages/NotBuilt.svelte';
  import Repos from './pages/Repos.svelte';
  import Workers from './pages/Workers.svelte';
  import Insights from './pages/Insights.svelte';

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
  {#if page === 'runs'}
    <History />
  {:else if page === 'repos'}
    <Repos />
  {:else if page === 'repo'}
    <!-- repo detail moved under Repositories; keep old links working -->
    {@const _ = (location.hash = `/repos/${$route.path[1] ?? ''}`)}
    <Repos />
  {:else if page === 'run'}
    {#key $route.path[1]}
      <RunDetail id={$route.path[1] ?? ''} initialJob={$route.query.get('job')} />
    {/key}
  {:else if page === 'workers'}
    <Workers />
  {:else if page === 'monitor'}
    <!-- the fleet view moved to Workers; keep old links and bookmarks working -->
    {@const _ = (location.hash = '/workers')}
    <Workers />
  {:else if page === 'insights'}
    <Insights />
  {:else if page === '' || page === 'overview'}
    <Overview />
  {:else}
    <NotBuilt path={$route.path.join('/')} />
  {/if}
{/if}
