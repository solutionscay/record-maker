<script lang="ts">
  // Shared single- and multi-object geometry controls (#187/#190). The canvas
  // resize controller and this section mutate the same reactive EditorDoc w/h
  // values, so handle gestures update these inputs and valid typed pixels update
  // the canvas live. A change commit seals every selected object's edits as one
  // undo step and persists their full geometry snapshots.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import { commitObjectGeometries } from './geometry-commit';
  import { applyLiveDimension, dimensionPixels, sharedDimension, type Dimension } from './size';

  let {
    doc,
    layoutId = '',
    objects,
  }: { doc: EditorDoc; layoutId?: string; objects: readonly Readonly<ObjectDoc>[] } = $props();

  let ids = $derived(objects.map((object) => object.id));
  let width = $derived(sharedDimension(objects, 'w'));
  let height = $derived(sharedDimension(objects, 'h'));

  /** Apply valid input immediately through the shared geometry path. */
  function applyLive(dimension: Dimension, value: number): void {
    applyLiveDimension(doc, ids, dimension, value);
  }

  function inputDimension(dimension: Dimension, input: HTMLInputElement): void {
    const value = dimensionPixels(input.value);
    if (value !== null) applyLive(dimension, value);
  }

  function commitDimension(dimension: Dimension, input: HTMLInputElement): void {
    const current = sharedDimension(
      ids.map((id) => doc.getObject(id)).filter((object): object is Readonly<ObjectDoc> => !!object),
      dimension,
    );
    const value = dimensionPixels(input.value);
    if (value === null) {
      // Empty/invalid drafts never poison geometry; restore the reactive value.
      input.value = current.mixed || current.value === null ? '' : String(current.value);
    } else {
      applyLive(dimension, value);
    }

    commitObjectGeometries(doc, layoutId, ids, 'size', dimension);
  }

  function finishOnEnter(event: KeyboardEvent): void {
    if (event.key === 'Enter') (event.currentTarget as HTMLInputElement).blur();
  }
</script>

<section class="insp-sec">
  <span class="side-label">Size</span>
  <div class="insp-row">
    <label for={`insp-width-${ids.join('-')}`}>Width</label>
    <div class="insp-pixel-ctl">
      <input
        id={`insp-width-${ids.join('-')}`}
        class="ctl-num"
        type="number"
        min="1"
        step="1"
        placeholder={width.mixed ? 'Mixed' : ''}
        value={width.mixed ? '' : (width.value ?? '')}
        aria-label="Width in pixels"
        oninput={(e) => inputDimension('w', e.currentTarget)}
        onchange={(e) => commitDimension('w', e.currentTarget)}
        onkeydown={finishOnEnter}
      />
      <span>px</span>
    </div>
  </div>
  <div class="insp-row">
    <label for={`insp-height-${ids.join('-')}`}>Height</label>
    <div class="insp-pixel-ctl">
      <input
        id={`insp-height-${ids.join('-')}`}
        class="ctl-num"
        type="number"
        min="1"
        step="1"
        placeholder={height.mixed ? 'Mixed' : ''}
        value={height.mixed ? '' : (height.value ?? '')}
        aria-label="Height in pixels"
        oninput={(e) => inputDimension('h', e.currentTarget)}
        onchange={(e) => commitDimension('h', e.currentTarget)}
        onkeydown={finishOnEnter}
      />
      <span>px</span>
    </div>
  </div>
</section>
