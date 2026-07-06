<script lang="ts">
  // The Layout-mode INSPECTOR island (issue #62 follow-up) — the selection-aware
  // Format panel (header + Binding / Style / Text / Band sections + a pinned
  // delete), mounted into the right `#layout-inspector` node and SHARING the
  // canvas's EditorDoc store with the rail-tools island. Like the rail, it
  // reads/writes ONLY through the store + persist helpers; it never touches the
  // parity-checked canvas DOM. Styling follows the "modern Mac" design ref.
  import type { EditorDoc, ObjectDoc } from './doc.svelte';
  import {
    deleteObject as persistDeleteObject,
    deleteObjectGroup as persistDeleteObjectGroup,
    deletePart as persistDeletePart,
    createObjectGroup as persistCreateObjectGroup,
    movePart as persistMovePart,
    setObjectBinding as persistBinding,
    setObjectContent as persistContent,
    setObjectGeometry as persistGeometry,
    setObjectProps as persistProps,
    setObjectReadOnly as persistReadOnly,
    setObjectsZ as persistObjectsZ,
    setPartHeight as persistPartHeight,
    setPartKind as persistPartKind,
    setPartProps as persistPartProps,
  } from './persist';
  import { llog, lerror } from './log';
  import { formatValue } from './format';
  import Icon from './Icon.svelte';
  import FieldSelect from './FieldSelect.svelte';
  import {
    lineGeometryForAngle as resolvedLineGeometryForAngle,
    lineLength as resolvedLineLength,
    normalizeAngle,
    parseProps,
  } from './object-props';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  const PART_KINDS: { id: string; label: string }[] = [
    { id: 'header', label: 'Header' },
    { id: 'body', label: 'Body' },
    { id: 'footer', label: 'Footer' },
    { id: 'subsummary', label: 'Sub-summary' },
    { id: 'grandsummary', label: 'Grand summary' },
  ];
  const KIND_LABEL: Record<string, string> = {
    field: 'Field',
    text: 'Text',
    rect: 'Rectangle',
    ellipse: 'Ellipse',
    line: 'Line',
  };

  let busy = $state(false);

  // ── Selection-aware derived state ─────────────────────────────────────────

  let selectedIds = $derived([...doc.selection]);
  let hasMultipleSelection = $derived(selectedIds.length > 1);
  let selectedId = $derived(selectedIds[0] ?? null);
  let selected = $derived(selectedId === null ? undefined : doc.getObject(selectedId));
  let selectedProps = $derived(parseProps(selected?.props ?? ''));
  // Per-object capability predicates (kind-based) so single- and multi-select share
  // exactly one definition of "can this kind be filled / text-formatted".
  let canFillLine = $derived(!!selected && kindCanFillLine(selected.kind));
  let canTextFormat = $derived(!!selected && kindCanTextFormat(selected.kind));
  let selectedBindingFieldId = $derived(selected?.kind === 'field' ? fieldIdForBinding(selected.binding) : null);

  // ── Multi-select derived state (#82) ──────────────────────────────────────
  // A control appears only when EVERY selected object shares the capability (not
  // just the first one), and each control reports a mixed-value state when the
  // selected objects disagree. Writes fan out to all of them as one undo step.
  let selectedObjects = $derived(
    selectedIds.map((id) => doc.getObject(id)).filter((o): o is Readonly<ObjectDoc> => !!o),
  );
  let allCanFillLine = $derived(
    selectedObjects.length > 0 && selectedObjects.every((o) => kindCanFillLine(o.kind)),
  );
  let allCanTextFormat = $derived(
    selectedObjects.length > 0 && selectedObjects.every((o) => kindCanTextFormat(o.kind)),
  );
  let allText = $derived(selectedObjects.length > 0 && selectedObjects.every((o) => o.kind === 'text'));

  // Arrange gating (#83): align/z-order need ≥2 (the multi-panel is already ≥2);
  // distribute needs ≥3; resize-to-match needs ≥2 objects that aren't lines (a
  // line's w/h encode its direction, so matching size would distort it).
  let canDistribute = $derived(selectedIds.length >= 3);
  let resizableCount = $derived(selectedObjects.filter((o) => o.kind !== 'line').length);
  let canResizeMatch = $derived(resizableCount >= 2);
  let activeGroupId = $derived(doc.groupIdForSelection(selectedIds));
  let canGroup = $derived(selectedIds.length >= 2 && activeGroupId === null);
  let canUngroup = $derived(activeGroupId !== null);

  // Shared style/text attributes across the whole selection, each as {mixed, value}.
  let mFill = $derived(sharedValue((p) => colorValue(p.fill, '#f7f8fa')));
  let mStrokeWidth = $derived(sharedValue((p) => numberValue(p.strokeWidth, 1)));
  let mStroke = $derived(sharedValue((p) => colorValue(p.stroke, '#d3d8de')));
  let mFontSize = $derived(sharedValue((p) => numberValue(p.fontSize, 13)));
  let mBold = $derived(sharedValue((p) => boolValue(p.bold)));
  let mItalic = $derived(sharedValue((p) => boolValue(p.italic)));
  let mUnderline = $derived(sharedValue((p) => boolValue(p.underline)));
  let mAlign = $derived(sharedValue((p) => alignValue(p.align)));
  let mTextColor = $derived(sharedValue((p) => colorValue(p.textColor, '#1b1b1f')));
  let mTextBg = $derived(sharedValue((p) => colorValue(p.fill, '#ffffff')));

  // ── Value-format (#77 number/Boolean, #78 date/time) ──────────────────────
  // Contextual by the bound field's kind (resolved through the binding, #79/#76).
  // The controls write the `format` sub-bag of the object's props; the server owns
  // the actual Browse/canvas render (crates/server/src/format.rs), so the panel's
  // Sample uses the byte-compatible TS mirror in ./format.ts for live feedback.
  let selectedFieldKind = $derived(
    selected?.kind === 'field' ? (doc.fields.find((f) => f.id === selectedBindingFieldId)?.kind ?? null) : null,
  );
  let isNumberFormat = $derived(selectedFieldKind === 'number' || selectedFieldKind === 'bool');
  let isDateFormat = $derived(selectedFieldKind === 'date');
  let isTimeFormat = $derived(selectedFieldKind === 'time');
  let isTimestampFormat = $derived(selectedFieldKind === 'timestamp');
  let hasValueFormat = $derived(isNumberFormat || isDateFormat || isTimeFormat || isTimestampFormat);

  let formatBag = $derived(asBag(selectedProps.format));
  // Date/Time write to the format bag directly for pure Date/Time fields, or into
  // the `date`/`time` sub-bags of a Timestamp's format (crates/server/src/format.rs).
  let dateBag = $derived(isTimestampFormat ? asBag(formatBag.date) : formatBag);
  let timeBag = $derived(isTimestampFormat ? asBag(formatBag.time) : formatBag);
  let dateComponents = $derived(
    Array.isArray(dateBag.components) ? (dateBag.components as Record<string, unknown>[]) : [],
  );

  let numberMode = $derived(bagStr(formatBag, 'mode', 'general'));
  let dateMode = $derived(bagStr(dateBag, 'mode', 'asEntered'));
  let timeMode = $derived(bagStr(timeBag, 'mode', 'asEntered'));
  let timeHas24 = $derived(bagBool(timeBag, 'hours24', true));
  let hasNegativeColor = $derived(typeof formatBag.negativeColor === 'string');

  // A representative raw value per kind so the Sample exercises every control.
  let sampleRaw = $derived(
    isNumberFormat
      ? '-1234.567'
      : isDateFormat
        ? '2003-12-25'
        : isTimeFormat
          ? '13:05:09'
          : isTimestampFormat
            ? '2003-12-25T13:05:09'
            : '',
  );
  let sample = $derived(formatValue(sampleRaw, formatBag, selectedFieldKind ?? 'text'));
  let selectedPartId = $derived(doc.selectedPartId);
  let selectedPart = $derived(selectedPartId === null ? undefined : doc.getPart(selectedPartId));
  let selectedPartProps = $derived(parseProps(selectedPart?.props ?? ''));
  // A form offers header/body/footer only; summaries are List/Table (Issue 3). The
  // current kind stays listed so an existing band always shows its own value.
  let partKinds = $derived(
    PART_KINDS.filter(
      (p) =>
        doc.view !== 'form' ||
        (p.id !== 'subsummary' && p.id !== 'grandsummary') ||
        p.id === selectedPart?.kind,
    ),
  );
  // Summary bands (sub/grand) are reorderable between the header and footer (Issue 4).
  let sortedParts = $derived([...doc.parts].sort((a, b) => a.position - b.position || a.id - b.id));
  let selectedPartIdx = $derived(selectedPart ? sortedParts.findIndex((p) => p.id === selectedPart.id) : -1);
  let selectedPartIsSummary = $derived(
    !!selectedPart && (selectedPart.kind === 'subsummary' || selectedPart.kind === 'grandsummary'),
  );
  let canMovePartUp = $derived(
    selectedPartIsSummary && selectedPartIdx > 0 && sortedParts[selectedPartIdx - 1].kind !== 'header',
  );
  let canMovePartDown = $derived(
    selectedPartIsSummary &&
      selectedPartIdx >= 0 &&
      selectedPartIdx < sortedParts.length - 1 &&
      sortedParts[selectedPartIdx + 1].kind !== 'footer',
  );

  // Header title/subtitle (design: "Field" · "Text · Name").
  let headerTitle = $derived(
    hasMultipleSelection ? 'Multiple items selected' : selected ? (KIND_LABEL[selected.kind] ?? 'Object') : selectedPart ? 'Band' : 'Inspector',
  );
  let headerSub = $derived(
    hasMultipleSelection
      ? `${selectedIds.length} objects`
      : selected
      ? selected.kind === 'field'
        ? doc.fields.find((f) => f.id === selectedBindingFieldId)?.name || selected.binding || ''
        : selected.kind === 'text'
          ? 'Label'
          : ''
      : selectedPart
        ? partKindLabel(selectedPart.kind)
        : '',
  );
  let deleteLabel = $derived(
    selectedIds.length > 1
      ? 'Delete objects'
      : selected?.kind === 'field'
        ? 'Delete Field'
        : selected
          ? `Delete ${KIND_LABEL[selected.kind] ?? 'Object'}`
          : '',
  );

  function partKindLabel(kind: string): string {
    return PART_KINDS.find((p) => p.id === kind)?.label ?? kind;
  }

  function fieldIdForBinding(binding: string): number | null {
    const fieldName = binding.split('.').at(-1)?.toLowerCase() ?? '';
    const found = doc.fields.find((f) => f.name.toLowerCase() === fieldName);
    return found?.id ?? null;
  }

  // Kind-based capability predicates — the single source both the single-object
  // (`canFillLine`/`canTextFormat`) and the whole-selection (`allCan*`) gates use.
  function kindCanFillLine(kind: string): boolean {
    return kind === 'field' || kind === 'rect' || kind === 'ellipse' || kind === 'line';
  }
  function kindCanTextFormat(kind: string): boolean {
    return kind === 'field' || kind === 'text';
  }

  /** Resolve one attribute across the whole selection into `{ mixed, value }`:
   * `mixed` is true when the selected objects disagree, and `value` is the first
   * object's resolved value (the control's shown value when not mixed). Reads each
   * object's props bag through the same coercers as the single-object controls. */
  function sharedValue<T>(resolve: (props: Record<string, unknown>) => T): { mixed: boolean; value: T } {
    const vals = selectedObjects.map((o) => resolve(parseProps(o.props)));
    const mixed = vals.some((v) => v !== vals[0]);
    return { mixed, value: vals[0] };
  }

  function colorValue(v: unknown, fallback: string): string {
    return typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v) ? v : fallback;
  }
  function numberValue(v: unknown, fallback: number): number {
    return typeof v === 'number' && Number.isFinite(v) ? v : fallback;
  }
  function boolValue(v: unknown): boolean {
    return v === true;
  }
  function asBag(v: unknown): Record<string, unknown> {
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
  }
  function bagStr(b: Record<string, unknown>, key: string, fallback: string): string {
    return typeof b[key] === 'string' ? (b[key] as string) : fallback;
  }
  function bagBool(b: Record<string, unknown>, key: string, fallback: boolean): boolean {
    return typeof b[key] === 'boolean' ? (b[key] as boolean) : fallback;
  }
  function bagNum(b: Record<string, unknown>, key: string, fallback: number): number {
    return typeof b[key] === 'number' && Number.isFinite(b[key] as number) ? (b[key] as number) : fallback;
  }
  function defaultDateComponent(type: string): Record<string, unknown> {
    switch (type) {
      case 'dayOfWeek':
        return { type, style: 'long', leading: '' };
      case 'month':
        return { type, style: 'number', leadingZero: true, leading: '' };
      case 'day':
        return { type, leadingZero: false, leading: '' };
      case 'year':
        return { type, style: 'full', leading: '' };
      default:
        return { type };
    }
  }
  const DATE_COMPONENT_LABEL: Record<string, string> = {
    dayOfWeek: 'Day of week',
    month: 'Month',
    day: 'Day',
    year: 'Year',
  };
  function alignValue(v: unknown): string {
    return typeof v === 'string' && ['left', 'center', 'right'].includes(v) ? v : 'left';
  }
  function lineLength(): number {
    if (!selected || selected.kind !== 'line') return 1;
    return resolvedLineLength(selected, selectedProps);
  }
  function lineGeometryForAngle(angle: number): { x: number; y: number; w: number; h: number } | null {
    if (!selected || selected.kind !== 'line') return null;
    return resolvedLineGeometryForAngle(selected, angle, lineLength());
  }

  function reportPersistError(label: string, e: unknown): void {
    lerror('persist', `${label} failed`, e);
    doc.setError(e instanceof Error ? e.message : String(e));
  }

  async function persistObjectPropsAndRefresh(id: number, props: Record<string, unknown>, label: string): Promise<void> {
    try {
      const styles = await persistProps(layoutId, id, props);
      doc.setObjectStyles(id, styles);
    } catch (e) {
      reportPersistError(label, e);
    }
  }

  function isSingletonPartKind(kind: string): boolean {
    return kind === 'header' || kind === 'body' || kind === 'footer';
  }

  function canSetSelectedPartKind(kind: string): boolean {
    if (!selectedPart) return false;
    if (selectedPart.kind === kind) return true;
    // A form allows only header/body/footer — summary bands are List/Table (Issue 3).
    if (doc.view === 'form' && (kind === 'subsummary' || kind === 'grandsummary')) return false;
    if (selectedPart.kind === 'body') return false;
    // Header/footer are structural anchors (top/bottom) — they can't become summaries,
    // which would strand a summary above the header or below the footer (mirrors move_part).
    if (
      (selectedPart.kind === 'header' || selectedPart.kind === 'footer') &&
      (kind === 'subsummary' || kind === 'grandsummary')
    )
      return false;
    if (isSingletonPartKind(kind) && doc.parts.some((p) => p.id !== selectedPart.id && p.kind === kind)) return false;
    if (kind === 'grandsummary') {
      const body = doc.parts.find((p) => p.kind === 'body');
      if (!body) return false;
      const wantsTrailing = selectedPart.position > body.position;
      return !doc.parts.some(
        (p) => p.id !== selectedPart.id && p.kind === 'grandsummary' && (p.position > body.position) === wantsTrailing,
      );
    }
    return true;
  }

  // ── Object / Style / Text handlers ────────────────────────────────────────

  async function setStyle(key: string, value: string | number | boolean): Promise<void> {
    if (selectedId === null) return;
    const next = { ...selectedProps, [key]: value };
    llog('persist', 'inspector: set style', { id: selectedId, key, value });
    // Optimistic + undoable document change; the canvas's shapeStyle then refreshes
    // from the server's single-source derivation.
    doc.setObjectProps(selectedId, JSON.stringify(next));
    doc.mark();
    await persistObjectPropsAndRefresh(selectedId, next, 'set style');
  }

  /** Multi-select generalization of `setStyle` (#82): write one style/text key to
   * EVERY selected object as a SINGLE undo step. Each object's own props bag is
   * merged optimistically (unchanged objects produce no diff), the whole batch is
   * sealed with one `doc.mark()`, then persisted in parallel so each object's
   * server-derived style refreshes. Callers gate the control by shared capability,
   * so the key already applies to all selected objects. */
  async function setStyleMany(key: string, value: string | number | boolean): Promise<void> {
    const ids = selectedIds;
    if (ids.length === 0) return;
    llog('persist', 'inspector: set style (many)', { ids, key, value });
    // Optimistic + undoable: apply to each object, accumulating into the open group.
    const nexts = new Map<number, Record<string, unknown>>();
    for (const id of ids) {
      const o = doc.getObject(id);
      if (!o) continue;
      const next = { ...parseProps(o.props), [key]: value };
      nexts.set(id, next);
      doc.setObjectProps(id, JSON.stringify(next));
    }
    doc.mark(); // one atomic undo step for the whole batch
    try {
      await Promise.all(
        [...nexts].map(async ([id, next]) => {
          const styles = await persistProps(layoutId, id, next);
          doc.setObjectStyles(id, styles);
        }),
      );
    } catch (e) {
      reportPersistError('set style (many)', e);
    }
  }

  // ── Arrange: align / distribute / resize-to-match / z-order (#83) ─────────
  // All align/distribute/resize edits are pure per-object geometry writes,
  // batched into a SINGLE undo step (mirrors setStyleMany over the geometry path).
  // z-order rewrites the `z` prop instead; it reaches the DOM straight from the
  // document `z` (Band.svelte), so no server style refresh is needed either way.

  type Geom = { x: number; y: number; w: number; h: number };

  /** Apply a batch of absolute geometries as one undo step, then persist each in
   * parallel (per-object endpoint, as #83 specifies). No-op writes are skipped by
   * the store's diff, and only changed objects are passed in by the callers. */
  async function applyGeometryMany(geoms: Map<number, Geom>): Promise<void> {
    if (geoms.size === 0) return;
    llog('persist', 'inspector: arrange geometry', { ids: [...geoms.keys()] });
    for (const [id, g] of geoms) doc.setObjectGeometry(id, g);
    doc.mark(); // one atomic undo step for the whole align/distribute action
    try {
      await Promise.all([...geoms].map(([id, g]) => persistGeometry(layoutId, id, g)));
    } catch (e) {
      reportPersistError('arrange geometry', e);
    }
  }

  /** The union bounding box of the current selection (the v1 reference frame). */
  function selectionBounds(): { minX: number; minY: number; maxX: number; maxY: number; cx: number; cy: number } {
    const os = selectedObjects;
    const minX = Math.min(...os.map((o) => o.x));
    const minY = Math.min(...os.map((o) => o.y));
    const maxX = Math.max(...os.map((o) => o.x + o.w));
    const maxY = Math.max(...os.map((o) => o.y + o.h));
    return { minX, minY, maxX, maxY, cx: (minX + maxX) / 2, cy: (minY + maxY) / 2 };
  }

  type AlignEdge = 'left' | 'hcenter' | 'right' | 'top' | 'vmiddle' | 'bottom';

  /** Align every selected object to the selection bounding box. Only x/y move —
   * w/h (and a line's angle/length) are untouched, so lines never distort. */
  function align(edge: AlignEdge): void {
    if (selectedObjects.length < 2) return;
    const b = selectionBounds();
    const geoms = new Map<number, Geom>();
    for (const o of selectedObjects) {
      let x = o.x;
      let y = o.y;
      switch (edge) {
        case 'left': x = b.minX; break;
        case 'right': x = b.maxX - o.w; break;
        case 'hcenter': x = Math.round(b.cx - o.w / 2); break;
        case 'top': y = b.minY; break;
        case 'bottom': y = b.maxY - o.h; break;
        case 'vmiddle': y = Math.round(b.cy - o.h / 2); break;
      }
      x = Math.max(0, x);
      y = Math.max(0, y);
      if (x !== o.x || y !== o.y) geoms.set(o.id, { x, y, w: o.w, h: o.h });
    }
    void applyGeometryMany(geoms);
  }

  /** Distribute with equal gaps between adjacent edges along one axis (#83, locked
   * decision). Outermost objects stay put; interior objects move so the empty space
   * between neighbours is equal. Needs ≥3 objects. */
  function distribute(axis: 'h' | 'v'): void {
    const os = selectedObjects.slice();
    if (os.length < 3) return;
    const geoms = new Map<number, Geom>();
    if (axis === 'h') {
      const sorted = os.sort((a, b) => a.x - b.x || a.id - b.id);
      const left = sorted[0].x;
      const right = sorted[sorted.length - 1].x + sorted[sorted.length - 1].w;
      const sumW = sorted.reduce((s, o) => s + o.w, 0);
      const gap = (right - left - sumW) / (sorted.length - 1);
      let cursor = left;
      for (const o of sorted) {
        const nx = Math.max(0, Math.round(cursor));
        if (nx !== o.x) geoms.set(o.id, { x: nx, y: o.y, w: o.w, h: o.h });
        cursor += o.w + gap;
      }
    } else {
      const sorted = os.sort((a, b) => a.y - b.y || a.id - b.id);
      const top = sorted[0].y;
      const bottom = sorted[sorted.length - 1].y + sorted[sorted.length - 1].h;
      const sumH = sorted.reduce((s, o) => s + o.h, 0);
      const gap = (bottom - top - sumH) / (sorted.length - 1);
      let cursor = top;
      for (const o of sorted) {
        const ny = Math.max(0, Math.round(cursor));
        if (ny !== o.y) geoms.set(o.id, { x: o.x, y: ny, w: o.w, h: o.h });
        cursor += o.h + gap;
      }
    }
    void applyGeometryMany(geoms);
  }

  /** Resize selected objects to the largest width/height/both among them. Lines are
   * excluded (their w/h encode direction); needs ≥2 non-line objects. */
  function resizeMatch(dim: 'w' | 'h' | 'both'): void {
    const targets = selectedObjects.filter((o) => o.kind !== 'line');
    if (targets.length < 2) return;
    const w = Math.max(...targets.map((o) => o.w));
    const h = Math.max(...targets.map((o) => o.h));
    const geoms = new Map<number, Geom>();
    for (const o of targets) {
      const nw = dim === 'w' || dim === 'both' ? w : o.w;
      const nh = dim === 'h' || dim === 'both' ? h : o.h;
      if (nw !== o.w || nh !== o.h) geoms.set(o.id, { x: o.x, y: o.y, w: nw, h: nh });
    }
    void applyGeometryMany(geoms);
  }

  type ZCmd = 'front' | 'back' | 'forward' | 'backward';

  /** Reorder a part's object ids for one z-command, preserving the selection's own
   * relative order when it moves as a block. `ids` is back→front; the result is the
   * new back→front order (index becomes the densified `z`). */
  function reorderZ(ids: number[], sel: Set<number>, cmd: ZCmd): number[] {
    const isSel = (id: number): boolean => sel.has(id);
    if (cmd === 'front') return [...ids.filter((id) => !isSel(id)), ...ids.filter((id) => isSel(id))];
    if (cmd === 'back') return [...ids.filter((id) => isSel(id)), ...ids.filter((id) => !isSel(id))];
    const a = ids.slice();
    if (cmd === 'forward') {
      // Shift each selected one step toward the front (higher index), as a block.
      for (let i = a.length - 2; i >= 0; i--) if (isSel(a[i]) && !isSel(a[i + 1])) [a[i], a[i + 1]] = [a[i + 1], a[i]];
    } else {
      for (let i = 1; i < a.length; i++) if (isSel(a[i]) && !isSel(a[i - 1])) [a[i], a[i - 1]] = [a[i - 1], a[i]];
    }
    return a;
  }

  /** Rewrite the stacking order of every part that holds a selected object, then
   * persist the changed `z` values as ONE undo step. z-order is per-part (paint
   * order is `(z, id)` within a band), so a selection spanning bands reorders each
   * independently. */
  async function zorder(cmd: ZCmd): Promise<void> {
    if (selectedIds.length === 0) return;
    const sel = new Set(selectedIds);
    const zmap = new Map<number, number>();
    for (const part of doc.renderModel.parts) {
      const ids = part.objects.map((o) => o.id); // already back→front by (z, id)
      if (!ids.some((id) => sel.has(id))) continue;
      reorderZ(ids, sel, cmd).forEach((id, i) => zmap.set(id, i));
    }
    // Densifying z touches non-selected objects too; keep only real changes.
    const changed = [...zmap].filter(([id, z]) => {
      const o = doc.getObject(id);
      return o !== undefined && o.z !== z;
    });
    if (changed.length === 0) return;
    llog('persist', 'inspector: z-order', { cmd, changed });
    for (const [id, z] of changed) doc.setProp(id, 'z', z);
    doc.mark(); // one atomic undo step for the whole restack
    try {
      await persistObjectsZ(layoutId, changed.map(([id, z]) => ({ id, z })));
    } catch (e) {
      reportPersistError('z-order', e);
    }
  }

  async function groupSelectedObjects(): Promise<void> {
    if (!canGroup || busy) return;
    busy = true;
    llog('persist', 'inspector: group objects', { ids: selectedIds });
    try {
      const group = await persistCreateObjectGroup(layoutId, selectedIds);
      doc.setGroup(group);
    } catch (e) {
      reportPersistError('group objects', e);
    } finally {
      busy = false;
    }
  }

  async function ungroupSelectedObjects(): Promise<void> {
    if (activeGroupId === null || busy) return;
    const groupId = activeGroupId;
    busy = true;
    llog('persist', 'inspector: ungroup objects', { groupId });
    try {
      await persistDeleteObjectGroup(layoutId, groupId);
      doc.removeGroup(groupId);
    } catch (e) {
      reportPersistError('ungroup objects', e);
    } finally {
      busy = false;
    }
  }

  async function setLineAngle(value: number): Promise<void> {
    if (selectedId === null || selected?.kind !== 'line') return;
    const angle = normalizeAngle(value);
    const length = lineLength();
    const geom = lineGeometryForAngle(angle);
    if (!geom) return;
    const next = { ...selectedProps, angle, length };
    llog('persist', 'inspector: set line angle', { id: selectedId, angle, length, geom });
    doc.setObjectGeometry(selectedId, geom);
    doc.setObjectProps(selectedId, JSON.stringify(next));
    doc.mark();
    try {
      await persistGeometry(layoutId, selectedId, geom);
      await persistObjectPropsAndRefresh(selectedId, next, 'set line angle');
    } catch (e) {
      reportPersistError('set line angle', e);
    }
  }

  async function setSelectedBinding(nextFieldId: number): Promise<void> {
    if (selectedId === null || selected?.kind !== 'field' || !Number.isFinite(nextFieldId)) return;
    llog('persist', 'inspector: set field binding', { id: selectedId, fieldId: nextFieldId });
    try {
      const view = await persistBinding(layoutId, selectedId, nextFieldId, doc.rec);
      doc.setProp(selectedId, 'binding', view.binding);
      doc.refreshResolved(view);
      doc.mark();
    } catch (e) {
      reportPersistError('set field binding', e);
    }
  }

  async function setSelectedContent(content: string): Promise<void> {
    if (selectedId === null || selected?.kind !== 'text') return;
    llog('persist', 'inspector: set text content', { id: selectedId });
    doc.setProp(selectedId, 'content', content);
    doc.mark();
    try {
      const view = await persistContent(layoutId, selectedId, content);
      doc.setProp(selectedId, 'content', view.content);
    } catch (e) {
      reportPersistError('set text content', e);
    }
  }

  async function setSelectedReadOnly(readOnly: boolean): Promise<void> {
    if (selectedId === null) return;
    llog('persist', 'inspector: set read-only', { id: selectedId, readOnly });
    doc.setProp(selectedId, 'readOnly', readOnly);
    doc.mark();
    try {
      const view = await persistReadOnly(layoutId, selectedId, readOnly, doc.rec);
      doc.setProp(selectedId, 'readOnly', view.readOnly);
      doc.refreshResolved(view);
    } catch (e) {
      reportPersistError('set read-only', e);
    }
  }

  async function deleteSelectedObjects(): Promise<void> {
    const ids = selectedIds;
    if (ids.length === 0 || busy) return;
    busy = true;
    llog('persist', 'inspector: delete object(s)', { ids });
    try {
      await Promise.all(ids.map((id) => persistDeleteObject(layoutId, id)));
      for (const id of ids) doc.removeObject(id);
      doc.mark();
    } catch (e) {
      reportPersistError('delete object', e);
    } finally {
      busy = false;
    }
  }

  // ── Value-format handlers ─────────────────────────────────────────────────
  // All writes merge into the object's `format` bag and persist through the same
  // doc-store + persistProps path as style/text edits, so they're undoable and the
  // server re-derives the object's style. `undefined` values are dropped by
  // JSON.stringify, which is how an optional key (e.g. negativeColor) is removed.

  async function commitFormat(next: Record<string, unknown>): Promise<void> {
    if (selectedId === null) return;
    const merged = { ...selectedProps, format: next };
    llog('persist', 'inspector: set value format', { id: selectedId, format: next });
    doc.setObjectProps(selectedId, JSON.stringify(merged));
    doc.mark();
    await persistObjectPropsAndRefresh(selectedId, merged, 'set value format');
  }
  // Number/Boolean write the format bag directly.
  function patchNumber(patch: Record<string, unknown>): void {
    void commitFormat({ ...formatBag, ...patch });
  }
  // Date/Time target the bag directly (pure field) or the timestamp sub-bag.
  function patchDate(patch: Record<string, unknown>): void {
    if (isTimestampFormat) void commitFormat({ ...formatBag, date: { ...dateBag, ...patch } });
    else void commitFormat({ ...formatBag, ...patch });
  }
  function patchTime(patch: Record<string, unknown>): void {
    if (isTimestampFormat) void commitFormat({ ...formatBag, time: { ...timeBag, ...patch } });
    else void commitFormat({ ...formatBag, ...patch });
  }
  function patchTimestamp(patch: Record<string, unknown>): void {
    void commitFormat({ ...formatBag, ...patch });
  }
  function setDateComponents(comps: Record<string, unknown>[]): void {
    patchDate({ components: comps });
  }
  function addDateComponent(type: string): void {
    setDateComponents([...dateComponents, defaultDateComponent(type)]);
  }
  function updateDateComponent(i: number, patch: Record<string, unknown>): void {
    setDateComponents(dateComponents.map((c, idx) => (idx === i ? { ...c, ...patch } : c)));
  }
  function removeDateComponent(i: number): void {
    setDateComponents(dateComponents.filter((_, idx) => idx !== i));
  }
  function moveDateComponent(i: number, up: boolean): void {
    const j = up ? i - 1 : i + 1;
    if (j < 0 || j >= dateComponents.length) return;
    const next = [...dateComponents];
    [next[i], next[j]] = [next[j], next[i]];
    setDateComponents(next);
  }

  // ── Band inspector ──────────────────────────────────────────────────────

  async function setSelectedPartKind(kind: string): Promise<void> {
    if (!selectedPart || !canSetSelectedPartKind(kind)) return;
    const id = selectedPart.id;
    llog('persist', 'inspector: set band kind', { id, kind });
    doc.setPartKind(id, kind);
    doc.mark();
    try {
      await persistPartKind(layoutId, id, kind);
    } catch (e) {
      reportPersistError('set band kind', e);
    }
  }

  async function setSelectedPartFill(value: string): Promise<void> {
    if (!selectedPart) return;
    const id = selectedPart.id;
    const next = { ...selectedPartProps, fill: value };
    llog('persist', 'inspector: set band fill', { id, value });
    // Optimistic + undoable document change; the band's partStyle then refreshes
    // from the server's single-source derivation (mirrors object setStyle).
    doc.setPartProps(id, JSON.stringify(next));
    doc.mark();
    try {
      const view = await persistPartProps(layoutId, id, next);
      doc.setPartStyle(id, view.partStyle);
    } catch (e) {
      reportPersistError('set band fill', e);
    }
  }

  async function setSelectedPartHeight(height: number): Promise<void> {
    if (!selectedPart) return;
    const id = selectedPart.id;
    const next = Math.max(doc.minPartHeight(id), Math.round(height || 1));
    llog('persist', 'inspector: set band height', { id, height: next });
    doc.setPartHeight(id, next);
    doc.mark();
    try {
      await persistPartHeight(layoutId, id, next);
    } catch (e) {
      reportPersistError('set band height', e);
    }
  }

  async function moveSelectedPart(up: boolean): Promise<void> {
    if (!selectedPart || busy) return;
    if (up ? !canMovePartUp : !canMovePartDown) return;
    const id = selectedPart.id;
    busy = true;
    llog('persist', 'inspector: move band', { id, up });
    try {
      const positions = await persistMovePart(layoutId, id, up);
      doc.applyPartPositions(positions);
    } catch (e) {
      reportPersistError('move band', e);
    } finally {
      busy = false;
    }
  }

  async function deleteSelectedPart(): Promise<void> {
    if (!selectedPart || selectedPart.kind === 'body' || busy) return;
    const id = selectedPart.id;
    busy = true;
    llog('persist', 'inspector: delete band', { id });
    try {
      await persistDeletePart(layoutId, id);
      doc.removePart(id);
    } catch (e) {
      reportPersistError('delete band', e);
    } finally {
      busy = false;
    }
  }
