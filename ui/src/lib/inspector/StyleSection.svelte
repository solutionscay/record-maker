<script lang="ts">
  // Style section: fill / border (and a line's angle when exactly one line is
  // selected). ONE panel for single- and multi-selections (#135): every control
  // resolves its value across the whole selection via `sharedValue` (a 1-element
  // selection is simply never mixed) and writes fan out to all selected objects
  // as one undo step. Callers gate the section by the shared fill capability.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import {
    hasAllOuterStrokeSides,
    strokeSides,
    withAllOuterStrokeSides,
    withStrokeSide,
    type StrokeSide,
  } from '../border-sides';
  import { setObjectGeometry as persistGeometry } from '../persist';
  import { llog } from '../log';
  import {
    lineGeometryForAngle as resolvedLineGeometryForAngle,
    lineLength as resolvedLineLength,
    normalizeAngle,
    parseProps,
  } from '../object-props';
  import { colorValue, numberValue, sharedValue } from './values';
  import {
    persistObjectPropsAndRefresh,
    reportPersistError,
    transformStyleMany,
    writeStyleMany,
  } from './persist-ops';

  let {
    doc,
    layoutId = '',
    objects,
  }: { doc: EditorDoc; layoutId?: string; objects: readonly Readonly<ObjectDoc>[] } = $props();

  let mFill = $derived(sharedValue(objects, (p) => colorValue(p.fill, '#f7f8fa')));
  let mStrokeWidth = $derived(sharedValue(objects, (p) => numberValue(p.strokeWidth, 1)));
  let mStroke = $derived(sharedValue(objects, (p) => colorValue(p.stroke, '#d3d8de')));

  // Border placement belongs only to rectangular boxes. Lines and ellipses keep
  // their existing uniform stroke control. Middle is available only when every
  // selected box is a portal, so a mixed field+portal selection cannot write an
  // invalid portal-only placement onto fields.
  let hasBorderPlacement = $derived(
    objects.length > 0 && objects.every((o) => ['field', 'rect', 'portal'].includes(o.kind)),
  );
  let allPortals = $derived(objects.length > 0 && objects.every((o) => o.kind === 'portal'));
  let mAllSides = $derived(sharedValue(objects, (p) => hasAllOuterStrokeSides(strokeSides(p))));
  let mTop = $derived(sharedValue(objects, (p) => strokeSides(p).includes('top')));
  let mRight = $derived(sharedValue(objects, (p) => strokeSides(p).includes('right')));
  let mBottom = $derived(sharedValue(objects, (p) => strokeSides(p).includes('bottom')));
  let mLeft = $derived(sharedValue(objects, (p) => strokeSides(p).includes('left')));
  let mMiddle = $derived(sharedValue(objects, (p) => strokeSides(p).includes('middle')));

  const sideButtons: { side: Exclude<StrokeSide, 'middle'>; label: string; path: string }[] = [
    { side: 'left', label: 'Left border', path: 'M3 2V16' },
    { side: 'right', label: 'Right border', path: 'M15 2V16' },
    { side: 'top', label: 'Top border', path: 'M2 3H16' },
    { side: 'bottom', label: 'Bottom border', path: 'M2 15H16' },
  ];

  // A line's angle control applies to a SINGLE selected line only (a multi
  // selection never shows it — matching size/direction across lines is undefined).
  let line = $derived(objects.length === 1 && objects[0].kind === 'line' ? objects[0] : null);
  let lineProps = $derived(parseProps(line?.props ?? ''));

  function setStyle(key: string, value: string | number | boolean): void {
    void writeStyleMany(doc, layoutId, objects.map((o) => o.id), key, value);
  }

  function sideState(side: StrokeSide): { mixed: boolean; value: boolean } {
    switch (side) {
      case 'top': return mTop;
      case 'right': return mRight;
      case 'bottom': return mBottom;
      case 'left': return mLeft;
      case 'middle': return mMiddle;
    }
  }

  function setSide(side: StrokeSide): void {
    const state = sideState(side);
    const enabled = state.mixed || !state.value;
    void transformStyleMany(doc, layoutId, objects.map((o) => o.id), `set ${side} border`, (props) => ({
      ...props,
      strokeSides: withStrokeSide(props, side, enabled),
    }));
  }

  function setAllSides(): void {
    const enabled = mAllSides.mixed || !mAllSides.value;
    void transformStyleMany(doc, layoutId, objects.map((o) => o.id), 'set all outer borders', (props) => ({
      ...props,
      strokeSides: withAllOuterStrokeSides(props, enabled),
    }));
  }

  async function setLineAngle(value: number): Promise<void> {
    if (!line) return;
    const angle = normalizeAngle(value);
    const length = resolvedLineLength(line, lineProps);
    const geom = resolvedLineGeometryForAngle(line, angle, length);
    if (!geom) return;
    const next = { ...lineProps, angle, length };
    llog('persist', 'inspector: set line angle', { id: line.id, angle, length, geom });
    doc.setObjectGeometry(line.id, geom);
    doc.setObjectProps(line.id, JSON.stringify(next));
    doc.mark();
    try {
      await persistGeometry(layoutId, line.id, geom);
      await persistObjectPropsAndRefresh(doc, layoutId, line.id, next, 'set line angle');
    } catch (e) {
      reportPersistError(doc, 'set line angle', e);
    }
  }
