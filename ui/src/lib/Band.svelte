<script lang="ts">
  // One layout part rendered as a band of absolutely-positioned objects. This
  // mirrors the server's askama `_band.html` macro exactly so the client-side
  // canvas DOM is byte-identical (after normalization) to Browse's render.
  //
  // The canvas is display-only and renders each object by kind (#60): a `field`
  // renders its VALUE span only (no inputs ever in Layout Mode, regardless of
  // readOnly) — its caption is a separate `text` object; a shape renders a styled
  // box from its server-derived `shapeStyle`; any other object is static `text`
  // from its `content` slot. All `fm-*` styling comes from the inherited
  // shell.html CSS — this component defines none.
  import type { ObjectView, PartView } from './model';

  let { part }: { part: PartView } = $props();

  // Build class/style as single template-literal strings so the markup carries
  // no stray whitespace inside tags (normalization only forgives whitespace
  // BETWEEN tags). Order: `fm-obj` [`fm-field`] [`fm-readonly`].
  function objClass(o: ObjectView): string {
    return `fm-obj${o.field ? ' fm-field' : ''}${o.readOnly ? ' fm-readonly' : ''}`;
  }

  function objStyle(o: ObjectView): string {
    return `left:${o.x}px; top:${o.y}px; width:${o.w}px; height:${o.h}px; z-index:${o.z};${o.objectStyle}`;
  }

  function fieldText(o: ObjectView): string {
    return o.label || o.binding || o.value;
  }
</script>

<!-- data-part-id / data-object-id mirror the askama macro: stable ids stamped
     on the DOM so hit-testing never maps DOM index to paint order (#134). -->
<div class="fm-part" style={`height: ${part.height}px;${part.partStyle}`} data-part-id={part.id}>
  {#each part.objects as o (o.id)}
    <div class={objClass(o)} style={objStyle(o)} data-object-id={o.id}>
      {#if o.field}
        <span class="fm-fvalue" style={o.textStyle || null}>{fieldText(o)}</span>
      {:else if o.shape}
        <div class="fm-shape fm-{o.kind}" style={o.shapeStyle}></div>
      {:else if o.kind === 'portal'}
        <!-- Portal (#168/#169): mirrors the askama `_band.html` branch. The design
             canvas passes no base record, so a portal here is never resolved and
             renders the route frame; a resolved Browse portal renders one row per
             related record, each stamped with its terminal id (#170/#172). Column
             headings remain ordinary authored text objects. `portalResolved` (not
             the column count) picks the branch, so a resolved portal with zero
             columns still renders cleanly. -->
        <div
          class="fm-portal"
          style={(o.portalRowHeight ?? 0) > 0 ? `--fm-portal-row-h: ${o.portalRowHeight}px` : null}
          data-route={o.binding}
        >
          {#if !o.portalResolved}
            <span class="fm-portal-tag">{o.binding}</span>
          {:else}
            {#each o.portalRows ?? [] as r (r.id)}
              <div class="fm-portal-row" data-related-id={r.id}>{#each r.cells as c, i}<span class="fm-portal-cell" style={`left:${o.portalColumnLefts?.[i] ?? 0}px;width:${o.portalColumnWidths?.[i] ?? 0}px`}>{c}</span>{/each}</div>
            {/each}
          {/if}
        </div>
      {:else}
        <span class="fm-text" style={o.textStyle || null}>{o.content}</span>
      {/if}
    </div>
  {/each}
</div>
