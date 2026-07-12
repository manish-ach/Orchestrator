<script lang="ts">
  let { name }: { name: string } = $props();

  const HUES = [280, 200, 30, 340, 110, 250];
  const hue = $derived.by(() => {
    let h = 0;
    for (const c of String(name || '?')) h = (h * 31 + c.charCodeAt(0)) | 0;
    return HUES[Math.abs(h) % HUES.length];
  });
</script>

<span class="avatar" style="background:oklch(0.52 0.11 {hue})" aria-hidden="true">
  {String(name || '?')[0]}
</span>
