<script lang="ts">
  // Shared single-object geometry controls (#187). The canvas resize controller
  // and this section mutate the same reactive EditorDoc w/h values, so handle
  // gestures update these inputs and valid typed pixels update the canvas live.
  // A change commit seals the live edits as one undo step and persists the full
  // geometry snapshot.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import { llog } from '../log';
  import { linePropsForBox, lineShapeStyle, parseProps } from '../object-props';
  import { setObjectGeometry as persistGeometry } from '../persist';
  import { persistObjectPropsAndRefresh, reportPersistError } from './persist-ops';

  let {
    doc,
    layoutId = '',
    selected,
  }: { doc: EditorDoc; layoutId?: string; selected: Readonly<ObjectDoc> } = $props();

  type Dimension = 'w' | 'h';

  // Serialize commits from rapid Width -> Height edits so an older request can
  // never arrive last and overwrite the newer full-geometry snapshot.
  let persistQueue = Promise.resolve();

  function pixels(raw: string): number | null {
    if (raw.trim() === '') return null;
    const value = Number(raw);
    return Number.isFinite(value) && value > 0 ? Math.max(1, Math.round(value)) : null;
  }

  /** Apply valid input immediately. Lines also derive their visible stroke from
   * the new box, mirroring the canvas handle-resize path. */
  function applyLive(dimension: Dimension, value: number): void {
    const current = doc.getObject(selected.id);
    if (!current) return;
    doc.setObjectGeometry(current.id, { [dimension]: value });

    const resized = doc.getObject(current.id);
    if (!resized || resized.kind !== 'line') return;
    const nextProps = linePropsForBox(resized, parseProps(resized.props));
    doc.setObjectProps(resized.id, JSON.stringify(nextProps));

    // Shape styles are ordinarily server-derived. During live input, derive the
    // same line-only CSS locally (as handle resizing does), then refresh from the
    // authoritative server response after commit.
    const resolved = doc.getResolved(resized.id);
    if (resolved) {
      doc.setObjectStyles(resized.id, {
        objectStyle: resolved.objectStyle,
        textStyle: resolved.textStyle,
        shapeStyle: lineShapeStyle(nextProps),
      });
    }
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

    const committed = doc.getObject(current.id);
    if (!committed) return;
    const geometry = { x: committed.x, y: committed.y, w: committed.w, h: committed.h };
    const lineProps = committed.kind === 'line' ? parseProps(committed.props) : null;
    doc.mark();
    llog('persist', 'inspector: set object size', { id: committed.id, dimension, value: geometry[dimension] });

    persistQueue = persistQueue.then(async () => {
      try {
        await persistGeometry(layoutId, committed.id, geometry);
        if (lineProps) {
          await persistObjectPropsAndRefresh(doc, layoutId, committed.id, lineProps, 'set line size');
        }
      } catch (e) {
        reportPersistError(doc, 'set object size', e);
      }
    });
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
