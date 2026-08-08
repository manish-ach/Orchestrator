<script lang="ts">
  // OS marks drawn inline. The dashboard is served by the coordinator on a
  // private network that may have no route to the internet, so a remote logo
  // asset is a broken image waiting to happen.
  //
  // Keyed by the `os_id` a worker reports (sysinfo's distribution_id), with a
  // generic Tux for any Linux we do not have a mark for. An unrecognised id
  // gets a neutral chip rather than a wrong logo — claiming a machine is Arch
  // when it is Void is worse than saying nothing.

  let { id = '', size = 22 }: { id?: string; size?: number } = $props();

  const KNOWN = ['macos', 'windows', 'ubuntu', 'debian', 'arch', 'fedora', 'alpine', 'linux'];
  const LINUXY = ['linux', 'raspbian', 'manjaro', 'endeavouros', 'pop', 'mint', 'gentoo', 'nixos', 'void', 'opensuse'];

  const kind = $derived(
    KNOWN.includes(id) ? id : LINUXY.includes(id) ? 'linux' : id.includes('linux') ? 'linux' : 'unknown',
  );
  /** Two letters for an OS with no mark — still identifies the machine. */
  const initials = $derived((id || '?').slice(0, 2).toUpperCase());
</script>

<span class="oslogo" style="width:{size}px;height:{size}px" title={id || 'unknown OS'} aria-label={id || 'unknown OS'}>
  {#if kind === 'macos'}
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path
        d="M16.4 5.1c.85-1.03 1.42-2.45 1.24-3.9-1.22.05-2.7.82-3.58 1.86-.78.92-1.47 2.38-1.29 3.78 1.36.1 2.76-.7 3.63-1.74z"
      />
      <path
        d="M20.2 12.4c.02-2.9 2.37-4.34 2.47-4.4-1.35-1.97-3.44-2.24-4.18-2.27-1.78-.18-3.47 1.05-4.38 1.05-.9 0-2.29-1.02-3.76-1-1.93.03-3.72 1.12-4.71 2.85-2.01 3.48-.51 8.63 1.44 11.46.95 1.38 2.09 2.93 3.58 2.87 1.44-.06 1.98-.93 3.72-.93s2.23.93 3.75.9c1.55-.03 2.53-1.4 3.48-2.79 1.1-1.6 1.55-3.15 1.57-3.23-.03-.01-3.01-1.16-3.04-4.6z"
      />
    </svg>
  {:else if kind === 'windows'}
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M3 5.4 10.4 4.3v7.2H3V5.4zM11.6 4.1 21 2.7v8.8h-9.4V4.1zM3 12.5h7.4v7.2L3 18.6v-6.1zM11.6 12.5H21v8.8l-9.4-1.4v-7.4z" />
    </svg>
  {:else if kind === 'ubuntu'}
    <!-- circle of friends: three nodes on a broken ring -->
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <g fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
        <path d="M7.1 5.4a8 8 0 0 1 9.6 1.1" />
        <path d="M19.4 9.1a8 8 0 0 1-2.7 8.4" />
        <path d="M13.6 19.7a8 8 0 0 1-8.3-4.4" />
      </g>
      <g fill="currentColor">
        <circle cx="4.3" cy="12" r="2.4" />
        <circle cx="17.8" cy="5.2" r="2.4" />
        <circle cx="17.8" cy="18.8" r="2.4" />
      </g>
    </svg>
  {:else if kind === 'debian'}
    <!-- the swirl: an open ring with an inward curl -->
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" aria-hidden="true">
      <path d="M18.6 7.4a8 8 0 1 0 1 6.4" stroke-width="1.9" stroke-linecap="round" />
      <path d="M15.7 10.1a4.4 4.4 0 1 0-.6 4.5" stroke-width="1.7" stroke-linecap="round" />
    </svg>
  {:else if kind === 'arch'}
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path
        d="M12 1.8c-1 2.4-1.6 4-2.7 6.3.7.7 1.5 1.6 2.8 2.5-1.4-.6-2.4-1.2-3.1-1.8-1.4 2.9-3.6 7-8 15 3.5-2 6.2-3.2 8.7-3.7-.1-.5-.2-1-.2-1.5v-.1c0-1.6 1-2.9 2.2-2.8 1.2 0 2.1 1.3 2.1 3 0 .5 0 1-.2 1.4 2.5.5 5.2 1.7 8.6 3.7-.7-1.2-1.3-2.4-1.9-3.5-.9-.7-1.9-1.6-3.9-2.6 1.4.4 2.4.8 3.1 1.3C14.9 8.6 14.5 7.3 12 1.8z"
      />
    </svg>
  {:else if kind === 'fedora'}
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path
        d="M12 2.2A9.8 9.8 0 0 0 2.2 12v7.1c0 1.5 1.2 2.7 2.7 2.7H12a9.8 9.8 0 0 0 0-19.6zm1.5 4.2c1.5 0 2.7 1.2 2.7 2.7h-2.2c-.3 0-.5.2-.5.5v1.7h2.2v2.2h-2.2v2.3c0 1.5-1.2 2.7-2.7 2.7A2.7 2.7 0 0 1 8 15.8h2.2c.3 0 .5-.2.5-.5v-1.8H8.5v-2.2h2.2V9c0-1.5 1.2-2.6 2.8-2.6z"
      />
    </svg>
  {:else if kind === 'alpine'}
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M9.1 4.2 2 20h20L14.9 4.2H9.1zm2.9 3.4 4.6 10.2H14l-2-4.5-2 4.5H7.4L12 7.6z" />
    </svg>
  {:else if kind === 'linux'}
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path
        d="M12 1.6c-2.36 0-3.94 1.86-3.94 4.36v2.5c0 1.09-1.38 2.42-2.24 3.9C4.66 14.3 3.4 16.86 3.4 18.8c0 2.06 1.5 2.82 3.1 3.13 1.06.2 1.4.95 2.14.95h6.72c.74 0 1.08-.75 2.14-.95 1.6-.31 3.1-1.07 3.1-3.13 0-1.94-1.26-4.5-2.42-6.44-.86-1.48-2.24-2.81-2.24-3.9v-2.5C15.94 3.46 14.36 1.6 12 1.6z"
        fill="currentColor"
      />
      <ellipse cx="10.1" cy="6.4" rx="1.25" ry="1.5" fill="#fff" />
      <ellipse cx="13.9" cy="6.4" rx="1.25" ry="1.5" fill="#fff" />
      <circle cx="10.35" cy="6.7" r=".6" fill="#1b2118" />
      <circle cx="13.65" cy="6.7" r=".6" fill="#1b2118" />
      <path d="M12 8.1c-1.05 0-1.95.72-1.95 1.4S10.95 11 12 11s1.95-.72 1.95-1.5S13.05 8.1 12 8.1z" fill="#f2b01e" />
    </svg>
  {:else}
    <span class="init" style="font-size:{Math.round(size * 0.42)}px">{initials}</span>
  {/if}
</span>

<style>
  .oslogo {
    display: inline-grid;
    place-items: center;
    flex: none;
    color: inherit;
  }
  .oslogo svg {
    width: 100%;
    height: 100%;
  }
  .init {
    font-family: var(--mono);
    font-weight: 600;
    letter-spacing: 0.02em;
    opacity: 0.75;
  }
</style>