</script>

<header class="insp-head">
  <span class="insp-title">{headerTitle}</span>
  {#if headerSub}<span class="insp-sub">{headerSub}</span>{/if}
</header>

<div class="insp-body">
  {#if hasMultipleSelection}
    <section class="insp-sec">
      <span class="side-label">Arrange</span>
      <div class="fmt-sub">Group</div>
      <div class="group-row">
        <button type="button" class="grp-btn" title="Group selected objects" disabled={!canGroup || busy} onclick={groupSelectedObjects}>Group</button>
        <button type="button" class="grp-btn" title="Ungroup selected group" disabled={!canUngroup || busy} onclick={ungroupSelectedObjects}>Ungroup</button>
      </div>
      <div class="fmt-sub">Align</div>
      <div class="arr-grid">
        <button type="button" class="arr-btn" title="Align left edges" onclick={() => align('left')}><Icon name="obj-align-left" /></button>
        <button type="button" class="arr-btn" title="Align horizontal centers" onclick={() => align('hcenter')}><Icon name="obj-align-hcenter" /></button>
        <button type="button" class="arr-btn" title="Align right edges" onclick={() => align('right')}><Icon name="obj-align-right" /></button>
        <button type="button" class="arr-btn" title="Align top edges" onclick={() => align('top')}><Icon name="obj-align-top" /></button>
        <button type="button" class="arr-btn" title="Align vertical middles" onclick={() => align('vmiddle')}><Icon name="obj-align-vmiddle" /></button>
        <button type="button" class="arr-btn" title="Align bottom edges" onclick={() => align('bottom')}><Icon name="obj-align-bottom" /></button>
      </div>
      <div class="fmt-sub">Distribute &amp; resize</div>
      <div class="arr-grid">
        <button type="button" class="arr-btn" title="Distribute horizontally (equal gaps)" disabled={!canDistribute} onclick={() => distribute('h')}><Icon name="obj-distribute-h" /></button>
        <button type="button" class="arr-btn" title="Distribute vertically (equal gaps)" disabled={!canDistribute} onclick={() => distribute('v')}><Icon name="obj-distribute-v" /></button>
        <button type="button" class="arr-btn" title="Match width (widest)" disabled={!canResizeMatch} onclick={() => resizeMatch('w')}><Icon name="obj-same-width" /></button>
        <button type="button" class="arr-btn" title="Match height (tallest)" disabled={!canResizeMatch} onclick={() => resizeMatch('h')}><Icon name="obj-same-height" /></button>
      </div>
      <div class="fmt-sub">Order</div>
      <div class="arr-grid">
        <button type="button" class="arr-btn" title="Bring to front" onclick={() => zorder('front')}><Icon name="z-front" /></button>
        <button type="button" class="arr-btn" title="Bring forward" onclick={() => zorder('forward')}><Icon name="z-forward" /></button>
        <button type="button" class="arr-btn" title="Send backward" onclick={() => zorder('backward')}><Icon name="z-backward" /></button>
        <button type="button" class="arr-btn" title="Send to back" onclick={() => zorder('back')}><Icon name="z-back" /></button>
      </div>
    </section>
    {#if allCanFillLine || allCanTextFormat}
      <div class="insp-div"></div>
      {#if allCanFillLine}
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
                onchange={(e) => setStyleMany('fill', e.currentTarget.value)}
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
                onchange={(e) => setStyleMany('strokeWidth', Number(e.currentTarget.value))}
              />
              {#if mStroke.mixed}<span class="mixed-tag">Mixed</span>{/if}
              <input
                class="swatch"
                type="color"
                value={mStroke.value}
                onchange={(e) => setStyleMany('stroke', e.currentTarget.value)}
              />
            </div>
          </div>
        </section>
      {/if}

      {#if allCanTextFormat}
        {#if allCanFillLine}<div class="insp-div"></div>{/if}
        <section class="insp-sec">
          <span class="side-label">Text</span>
          <div class="insp-row">
            <span>Size</span>
            <input
              class="ctl-num"
              type="number"
              min="6"
              max="96"
              placeholder={mFontSize.mixed ? 'Mixed' : ''}
              value={mFontSize.mixed ? '' : mFontSize.value}
              onchange={(e) => setStyleMany('fontSize', Number(e.currentTarget.value))}
            />
          </div>
          <div class="seg-row">
            <div class="seg">
              <button
                type="button"
                class="seg-btn"
                class:active={!mBold.mixed && mBold.value}
                class:mixed={mBold.mixed}
                title="Bold"
                onclick={() => setStyleMany('bold', mBold.mixed ? true : !mBold.value)}
              ><b>B</b></button>
              <button
                type="button"
                class="seg-btn"
                class:active={!mItalic.mixed && mItalic.value}
                class:mixed={mItalic.mixed}
                title="Italic"
                onclick={() => setStyleMany('italic', mItalic.mixed ? true : !mItalic.value)}
              ><i>I</i></button>
              <button
                type="button"
                class="seg-btn"
                class:active={!mUnderline.mixed && mUnderline.value}
                class:mixed={mUnderline.mixed}
                title="Underline"
                onclick={() => setStyleMany('underline', mUnderline.mixed ? true : !mUnderline.value)}
              ><u>U</u></button>
            </div>
            <div class="seg">
              {#each ['left', 'center', 'right'] as a}
                <button
                  type="button"
                  class="seg-btn"
                  class:active={!mAlign.mixed && mAlign.value === a}
                  title={`Align ${a}`}
                  onclick={() => setStyleMany('align', a)}
                ><Icon name={`align-${a}`} /></button>
              {/each}
            </div>
          </div>
          <div class="insp-row">
            <span>Color</span>
            <div class="insp-ctls">
              {#if mTextColor.mixed}<span class="mixed-tag">Mixed</span>{/if}
              <input
                class="swatch"
                type="color"
                value={mTextColor.value}
                onchange={(e) => setStyleMany('textColor', e.currentTarget.value)}
              />
            </div>
          </div>
          {#if allText}
            <!-- Background fill is a text-object attribute (Issue 7); shown only when
                 every selected object is a text label. -->
            <div class="insp-row">
              <span>Background</span>
              <div class="insp-ctls">
                {#if mTextBg.mixed}<span class="mixed-tag">Mixed</span>{/if}
                <input
                  class="swatch"
                  type="color"
                  value={mTextBg.value}
                  onchange={(e) => setStyleMany('fill', e.currentTarget.value)}
                />
              </div>
            </div>
          {/if}
        </section>
      {/if}
    {/if}
  {:else if selected}
    <section class="insp-sec">
      <span class="side-label">Arrange</span>
      <div class="fmt-sub">Order</div>
      <div class="arr-grid">
        <button type="button" class="arr-btn" title="Bring to front" onclick={() => zorder('front')}><Icon name="z-front" /></button>
        <button type="button" class="arr-btn" title="Bring forward" onclick={() => zorder('forward')}><Icon name="z-forward" /></button>
        <button type="button" class="arr-btn" title="Send backward" onclick={() => zorder('backward')}><Icon name="z-backward" /></button>
        <button type="button" class="arr-btn" title="Send to back" onclick={() => zorder('back')}><Icon name="z-back" /></button>
      </div>
    </section>
    <div class="insp-div"></div>
    {#if selected.kind === 'field' || selected.kind === 'text'}
      <section class="insp-sec">
        <span class="side-label">{selected.kind === 'text' ? 'Text' : 'Binding'}</span>
        {#if selected.kind === 'field'}
          <FieldSelect
            fields={doc.fields}
            value={selectedBindingFieldId}
            placeholder="Unresolved"
            title="Bound field"
            onselect={(id) => setSelectedBinding(id)}
          />
          {#if selectedBindingFieldId === null && selected.binding}
            <span class="le-hint">{selected.binding}</span>
          {/if}
          <div class="insp-row">
            <span>Read-only</span>
            <label class="toggle">
              <input
                type="checkbox"
                checked={selected.readOnly}
                onchange={(e) => setSelectedReadOnly(e.currentTarget.checked)}
              />
              <span class="toggle-track"><span class="toggle-knob"></span></span>
            </label>
          </div>
        {:else}
          <input
            class="ctl-input"
            type="text"
            value={selected.content}
            onchange={(e) => setSelectedContent(e.currentTarget.value)}
          />
        {/if}
      </section>
    {/if}

    {#if canFillLine}
      <div class="insp-div"></div>
      <section class="insp-sec">
        <span class="side-label">Style</span>
        <div class="insp-row">
          <span>Fill</span>
          <input
            class="swatch"
            type="color"
            value={colorValue(selectedProps.fill, '#f7f8fa')}
            onchange={(e) => setStyle('fill', e.currentTarget.value)}
          />
        </div>
        <div class="insp-row">
          <span>Border</span>
          <div class="insp-ctls">
            <input
              class="ctl-num"
              type="number"
              min="0"
              max="12"
              value={numberValue(selectedProps.strokeWidth, 1)}
              onchange={(e) => setStyle('strokeWidth', Number(e.currentTarget.value))}
            />
            <input
              class="swatch"
              type="color"
              value={colorValue(selectedProps.stroke, '#d3d8de')}
              onchange={(e) => setStyle('stroke', e.currentTarget.value)}
            />
          </div>
        </div>
        {#if selected.kind === 'line'}
          <div class="insp-row">
            <span>Angle</span>
            <input
              class="ctl-num"
              type="number"
              min="0"
              max="359"
              step="1"
              value={numberValue(selectedProps.angle, 0)}
              onchange={(e) => setLineAngle(Number(e.currentTarget.value))}
            />
          </div>
        {/if}
      </section>
    {/if}

    {#if canTextFormat}
      <div class="insp-div"></div>
      <section class="insp-sec">
        <span class="side-label">Text</span>
        <div class="insp-row">
          <span>Size</span>
          <input
            class="ctl-num"
            type="number"
            min="6"
            max="96"
            value={numberValue(selectedProps.fontSize, 13)}
            onchange={(e) => setStyle('fontSize', Number(e.currentTarget.value))}
          />
        </div>
        <div class="seg-row">
          <div class="seg">
            <button
              type="button"
              class="seg-btn"
              class:active={boolValue(selectedProps.bold)}
              title="Bold"
              onclick={() => setStyle('bold', !boolValue(selectedProps.bold))}
            ><b>B</b></button>
            <button
              type="button"
              class="seg-btn"
              class:active={boolValue(selectedProps.italic)}
              title="Italic"
              onclick={() => setStyle('italic', !boolValue(selectedProps.italic))}
            ><i>I</i></button>
            <button
              type="button"
              class="seg-btn"
              class:active={boolValue(selectedProps.underline)}
              title="Underline"
              onclick={() => setStyle('underline', !boolValue(selectedProps.underline))}
            ><u>U</u></button>
          </div>
          <div class="seg">
            {#each ['left', 'center', 'right'] as a}
              <button
                type="button"
                class="seg-btn"
                class:active={alignValue(selectedProps.align) === a}
                title={`Align ${a}`}
                onclick={() => setStyle('align', a)}
              ><Icon name={`align-${a}`} /></button>
            {/each}
          </div>
        </div>
        <div class="insp-row">
          <span>Color</span>
          <input
            class="swatch"
            type="color"
            value={colorValue(selectedProps.textColor, '#1b1b1f')}
            onchange={(e) => setStyle('textColor', e.currentTarget.value)}
          />
        </div>
        {#if selected.kind === 'text'}
          <!-- Text objects have a background fill too (Issue 7); the server's
               object_style() renders `background:{fill}` for them. -->
          <div class="insp-row">
            <span>Background</span>
            <input
              class="swatch"
              type="color"
              value={colorValue(selectedProps.fill, '#ffffff')}
              onchange={(e) => setStyle('fill', e.currentTarget.value)}
            />
          </div>
        {/if}
      </section>
    {/if}

    {#if hasValueFormat}
      <div class="insp-div"></div>
      <section class="insp-sec">
        <span class="side-label">Value format</span>
        {#if isNumberFormat}
          {@render numberControls()}
        {/if}
        {#if isDateFormat}
          {@render dateControls()}
        {/if}
        {#if isTimeFormat}
          {@render timeControls()}
        {/if}
        {#if isTimestampFormat}
          <div class="fmt-sub">Date</div>
          {@render dateControls()}
          <div class="fmt-sub">Time</div>
          {@render timeControls()}
          <div class="insp-row">
            <span>Date/time gap</span>
            <input
              class="ctl-char"
              type="text"
              value={bagStr(formatBag, 'separator', ' ')}
              onchange={(e) => patchTimestamp({ separator: e.currentTarget.value })}
            />
          </div>
        {/if}
        <div class="fmt-sample">
          <span class="fmt-sample-label">Sample</span>
          <span class="fmt-sample-val" style={sample.color ? `color:${sample.color}` : ''}
            >{sample.text || '—'}</span
          >
        </div>
      </section>
    {/if}
  {:else if selectedPart}
    <section class="insp-sec">
      <span class="side-label">Band</span>
      <div class="insp-row">
        <span>Kind</span>
        <select
          class="ctl-select ctl-select-auto"
          value={selectedPart.kind}
          onchange={(e) => setSelectedPartKind(e.currentTarget.value)}
        >
          {#each partKinds as p (p.id)}
            <option value={p.id} disabled={!canSetSelectedPartKind(p.id)}>{p.label}</option>
          {/each}
        </select>
      </div>
      <div class="insp-row">
        <span>Height</span>
        <input
          class="ctl-num"
          type="number"
          min={doc.minPartHeight(selectedPart.id)}
          value={selectedPart.height}
          onchange={(e) => setSelectedPartHeight(Number(e.currentTarget.value))}
        />
      </div>
      <!-- Band background fill (Issue 7): the server's part_style() renders
           `background:{fill}` for the band, live on the canvas and in Browse. -->
      <div class="insp-row">
        <span>Background</span>
        <input
          class="swatch"
          type="color"
          value={colorValue(selectedPartProps.fill, '#ffffff')}
          onchange={(e) => setSelectedPartFill(e.currentTarget.value)}
        />
      </div>
      {#if selectedPartIsSummary}
        <!-- Summary bands reorder between the header and footer (Issue 4). -->
        <div class="insp-row">
          <span>Order</span>
          <div class="insp-ctls">
            <button
              type="button"
              class="ord-btn"
              title="Move band up"
              disabled={busy || !canMovePartUp}
              onclick={() => moveSelectedPart(true)}
            >↑</button>
            <button
              type="button"
              class="ord-btn"
              title="Move band down"
              disabled={busy || !canMovePartDown}
              onclick={() => moveSelectedPart(false)}
            >↓</button>
          </div>
        </div>
      {/if}
    </section>
  {:else}
    <p class="insp-empty">Select an object or band to edit it.</p>
  {/if}
</div>

{#if selectedIds.length > 0}
  <footer class="insp-foot">
    <button
      type="button"
      class="insp-delete"
      title={hasMultipleSelection ? 'Delete selected objects' : 'Delete selected object'}
      disabled={selectedIds.length === 0 || busy}
      onclick={deleteSelectedObjects}
    >{deleteLabel}</button>
  </footer>
{:else if selectedPart}
  <footer class="insp-foot">
    <button
      type="button"
      class="insp-delete"
      title="Delete selected band"
      disabled={busy || selectedPart.kind === 'body'}
      onclick={deleteSelectedPart}
    >Delete band</button>
  </footer>
{/if}

<!-- ── Value-format control snippets (#77/#78) ────────────────────────────────
     Reused for pure Number/Bool/Date/Time fields and, together, for a Timestamp
     (which formats a Date sub-bag + a Time sub-bag). The patch* helpers route each
     write to the right bag, so the same snippet serves both cases. -->
{#snippet numberControls()}
  <div class="insp-row">
    <span>Format</span>
    <select
      class="ctl-select ctl-select-auto"
      value={numberMode}
      onchange={(e) => patchNumber({ mode: e.currentTarget.value })}
    >
      <option value="general">General</option>
      <option value="asEntered">Leave as entered</option>
      <option value="boolean">Boolean</option>
      <option value="decimal">Decimal</option>
    </select>
  </div>
  {#if numberMode === 'boolean'}
    <div class="insp-row">
      <span>Non-zero as</span>
      <input
        class="ctl-input fmt-grow"
        type="text"
        placeholder="e.g. Yes"
        value={bagStr(formatBag, 'booleanNonZero', '')}
        onchange={(e) => patchNumber({ booleanNonZero: e.currentTarget.value })}
      />
    </div>
    <div class="insp-row">
      <span>Zero as</span>
      <input
        class="ctl-input fmt-grow"
        type="text"
        placeholder="e.g. No"
        value={bagStr(formatBag, 'booleanZero', '')}
        onchange={(e) => patchNumber({ booleanZero: e.currentTarget.value })}
      />
    </div>
  {:else if numberMode !== 'asEntered'}
    {#if numberMode === 'decimal'}
      <div class="insp-row">
        <span>Fixed decimals</span>
        <div class="insp-ctls">
          <input
            class="ctl-num"
            type="number"
            min="0"
            max="15"
            disabled={!bagBool(formatBag, 'fixedDecimals', false)}
            value={bagNum(formatBag, 'decimalDigits', 2)}
            onchange={(e) =>
              patchNumber({ decimalDigits: Math.min(Math.max(Math.round(Number(e.currentTarget.value) || 0), 0), 15) })}
          />
          <label class="toggle">
            <input
              type="checkbox"
              checked={bagBool(formatBag, 'fixedDecimals', false)}
              onchange={(e) => patchNumber({ fixedDecimals: e.currentTarget.checked })}
            />
            <span class="toggle-track"><span class="toggle-knob"></span></span>
          </label>
        </div>
      </div>
      <div class="insp-row">
        <span>Currency</span>
        <select
          class="ctl-select ctl-select-auto"
          value={bagStr(formatBag, 'currency', 'none')}
          onchange={(e) => patchNumber({ currency: e.currentTarget.value })}
        >
          <option value="none">None</option>
          <option value="leading">Leading</option>
          <option value="inside">Inside</option>
        </select>
      </div>
      {#if bagStr(formatBag, 'currency', 'none') !== 'none'}
        <div class="insp-row">
          <span>Symbol</span>
          <input
            class="ctl-char"
            type="text"
            placeholder="$"
            value={bagStr(formatBag, 'currencySymbol', '')}
            onchange={(e) => patchNumber({ currencySymbol: e.currentTarget.value })}
          />
        </div>
      {/if}
      <div class="insp-row">
        <span>Hide if zero</span>
        <label class="toggle">
          <input
            type="checkbox"
            checked={bagBool(formatBag, 'hideZero', false)}
            onchange={(e) => patchNumber({ hideZero: e.currentTarget.checked })}
          />
          <span class="toggle-track"><span class="toggle-knob"></span></span>
        </label>
      </div>
    {/if}
    <div class="insp-row">
      <span>Decimal separator</span>
      <input
        class="ctl-char"
        type="text"
        maxlength="1"
        value={bagStr(formatBag, 'decimalSeparator', '.')}
        onchange={(e) => patchNumber({ decimalSeparator: e.currentTarget.value || '.' })}
      />
    </div>
    <div class="insp-row">
      <span>Thousands separator</span>
      <select
        class="ctl-select ctl-select-auto"
        value={bagStr(formatBag, 'thousandsSeparator', '')}
        onchange={(e) => patchNumber({ thousandsSeparator: e.currentTarget.value })}
      >
        <option value="">None</option>
        <option value=",">Comma ,</option>
        <option value=".">Period .</option>
        <option value=" ">Space</option>
        <option value="'">Apostrophe '</option>
      </select>
    </div>
    <div class="insp-row">
      <span>Negatives</span>
      <select
        class="ctl-select ctl-select-auto"
        value={bagStr(formatBag, 'negativeStyle', 'minus')}
        onchange={(e) => patchNumber({ negativeStyle: e.currentTarget.value })}
      >
        <option value="minus">Minus −1234</option>
        <option value="parens">Parens (1234)</option>
      </select>
    </div>
    <div class="insp-row">
      <span>Negative color</span>
      <div class="insp-ctls">
        {#if hasNegativeColor}
          <input
            class="swatch"
            type="color"
            value={colorValue(formatBag.negativeColor, '#d70015')}
            onchange={(e) => patchNumber({ negativeColor: e.currentTarget.value })}
          />
        {/if}
        <label class="toggle">
          <input
            type="checkbox"
            checked={hasNegativeColor}
            onchange={(e) =>
              patchNumber({ negativeColor: e.currentTarget.checked ? colorValue(formatBag.negativeColor, '#d70015') : undefined })}
          />
          <span class="toggle-track"><span class="toggle-knob"></span></span>
        </label>
      </div>
    </div>
  {/if}
{/snippet}

{#snippet dateControls()}
  <div class="insp-row">
    <span>Date</span>
    <select
      class="ctl-select ctl-select-auto"
      value={dateMode}
      onchange={(e) => patchDate({ mode: e.currentTarget.value })}
    >
      <option value="asEntered">Leave as entered</option>
      <option value="predefined">Predefined</option>
      <option value="custom">Custom</option>
    </select>
  </div>
  {#if dateMode === 'predefined'}
    <div class="insp-row">
      <span>Style</span>
      <select
        class="ctl-select ctl-select-auto"
        value={bagStr(dateBag, 'predefined', 'mm/dd/yyyy')}
        onchange={(e) => patchDate({ predefined: e.currentTarget.value })}
      >
        <option value="mm/dd/yy">mm/dd/yy</option>
        <option value="mm/dd/yyyy">mm/dd/yyyy</option>
        <option value="dd/mm/yy">dd/mm/yy</option>
        <option value="dd/mm/yyyy">dd/mm/yyyy</option>
        <option value="yyyy-mm-dd">yyyy-mm-dd</option>
      </select>
    </div>
    <div class="insp-row">
      <span>Separator</span>
      <input
        class="ctl-char"
        type="text"
        maxlength="1"
        value={bagStr(dateBag, 'dateSeparator', bagStr(dateBag, 'predefined', 'mm/dd/yyyy').includes('-') ? '-' : '/')}
        onchange={(e) => patchDate({ dateSeparator: e.currentTarget.value })}
      />
    </div>
  {:else if dateMode === 'custom'}
    <div class="fmt-comps">
      {#each dateComponents as comp, i (i)}
        <div class="fmt-comp">
          <div class="fmt-comp-head">
            <span class="fmt-comp-name">{DATE_COMPONENT_LABEL[bagStr(comp, 'type', '')] ?? 'Part'}</span>
            <div class="insp-ctls">
              <button
                type="button"
                class="ord-btn"
                title="Move up"
                disabled={i === 0}
                onclick={() => moveDateComponent(i, true)}>↑</button
              >
              <button
                type="button"
                class="ord-btn"
                title="Move down"
                disabled={i === dateComponents.length - 1}
                onclick={() => moveDateComponent(i, false)}>↓</button
              >
              <button type="button" class="ord-btn" title="Remove" onclick={() => removeDateComponent(i)}>×</button>
            </div>
          </div>
          <div class="insp-row">
            <span>Leading</span>
            <input
              class="ctl-char"
              type="text"
              placeholder="sep"
              value={bagStr(comp, 'leading', '')}
              onchange={(e) => updateDateComponent(i, { leading: e.currentTarget.value })}
            />
          </div>
          {#if bagStr(comp, 'type', '') === 'dayOfWeek'}
            <div class="insp-row">
              <span>Style</span>
              <select
                class="ctl-select ctl-select-auto"
                value={bagStr(comp, 'style', 'long')}
                onchange={(e) => updateDateComponent(i, { style: e.currentTarget.value })}
              >
                <option value="long">Long</option>
                <option value="short">Short</option>
              </select>
            </div>
          {:else if bagStr(comp, 'type', '') === 'month'}
            <div class="insp-row">
              <span>Style</span>
              <select
                class="ctl-select ctl-select-auto"
                value={bagStr(comp, 'style', 'number')}
                onchange={(e) => updateDateComponent(i, { style: e.currentTarget.value })}
              >
                <option value="number">Number</option>
                <option value="short">Short</option>
                <option value="long">Long</option>
              </select>
            </div>
            {#if bagStr(comp, 'style', 'number') === 'number'}
              <div class="insp-row">
                <span>Leading zero</span>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={bagBool(comp, 'leadingZero', true)}
                    onchange={(e) => updateDateComponent(i, { leadingZero: e.currentTarget.checked })}
                  />
                  <span class="toggle-track"><span class="toggle-knob"></span></span>
                </label>
              </div>
            {/if}
          {:else if bagStr(comp, 'type', '') === 'day'}
            <div class="insp-row">
              <span>Leading zero</span>
              <label class="toggle">
                <input
                  type="checkbox"
                  checked={bagBool(comp, 'leadingZero', false)}
                  onchange={(e) => updateDateComponent(i, { leadingZero: e.currentTarget.checked })}
                />
                <span class="toggle-track"><span class="toggle-knob"></span></span>
              </label>
            </div>
          {:else if bagStr(comp, 'type', '') === 'year'}
            <div class="insp-row">
              <span>Style</span>
              <select
                class="ctl-select ctl-select-auto"
                value={bagStr(comp, 'style', 'full')}
                onchange={(e) => updateDateComponent(i, { style: e.currentTarget.value })}
              >
                <option value="full">Full 2003</option>
                <option value="short">Short 03</option>
              </select>
            </div>
          {/if}
        </div>
      {/each}
      <div class="fmt-add">
        {#each ['dayOfWeek', 'month', 'day', 'year'] as t (t)}
          <button type="button" class="fmt-add-btn" onclick={() => addDateComponent(t)}>+ {DATE_COMPONENT_LABEL[t]}</button>
        {/each}
      </div>
    </div>
  {/if}
{/snippet}

{#snippet timeControls()}
  <div class="insp-row">
    <span>Time</span>
    <select
      class="ctl-select ctl-select-auto"
      value={timeMode}
      onchange={(e) => patchTime({ mode: e.currentTarget.value })}
    >
      <option value="asEntered">Leave as entered</option>
      <option value="predefined">Predefined</option>
      <option value="custom">Custom</option>
    </select>
  </div>
  {#if timeMode === 'predefined'}
    <div class="insp-row">
      <span>Style</span>
      <select
        class="ctl-select ctl-select-auto"
        value={bagStr(timeBag, 'predefined', 'hh:mm:ss')}
        onchange={(e) => patchTime({ predefined: e.currentTarget.value })}
      >
        <option value="hh:mm:ss">hh:mm:ss</option>
        <option value="hh:mm">hh:mm</option>
      </select>
    </div>
  {/if}
  {#if timeMode === 'custom'}
    <div class="insp-row">
      <span>Show seconds</span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={bagBool(timeBag, 'showSeconds', true)}
          onchange={(e) => patchTime({ showSeconds: e.currentTarget.checked })}
        />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
  {/if}
  {#if timeMode !== 'asEntered'}
    <div class="insp-row">
      <span>24-hour</span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={timeHas24}
          onchange={(e) => patchTime({ hours24: e.currentTarget.checked })}
        />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
    <div class="insp-row">
      <span>Separator</span>
      <input
        class="ctl-char"
        type="text"
        maxlength="1"
        value={bagStr(timeBag, 'timeSeparator', ':')}
        onchange={(e) => patchTime({ timeSeparator: e.currentTarget.value || ':' })}
      />
    </div>
    <div class="insp-row">
      <span>Hours leading zero</span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={bagBool(timeBag, 'hoursLeadingZero', true)}
          onchange={(e) => patchTime({ hoursLeadingZero: e.currentTarget.checked })}
        />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
    <div class="insp-row">
      <span>Min/sec leading zero</span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={bagBool(timeBag, 'minutesSecondsLeadingZero', true)}
          onchange={(e) => patchTime({ minutesSecondsLeadingZero: e.currentTarget.checked })}
        />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
    {#if !timeHas24}
      <div class="insp-row">
        <span>AM label</span>
        <input
          class="ctl-char"
          type="text"
          value={bagStr(timeBag, 'amLabel', 'AM')}
          onchange={(e) => patchTime({ amLabel: e.currentTarget.value })}
        />
      </div>
      <div class="insp-row">
        <span>PM label</span>
        <input
          class="ctl-char"
          type="text"
          value={bagStr(timeBag, 'pmLabel', 'PM')}
          onchange={(e) => patchTime({ pmLabel: e.currentTarget.value })}
        />
      </div>
      <div class="insp-row">
        <span>AM/PM placement</span>
        <select
          class="ctl-select ctl-select-auto"
          value={bagStr(timeBag, 'amPmPlacement', 'after')}
          onchange={(e) => patchTime({ amPmPlacement: e.currentTarget.value })}
        >
          <option value="after">After</option>
          <option value="before">Before</option>
          <option value="none">None</option>
        </select>
      </div>
    {/if}
  {/if}
{/snippet}

<style>
  /* Format inspector — mirrors the design ref's right panel. Reuses the global
     `.side-label` and the shared --rm-* palette (defined on body). */
  .insp-head {
    padding: 16px 18px 12px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .insp-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--rm-text);
  }
  .insp-sub {
    min-width: 0;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--rm-text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .insp-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .insp-sec {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .insp-div {
    height: 0.5px;
    background: var(--rm-border);
  }
  .insp-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 13px;
    color: var(--rm-text);
  }
  .insp-ctls {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .insp-empty {
    margin: 0;
    font-size: 12px;
    color: var(--rm-text-dim);
  }
  /* Controls: the .ctl-* input/select recipe is shared vocabulary now — see
     ui/src/shared/controls.css (#132). */
  /* Band reorder buttons (Issue 4). */
  .ord-btn {
    width: 30px;
    height: 26px;
    padding: 0;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    font-size: 13px;
    line-height: 1;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .ord-btn:hover:not(:disabled) {
    background: #f0f0f2;
  }
  .ord-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  /* Arrange panel (#83): wrapping grids of icon buttons for align / distribute /
     resize-to-match / z-order. */
  .arr-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .arr-btn {
    width: 34px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .arr-btn:hover:not(:disabled) {
    background: #f0f0f2;
  }
  .arr-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .group-row {
    display: flex;
    gap: 8px;
  }
  .grp-btn {
    height: 28px;
    padding: 0 10px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    font-weight: 650;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .grp-btn:hover:not(:disabled) {
    background: #f0f0f2;
  }
  .grp-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  /* Color swatch — a 26px rounded chip. */
  .swatch {
    width: 26px;
    height: 26px;
    padding: 0;
    border: 1px solid var(--rm-border-strong);
    border-radius: 7px;
    background: var(--rm-control-bg);
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
  }
  .swatch::-webkit-color-swatch-wrapper {
    padding: 0;
  }
  .swatch::-webkit-color-swatch {
    border: 0;
    border-radius: 6px;
  }
  .swatch:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  /* iOS-style toggle. */
  .toggle {
    position: relative;
    display: inline-flex;
    cursor: pointer;
  }
  .toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .toggle-track {
    width: 36px;
    height: 21px;
    border-radius: 21px;
    background: var(--rm-segment-track);
    transition: background 0.15s ease;
  }
  .toggle-knob {
    position: absolute;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: #fff;
    top: 2px;
    left: 2px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: left 0.15s ease;
  }
  .toggle input:checked + .toggle-track {
    background: var(--rm-accent);
  }
  .toggle input:checked + .toggle-track .toggle-knob {
    left: 17px;
  }
  /* Segmented controls (B/I/U, L/C/R). */
  .seg-row {
    display: flex;
    gap: 10px;
  }
  .seg {
    flex: 1;
    display: inline-flex;
    background: var(--rm-segment-track);
    border-radius: 7px;
    padding: 2px;
  }
  .seg-btn {
    flex: 1;
    text-align: center;
    padding: 5px 0;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--rm-text-dim);
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    line-height: 1;
  }
  .seg-btn.active {
    background: var(--rm-segment-active-bg);
    color: var(--rm-text);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.14);
  }
  /* A mixed toggle across a multi-selection (#82): neither on nor off — a dashed
     outline signals the indeterminate state without claiming a value. */
  .seg-btn.mixed {
    outline: 1px dashed var(--rm-border-strong);
    outline-offset: -3px;
  }
  /* "Mixed" pill shown beside a control when the selection's values disagree (#82). */
  .mixed-tag {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--rm-text-dim);
    padding: 2px 7px;
    border-radius: 5px;
    background: var(--rm-segment-track);
  }
  /* Pinned delete footer. */
  .insp-foot {
    margin-top: auto;
    padding: 14px 18px;
    border-top: 0.5px solid var(--rm-border);
  }
  .insp-delete {
    width: 100%;
    text-align: center;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    color: var(--rm-danger);
    padding: 8px;
    border-radius: 8px;
    border: 0.5px solid var(--rm-border);
    background: var(--rm-control-bg);
    cursor: pointer;
  }
  .insp-delete:hover:not(:disabled) {
    background: #fff5f5;
  }
  .insp-delete:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .le-hint {
    font-size: 11px;
    color: var(--rm-text-dim);
  }
  /* Value format (#77/#78). A Timestamp sub-heading, the custom-date component
     cards, and the Sample preview (the .ctl-char input lives in shared/controls.css). */
  .fmt-grow {
    width: 150px;
  }
  .fmt-sub {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--rm-text-dim);
    margin-top: 4px;
  }
  .fmt-comps {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .fmt-comp {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 0.5px solid var(--rm-border);
    border-radius: 8px;
    background: var(--rm-control-bg);
  }
  .fmt-comp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .fmt-comp-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .fmt-add {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .fmt-add-btn {
    font: inherit;
    font-size: 12px;
    color: var(--rm-text);
    padding: 5px 9px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .fmt-add-btn:hover {
    background: #f0f0f2;
  }
  .fmt-sample {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 4px;
    padding: 10px 12px;
    border: 0.5px solid var(--rm-border);
    border-radius: 8px;
    background: var(--rm-segment-track);
  }
  .fmt-sample-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--rm-text-dim);
  }
  .fmt-sample-val {
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    color: var(--rm-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
