<script lang="ts">
  // The Layout Mode canvas: the `.fm-canvas` wrapper plus one `<Band>` per part.
  // PURE (props in → DOM out) so the parity check can server-render it via
  // `svelte/server`'s `render` and compare against the askama golden. No `fm-*`
  // CSS here — it is inherited from the server's shell.html.
  import type { DesignModel } from './model';
  import Band from './Band.svelte';

  let { model }: { model: DesignModel } = $props();
</script>

<div class="fm-canvas" style={`width: ${model.width}px;`}>
  {#each model.parts as p (p.id)}
    <Band part={p} />
  {/each}
</div>
