<script lang="ts">
  // Style section: fill / border (and a line's angle when exactly one line is
  // selected). ONE panel for single- and multi-selections (#135): every control
  // resolves its value across the whole selection via `sharedValue` (a 1-element
  // selection is simply never mixed) and writes fan out to all selected objects
  // as one undo step. Callers gate the section by the shared fill capability.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import { setObjectGeometry as persistGeometry } from '../persist';
  import { llog } from '../log';
  import {
    lineGeometryForAngle as resolvedLineGeometryForAngle,
    lineLength as resolvedLineLength,
    normalizeAngle,
    parseProps,
  } from '../object-props';
  import { colorValue, numberValue, sharedValue } from './values';
  import { persistObjectPropsAndRefresh, reportPersistError, writeStyleMany } from './persist-ops';

  let {
    doc,
    layoutId = '',
    objects,
  }: { doc: EditorDoc; layoutId?: string; objects: readonly Readonly<ObjectDoc>[] } = $props();

  let mFill = $derived(sharedValue(objects, (p) => colorValue(p.fill, '#f7f8fa')));
  let mStrokeWidth = $derived(sharedValue(objects, (p) => numberValue(p.strokeWidth, 1)));
  let mStroke = $derived(sharedValue(objects, (p) => colorValue(p.stroke, '#d3d8de')));

  // A line's angle control applies to a SINGLE selected line only (a multi
  // selection never shows it — matching size/direction across lines is undefined).
  let line = $derived(objects.length === 1 && objects[0].kind === 'line' ? objects[0] : null);
  let lineProps = $derived(parseProps(line?.props ?? ''));

  function setStyle(key: string, value: string | number | boolean): void {
    void writeStyleMany(doc, layoutId, objects.map((o) => o.id), key, value);
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
