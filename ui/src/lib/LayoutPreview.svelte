<script lang="ts">
  // The Layout Mode canvas: the `.fm-canvas` wrapper plus one `<Band>` per part.
  // PURE (props in → DOM out) so the parity check can server-render it via
  // `svelte/server`'s `render` and compare against the askama golden. No `fm-*`
  // CSS here — it is inherited from the server's shell.html.
  import type { DesignModel } from './model';
  import Band from './Band.svelte';
  import { tableColumnSettings } from './table-columns';

  let { model }: { model: DesignModel } = $props();

  function canvasPart(part: DesignModel['parts'][number]): DesignModel['parts'][number] {
    if (model.view !== 'table' || part.kind !== 'body') return part;
    return {
      ...part,
      objects: part.objects.filter(
        (object) =>
          object.kind !== 'field' ||
          object.parentObjectId !== undefined ||
          tableColumnSettings(object.props).visible,
      ),
    };
  }
</script>

<div class="fm-canvas" style={`width: ${model.width}px;`}>
  {#each model.parts as p (p.id)}
    <Band part={canvasPart(p)} />
  {/each}
</div>
