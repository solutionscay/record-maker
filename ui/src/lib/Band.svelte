<script lang="ts">
  // One layout part rendered as a band of absolutely-positioned objects. This
  // mirrors the server's askama `_band.html` macro exactly so the client-side
  // canvas DOM is byte-identical (after normalization) to Browse's render.
  //
  // The canvas is display-only: a field ALWAYS renders its value span (no
  // inputs ever in Layout Mode), regardless of readOnly. All `fm-*` styling
  // comes from the inherited shell.html CSS — this component defines none.
  import type { ObjectView, PartView } from './model';

  let { part }: { part: PartView } = $props();

  // Build class/style as single template-literal strings so the markup carries
  // no stray whitespace inside tags (normalization only forgives whitespace
  // BETWEEN tags). Order: `fm-obj` [`fm-field`] [`fm-readonly`].
  function objClass(o: ObjectView): string {
    return `fm-obj${o.field ? ' fm-field' : ''}${o.readOnly ? ' fm-readonly' : ''}`;
  }

  function objStyle(o: ObjectView): string {
    return `left:${o.x}px; top:${o.y}px; width:${o.w}px; height:${o.h}px; z-index:${o.z};`;
  }
</script>

<div class="fm-part" style={`height: ${part.height}px;`}>
  {#each part.objects as o (o.id)}
    <div class={objClass(o)} style={objStyle(o)}>
      {#if o.field}
        <span class="fm-flabel">{o.label}</span><span class="fm-fvalue">{o.value}</span>
      {:else}
        <span class="fm-text">{o.value}</span>
      {/if}
    </div>
  {/each}
</div>