</script>

<section class="insp-sec">
  <span class="side-label">Style</span>
  <div class="insp-row">
    <span>Fill</span>
    <div class="insp-ctls">
      {#if mFill.mixed}<span class="mixed-tag">Mixed</span>{/if}
      <input
        class="swatch"
        type="color"
        value={mFill.value}
        onchange={(e) => setStyle('fill', e.currentTarget.value)}
      />
    </div>
  </div>
  {#if hasBorderPlacement}
    <div class="insp-row border-placement-row">
      <span>Placement</span>
      <div class="border-grid" aria-label="Border placement">
        <button
          type="button"
          class="border-btn"
          class:active={!mAllSides.mixed && mAllSides.value}
          class:mixed={mAllSides.mixed}
          title="All outer borders"
          aria-label="All outer borders"
          aria-pressed={mAllSides.mixed ? 'mixed' : mAllSides.value}
          onclick={setAllSides}
        >
          <svg viewBox="0 0 18 18" aria-hidden="true"><rect class="border-guide" x="3" y="3" width="12" height="12" /><rect class="border-mark" x="3" y="3" width="12" height="12" /></svg>
        </button>
        {#each sideButtons as button}
          {@const state = sideState(button.side)}
          <button
            type="button"
            class="border-btn"
            class:active={!state.mixed && state.value}
            class:mixed={state.mixed}
            title={button.label}
            aria-label={button.label}
            aria-pressed={state.mixed ? 'mixed' : state.value}
            onclick={() => setSide(button.side)}
          >
            <svg viewBox="0 0 18 18" aria-hidden="true"><rect class="border-guide" x="3" y="3" width="12" height="12" /><path class="border-mark" d={button.path} /></svg>
          </button>
        {/each}
        {#if allPortals}
          <button
            type="button"
            class="border-btn"
            class:active={!mMiddle.mixed && mMiddle.value}
            class:mixed={mMiddle.mixed}
            title="Middle row separators"
            aria-label="Middle row separators"
            aria-pressed={mMiddle.mixed ? 'mixed' : mMiddle.value}
            onclick={() => setSide('middle')}
          >
            <svg viewBox="0 0 18 18" aria-hidden="true"><rect class="border-guide" x="3" y="3" width="12" height="12" /><path class="border-mark" d="M3 9H15" /></svg>
          </button>
        {/if}
      </div>
    </div>
  {/if}
  <div class="insp-row">
    <span>Border</span>
    <div class="insp-ctls">
      <input
        class="ctl-num"
        type="number"
        min="0"
        max="12"
        placeholder={mStrokeWidth.mixed ? 'Mixed' : ''}
        value={mStrokeWidth.mixed ? '' : mStrokeWidth.value}
        onchange={(e) => setStyle('strokeWidth', Number(e.currentTarget.value))}
      />
      {#if mStroke.mixed}<span class="mixed-tag">Mixed</span>{/if}
      <input
        class="swatch"
        type="color"
        value={mStroke.value}
        onchange={(e) => setStyle('stroke', e.currentTarget.value)}
      />
    </div>
  </div>
  {#if line}
    <div class="insp-row">
      <span>Angle</span>
      <input
        class="ctl-num"
        type="number"
        min="0"
        max="359"
        step="1"
        value={numberValue(lineProps.angle, 0)}
        onchange={(e) => setLineAngle(Number(e.currentTarget.value))}
      />
    </div>
  {/if}
</section>
