<script lang="ts">
  // Layout-owned grid (#193). This section may appear from a band selection or
  // the empty canvas, but every instance writes the same /design/:layout/grid
  // resource — never band props.
  import type { EditorDoc } from '../doc.svelte';
  import { setLayoutGrid as persistLayoutGrid, type LayoutGridSettings } from '../persist';
  import { llog } from '../log';
  import { reportPersistError } from './persist-ops';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();
  let busy = $state(false);

  async function commit(patch: Partial<LayoutGridSettings>): Promise<void> {
    if (busy) return;
    const next: LayoutGridSettings = {
      gridSize: Math.max(1, Math.round(patch.gridSize ?? doc.gridSize)),
      showGrid: patch.showGrid ?? doc.showGrid,
      snapToGrid: patch.snapToGrid ?? doc.snapToGrid,
    };
    doc.setLayoutGrid(next.gridSize, next.showGrid, next.snapToGrid);
    busy = true;
    llog('persist', 'inspector: set layout grid', {
      gridSize: next.gridSize,
      showGrid: next.showGrid,
      snapToGrid: next.snapToGrid,
    });
    try {
      const saved = await persistLayoutGrid(layoutId, next);
      doc.setLayoutGrid(saved.gridSize, saved.showGrid, saved.snapToGrid);
    } catch (e) {
      reportPersistError(doc, 'set layout grid', e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="insp-sec">
  <span class="side-label">Layout Grid</span>
  <div class="insp-row">
    <span>Size</span>
    <div class="insp-ctls">
      <input
        class="ctl-num"
        type="number"
        min="1"
        step="1"
        disabled={busy}
        value={doc.gridSize}
        aria-label="Layout grid size in pixels"
        onchange={(e) => commit({ gridSize: Number(e.currentTarget.value) })}
      />
      <span>px</span>
    </div>
  </div>
  <div class="insp-row">
    <span>Show grid</span>
    <label class="toggle">
      <input
        type="checkbox"
        disabled={busy}
        checked={doc.showGrid}
        onchange={(e) => commit({ showGrid: e.currentTarget.checked })}
      />
      <span class="toggle-track"><span class="toggle-knob"></span></span>
    </label>
  </div>
  <div class="insp-row">
    <span>Snap to grid</span>
    <label class="toggle">
      <input
        type="checkbox"
        disabled={busy}
        checked={doc.snapToGrid}
        onchange={(e) => commit({ snapToGrid: e.currentTarget.checked })}
      />
      <span class="toggle-track"><span class="toggle-knob"></span></span>
    </label>
  </div>
  {#if doc.gridSize < 4 && doc.showGrid}
    <span class="le-hint">Fine grids show every tenth intersection for clarity; snapping still uses {doc.gridSize}px.</span>
  {/if}
</section>
