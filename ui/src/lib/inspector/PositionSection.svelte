<script lang="ts">
  // Single-object edge controls (#188). Values are part-relative grid pixels:
  // Left=x, Right=x+w, Top=y, Bottom=y+h. Moving one edge preserves its opposite
  // edge, and valid drafts update the canvas immediately like SizeSection.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import { applyLiveObjectGeometry, commitObjectGeometry } from './geometry-commit';
  import { edgeValue, geometryForEdge, type ObjectEdge } from './position';

  let {
    doc,
    layoutId = '',
    selected,
  }: { doc: EditorDoc; layoutId?: string; selected: Readonly<ObjectDoc> } = $props();

  const EDGES: readonly { edge: ObjectEdge; label: string }[] = [
    { edge: 'left', label: 'Left' },
    { edge: 'right', label: 'Right' },
    { edge: 'top', label: 'Top' },
    { edge: 'bottom', label: 'Bottom' },
  ];

  function pixels(raw: string): number | null {
    if (raw.trim() === '') return null;
    const value = Number(raw);
    return Number.isFinite(value) && value >= 0 ? Math.round(value) : null;
  }

  function applyLive(edge: ObjectEdge, value: number): boolean {
    const current = doc.getObject(selected.id);
    if (!current) return false;
    const geometry = geometryForEdge(current, edge, value);
    if (!geometry) return false;
    applyLiveObjectGeometry(doc, current.id, geometry);
    return true;
  }

  function inputEdge(edge: ObjectEdge, input: HTMLInputElement): void {
    const value = pixels(input.value);
    if (value !== null) applyLive(edge, value);
  }

  function commitEdge(edge: ObjectEdge, input: HTMLInputElement): void {
    const current = doc.getObject(selected.id);
    if (!current) return;
    const value = pixels(input.value);
    if (value === null || !applyLive(edge, value)) {
      // Empty, invalid, or crossed-edge drafts never poison geometry.
      input.value = String(edgeValue(current, edge));
    }
    commitObjectGeometry(doc, layoutId, current.id, 'position', edge);
  }

  function finishOnEnter(event: KeyboardEvent): void {
    if (event.key === 'Enter') (event.currentTarget as HTMLInputElement).blur();
  }
</script>

<section class="insp-sec">
  <span class="side-label">Position</span>
  {#each EDGES as { edge, label }}
    <div class="insp-row">
      <label for={`insp-${edge}-${selected.id}`}>{label}</label>
      <div class="insp-pixel-ctl">
        <input
          id={`insp-${edge}-${selected.id}`}
          class="ctl-num"
          type="number"
          min="0"
          step="1"
          value={edgeValue(selected, edge)}
          aria-label={`${label} position in pixels`}
          oninput={(e) => inputEdge(edge, e.currentTarget)}
          onchange={(e) => commitEdge(edge, e.currentTarget)}
          onkeydown={finishOnEnter}
        />
        <span>px</span>
      </div>
    </div>
  {/each}
</section>
