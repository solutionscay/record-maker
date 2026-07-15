<script lang="ts">
  // Shared single-object geometry controls (#187). The canvas resize controller
  // and this section mutate the same reactive EditorDoc w/h values, so handle
  // gestures update these inputs and valid typed pixels update the canvas live.
  // A change commit seals the live edits as one undo step and persists the full
  // geometry snapshot.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import { applyLiveObjectGeometry, commitObjectGeometry } from './geometry-commit';

  let {
    doc,
    layoutId = '',
    selected,
  }: { doc: EditorDoc; layoutId?: string; selected: Readonly<ObjectDoc> } = $props();

  type Dimension = 'w' | 'h';

  function pixels(raw: string): number | null {
    if (raw.trim() === '') return null;
    const value = Number(raw);
    return Number.isFinite(value) && value > 0 ? Math.max(1, Math.round(value)) : null;
  }

  /** Apply valid input immediately through the shared geometry path. */
  function applyLive(dimension: Dimension, value: number): void {
    const current = doc.getObject(selected.id);
    if (!current) return;
    applyLiveObjectGeometry(doc, current.id, { [dimension]: value });
  }

  function inputDimension(dimension: Dimension, input: HTMLInputElement): void {
    const value = pixels(input.value);
    if (value !== null) applyLive(dimension, value);
  }

  function commitDimension(dimension: Dimension, input: HTMLInputElement): void {
    const current = doc.getObject(selected.id);
    if (!current) return;
    const value = pixels(input.value);
    if (value === null) {
      // Empty/invalid drafts never poison geometry; restore the reactive value.
      input.value = String(current[dimension]);
    } else {
      applyLive(dimension, value);
    }

    commitObjectGeometry(doc, layoutId, current.id, 'size', dimension);
  }

  function finishOnEnter(event: KeyboardEvent): void {
    if (event.key === 'Enter') (event.currentTarget as HTMLInputElement).blur();
  }
</script>

<section class="insp-sec">
  <span class="side-label">Size</span>
  <div class="insp-row">
    <label for={`insp-width-${selected.id}`}>Width</label>
    <div class="insp-pixel-ctl">
      <input
        id={`insp-width-${selected.id}`}
        class="ctl-num"
        type="number"
        min="1"
        step="1"
        value={selected.w}
        aria-label="Width in pixels"
        oninput={(e) => inputDimension('w', e.currentTarget)}
        onchange={(e) => commitDimension('w', e.currentTarget)}
        onkeydown={finishOnEnter}
      />
      <span>px</span>
    </div>
  </div>
  <div class="insp-row">
    <label for={`insp-height-${selected.id}`}>Height</label>
    <div class="insp-pixel-ctl">
      <input
        id={`insp-height-${selected.id}`}
        class="ctl-num"
        type="number"
        min="1"
        step="1"
        value={selected.h}
        aria-label="Height in pixels"
        oninput={(e) => inputDimension('h', e.currentTarget)}
        onchange={(e) => commitDimension('h', e.currentTarget)}
        onkeydown={finishOnEnter}
      />
      <span>px</span>
    </div>
  </div>
</section>
