<script lang="ts">
  import { login } from '../lib/api';

  let { onsuccess }: { onsuccess: () => void } = $props();

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      await login(username, password);
      onsuccess();
    } catch (err) {
      error = (err as Error).message || 'login failed';
    } finally {
      busy = false;
    }
  }
</script>

<div class="login-screen">
  <div class="login-band">
    <span class="wordmark">orchestrator</span>
  </div>
  <form class="card login-card" onsubmit={submit}>
    <h1>Sign in</h1>
    <p class="login-sub">This dashboard is protected — enter the credentials configured on the coordinator.</p>
    {#if error}<div class="err-banner login-err">{error}</div>{/if}
    <label class="login-field">
      <span>username</span>
      <!-- svelte-ignore a11y_autofocus — the login form is the only thing on screen -->
      <input type="text" bind:value={username} autocomplete="username" required autofocus />
    </label>
    <label class="login-field">
      <span>password</span>
      <input type="password" bind:value={password} autocomplete="current-password" required />
    </label>
    <button class="btn btn-lime login-btn" type="submit" disabled={busy || !username || !password}>
      {busy ? 'Signing in…' : 'Sign in'}
    </button>
    <p class="login-hint">Credentials come from <code>DASHBOARD_USERNAME</code> / <code>DASHBOARD_PASSWORD</code> in the coordinator's environment.</p>
  </form>
</div>
