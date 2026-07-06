<script lang="ts">
  import { GLYPH } from '../format';
  let { status }: { status: string } = $props();
</script>

<span class="st st-{status}"><span class="g">{GLYPH[status] ?? '·'}</span>{status}</span>
