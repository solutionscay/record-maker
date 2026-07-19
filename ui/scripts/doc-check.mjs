// #45 editor document store test harness.
//
// Exercises the Svelte 5 runes store (src/lib/doc.svelte.ts) headlessly by
// loading it through the SAME vite SSR module graph the parity check uses — no
// test-runner dependency. Asserts the store's contract:
//   • hydration seeds state and pushes NO undo history;
//   • move/resize/set-prop undo + redo round-trip geometry EXACTLY;
//   • a mark groups multiple diffs into one atomic undo step;
//   • selection is session scope (updates live, never enters the undo history);
//   • the store's renderModel matches the committed #44 fixture, and feeding it
//     through <LayoutPreview> reproduces the committed Layout golden byte-for-
//     byte (so rendering from the store keeps parity green).
//
// `normalize()` below MUST stay byte-equivalent to scripts/parity-check.mjs and
// the Rust `normalize_html`. Do NOT edit tests/* to force a pass — the fixture
// and golden are authoritative.

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

// Replicates Rust `normalize_html` exactly (see scripts/parity-check.mjs).
function normalize(html) {
  return html
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/\s+/g, ' ')
    .replaceAll('> ', '>')
    .replaceAll(' <', '<')
    .trim();
}

let failures = 0;
function ok(name, cond, detail) {
  if (cond) {
    console.log(`  ok   ${name}`);
  } else {
    failures++;
    console.error(`  FAIL ${name}${detail ? `\n       ${detail}` : ''}`);
  }
}
function eq(name, actual, expected) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  ok(name, a === e, a === e ? '' : `expected ${e}\n       actual   ${a}`);
}

// The one object in the render model with the given id (search across parts).
function obj(model, id) {
  for (const p of model.parts) {
    const o = p.objects.find((x) => x.id === id);
    if (o) return o;
  }
  return undefined;
}
// Just the structural geometry of an object — what undo must restore exactly.
function geom(o) {
  return { x: o.x, y: o.y, w: o.w, h: o.h, z: o.z, readOnly: o.readOnly, binding: o.binding };
}

const fixture = JSON.parse(readFileSync(resolve(root, 'tests/canvas.fixture.json'), 'utf8'));
const golden = readFileSync(resolve(root, 'tests/canvas.layout.html'), 'utf8');
const fresh = () => structuredClone(fixture);

const { createServer } = await import('vite');
const vite = await createServer({
  root,
  server: { middlewareMode: true },
  appType: 'custom',
  logLevel: 'error',
});

try {
  const { EditorDoc } = await vite.ssrLoadModule('/src/lib/doc.svelte.ts');

  // 1. Hydration seeds state, records NO history, and projects the fixture back.
  {
    const d = new EditorDoc();
    ok('hydrate: not hydrated before', d.hydrated === false);
    d.hydrate(fresh());
    ok('hydrate: hydrated after', d.hydrated === true);
    ok('hydrate: history empty (no undo)', d.canUndo === false && d.canRedo === false);
    eq('hydrate: renderModel deep-equals fixture', d.renderModel, fixture);
    ok('layout grid: hydrates the layout-owned defaults', d.gridSize === 1 && d.showGrid && d.snapToGrid);
    d.setLayoutGrid(6, false, true);
    ok('layout grid: updates the shared projection without object history',
      d.renderModel.gridSize === 6 && !d.renderModel.showGrid && d.renderModel.snapToGrid && !d.canUndo);

    // #60 object kinds project through the store: a text label carries `content`
    // (no value), a value field carries its value (no content), a shape carries a
    // server-derived `shapeStyle`.
    const nameLabel = obj(d.renderModel, 1);
    const nameValue = obj(d.renderModel, 2);
    const rect = obj(d.renderModel, 14);
    ok('kinds: text label projects content only', nameLabel.kind === 'text' && nameLabel.content === 'Name' && nameLabel.value === '');
    ok('kinds: value field projects value only', nameValue.field === true && nameValue.value === 'Ada' && nameValue.content === '');
    ok('kinds: shape projects shapeStyle', rect.shape === true && rect.shapeStyle.startsWith('background:#eef') && rect.content === '');
  }

  // 2. moveObject → undo restores exact geometry → redo re-applies it.
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    const before = geom(obj(d.renderModel, 1));
    d.moveObject(1, 10, -4);
    d.mark();
    const moved = obj(d.renderModel, 1);
    ok('move: x/y changed', moved.x === before.x + 10 && moved.y === before.y - 4);
    ok('move: canUndo true', d.canUndo === true);
    d.undo();
    eq('move: undo restores exact geometry', geom(obj(d.renderModel, 1)), before);
    ok('move: undo empties past, fills redo', d.canUndo === false && d.canRedo === true);
    d.redo();
    eq('move: redo re-applies geometry', geom(obj(d.renderModel, 1)), {
      ...before,
      x: before.x + 10,
      y: before.y - 4,
    });
  }

  // 3. resizeObject (corner drag: shifts x/y AND changes w/h) round-trips.
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    const before = geom(obj(d.renderModel, 2));
    d.resizeObject(2, { x: before.x + 5, w: before.w - 5, h: before.h + 12 });
    d.mark();
    const r = obj(d.renderModel, 2);
    ok('resize: w/h/x changed, y untouched', r.x === before.x + 5 && r.w === before.w - 5 && r.h === before.h + 12 && r.y === before.y);
    d.undo();
    eq('resize: undo restores exact geometry', geom(obj(d.renderModel, 2)), before);

    const cancelled = d.beginGestureTransaction();
    d.resizeObject(2, { x: before.x + 17, w: before.w + 9 });
    const cancelledDiffs = d.cancelGestureTransaction(cancelled);
    eq('gesture transaction: cancel restores exact geometry', geom(obj(d.renderModel, 2)), before);
    ok('gesture transaction: cancel restores redo and seals no undo',
      cancelledDiffs.length === 2 && d.canUndo === false && d.canRedo === true);

    const committed = d.beginGestureTransaction();
    d.resizeObject(2, { x: before.x + 5 });
    d.commitGestureTransaction(committed);
    ok('gesture transaction: commit seals one undo step', d.canUndo === true);
    d.undo();
    eq('gesture transaction: committed step undoes normally', geom(obj(d.renderModel, 2)), before);
  }

  // 4. setProp on a structural prop (z) round-trips; no-op set records nothing.
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    const before = obj(d.renderModel, 1).z;
    d.setProp(1, 'z', before); // no-op: same value
    ok('setProp: no-op records no history', d.canUndo === false);
    d.setProp(1, 'z', before + 3);
    d.mark();
    ok('setProp: z applied', obj(d.renderModel, 1).z === before + 3);
    d.undo();
    ok('setProp: undo restores z', obj(d.renderModel, 1).z === before);
  }

  // 5. A mark groups MULTIPLE diffs (across objects/props) into ONE undo step.
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    const b1 = geom(obj(d.renderModel, 1));
    const b2 = geom(obj(d.renderModel, 2));
    d.moveObject(1, 7, 0);
    d.moveObject(1, 0, 9);
    d.setProp(1, 'readOnly', !b1.readOnly);
    d.resizeObject(2, { w: b2.w + 4 });
    d.mark(); // four diffs across two objects → one step
    ok('mark: edits applied pre-undo', obj(d.renderModel, 1).x === b1.x + 7 && obj(d.renderModel, 2).w === b2.w + 4);
    d.undo(); // single undo reverts the whole group
    eq('mark: one undo reverts all object 1 edits', geom(obj(d.renderModel, 1)), b1);
    eq('mark: one undo reverts the object 2 edit too', geom(obj(d.renderModel, 2)), b2);
    ok('mark: history empty after the single undo', d.canUndo === false);
  }

  // 6. Undo with an OPEN (un-marked) group auto-seals it, then reverts.
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    const before = geom(obj(d.renderModel, 1));
    d.moveObject(1, 3, 3); // never marked
    ok('auto-seal: canUndo with pending edits', d.canUndo === true);
    d.undo();
    eq('auto-seal: undo reverts the open group', geom(obj(d.renderModel, 1)), before);
  }

  // 7. Selection is session scope: it updates live and never enters undo history.
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    d.selectOnly([1, 2]);
    ok('selection: selectOnly sets the set', d.isSelected(1) && d.isSelected(2) && !d.isSelected(3) && d.selection.size === 2);
    d.toggle(2);
    ok('selection: toggle removes', !d.isSelected(2) && d.selection.size === 1);
    d.select(3);
    ok('selection: select replaces (non-additive)', d.isSelected(3) && !d.isSelected(1) && d.selection.size === 1);
    d.select(1, true);
    ok('selection: additive select keeps prior', d.isSelected(1) && d.isSelected(3) && d.selection.size === 2);
    const wasUndoable = d.canUndo;
    d.clearSelection();
    d.selectOnly([2]);
    ok('selection: changes record NO undo history', d.canUndo === wasUndoable && wasUndoable === false);
  }

  // 8. After undo, the step's touched objects become the selection (session-side).
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    d.clearSelection();
    d.moveObject(2, 1, 1);
    d.mark();
    d.undo();
    ok('selectTouched: undo selects the changed object', d.isSelected(2) && d.selection.size === 1);
  }

  // 8b. Durable groups expand selection and group/ungroup are undoable (#75/#114).
  {
    const d = new EditorDoc();
    const grouped = fresh();
    grouped.groups = [{ id: 99, objectIds: [1, 2] }];
    d.hydrate(grouped);
    d.selectOnly([1]);
    ok('groups: selecting one member selects the whole group', d.isSelected(1) && d.isSelected(2) && d.selection.size === 2);
    ok('groups: selection resolves active group id', d.groupIdForSelection() === 99);
    d.toggle(2);
    ok('groups: toggling a selected member removes the whole group', !d.isSelected(1) && !d.isSelected(2) && d.selection.size === 0);
    d.setGroup({ id: 100, objectIds: [3, 4] });
    ok('groups: setGroup replaces selection with new group', d.groupIdForSelection() === 100 && d.selection.size === 2);
    ok('groups: setGroup records undo history', d.canUndo === true);
    d.undo();
    ok('groups: undo group restores prior group only', d.groupIdForSelection([1, 2]) === 99 && d.groupIdForSelection([3, 4]) === null);
    d.redo();
    ok('groups: redo group restores new group', d.groupIdForSelection([3, 4]) === 100);
    d.removeGroup(100);
    ok('groups: removeGroup keeps child selection but clears active group', d.isSelected(3) && d.isSelected(4) && d.groupIdForSelection() === null);
    d.undo();
    ok('groups: undo ungroup restores group', d.groupIdForSelection([3, 4]) === 100);
    d.redo();
    ok('groups: redo ungroup removes group again', d.groupIdForSelection([3, 4]) === null);
  }

  // 9. Re-hydration resets history (a record refresh is not a user edit).
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    d.moveObject(1, 50, 50);
    d.mark();
    ok('rehydrate: dirty before', d.canUndo === true);
    d.hydrate(fresh());
    ok('rehydrate: history cleared', d.canUndo === false && d.canRedo === false);
    eq('rehydrate: renderModel back to fixture', d.renderModel, fixture);
  }

  // 9b. Structural create/delete + props are undoable (#48/#49). addObject and
  //     removeObject are `life` diffs (one atomic step); setObjectProps is a doc
  //     change. All round-trip through undo/redo exactly.
  {
    const d = new EditorDoc();
    d.hydrate(fresh());
    const partId = fixture.parts[0].id;
    const count = () => d.renderModel.parts[0].objects.length;
    const before = count();

    // create
    const newRect = {
      id: 99, kind: 'rect', field: false, shape: true, fieldId: null,
      x: 200, y: 10, w: 40, h: 40, z: 0, readOnly: false,
      binding: '', content: '', props: '{"fill":"#abc"}',
      objectStyle: '', textStyle: '', label: '', value: '', shapeStyle: 'background:#abc;',
    };
    d.addObject(newRect, partId);
    d.mark();
    ok('create: object added + canUndo', obj(d.renderModel, 99) !== undefined && count() === before + 1 && d.canUndo === true);
    d.undo();
    ok('create: undo removes it', obj(d.renderModel, 99) === undefined && count() === before);
    d.redo();
    const redone = obj(d.renderModel, 99);
    ok('create: redo restores it exactly', redone !== undefined && redone.shapeStyle === 'background:#abc;' && redone.props === '{"fill":"#abc"}');

    // delete (the fixture rect, id 14) → undo brings it back exactly
    const d2 = new EditorDoc();
    d2.hydrate(fresh());
    d2.removeObject(14);
    d2.mark();
    ok('delete: object removed', obj(d2.renderModel, 14) === undefined);
    d2.undo();
    const back = obj(d2.renderModel, 14);
    ok('delete: undo restores it exactly', back !== undefined && back.shapeStyle.startsWith('background:#eef'));

    // props edit → undoable document change (shapeStyle is session/server-derived,
    // so the doc props are the undo truth here)
    const d3 = new EditorDoc();
    d3.hydrate(fresh());
    const props0 = obj(d3.renderModel, 14).props;
    d3.setObjectProps(14, '{"fill":"#000000"}');
    d3.mark();
    ok('props: doc props updated', obj(d3.renderModel, 14).props === '{"fill":"#000000"}');
    d3.undo();
    ok('props: undo restores props', obj(d3.renderModel, 14).props === props0);
  }

  // 9b. #184 portal geometry is one row; props independently control the full
  // preview footprint and remain ordinary undoable document state.
  {
    const model = fresh();
    const portal = {
      id: 184, kind: 'portal', field: false, shape: false, fieldId: null,
      x: 8, y: 8, w: 280, h: 24, z: 0, readOnly: false,
      binding: 'orders', content: '',
      props: '{"rowCount":4,"fill":"#abc123","stroke":"#123abc","strokeWidth":2}',
      objectStyle: 'background:#abc123;box-shadow:0 0 0 2px #123abc;',
      textStyle: '', label: '', value: '', shapeStyle: '',
      portalRowHeight: 24, portalRowCount: 4,
    };
    model.parts[1].objects.push(portal);
    const d = new EditorDoc();
    d.hydrate(model);
    const first = obj(d.renderModel, 184);
    ok('portal rows: store derives one-row geometry + explicit count',
      first.h === 24 && first.portalRowHeight === 24 && first.portalRowCount === 4);
    ok('portal rows: full preview contributes to minimum band height', d.minPartHeight(2) === 104);

    d.setObjectProps(184, '{"rowCount":7}');
    d.mark();
    ok('portal rows: props edit changes count without changing row height',
      obj(d.renderModel, 184).portalRowCount === 7 && obj(d.renderModel, 184).h === 24);
    ok('portal rows: expanded preview updates effective footprint', d.minPartHeight(2) === 176);
    d.undo();
    ok('portal rows: undo restores count', obj(d.renderModel, 184).portalRowCount === 4);

    d.resizeObject(184, { w: 320, h: 32 });
    d.mark();
    ok('portal rows: direct resize edits first-row width and height',
      obj(d.renderModel, 184).w === 320 &&
      obj(d.renderModel, 184).h === 32 &&
      obj(d.renderModel, 184).portalRowHeight === 32 &&
      obj(d.renderModel, 184).portalRowCount === 4);
    ok('portal rows: resized row reflows the fixed-count footprint', d.minPartHeight(2) === 136);
    d.undo();

    const mod = await vite.ssrLoadModule('/src/lib/LayoutPreview.svelte');
    const { render } = await vite.ssrLoadModule('svelte/server');
    const { body } = render(mod.default, { props: { model: d.renderModel } });
    ok('portal rows: Layout preview paints below one-row selection box',
      body.includes('fm-obj fm-portal-obj') &&
      body.includes('fm-portal fm-portal-preview') &&
      body.includes('--fm-portal-row-h: 24px;--fm-portal-h: 96px'));
    ok('portal style: fill and border paint on the full preview',
      body.includes('--fm-portal-h: 96px;background:#abc123;box-shadow:0 0 0 2px #123abc;'));
    ok('portal style: one-row selection wrapper carries geometry only',
      body.includes('left:8px; top:8px; width:280px; height:24px; z-index:0;') &&
      !body.includes('z-index:0;background:#abc123'));
  }

  // 9c. #203 portal movement expands to owned columns without changing selection.
  {
    const model = fresh();
    const body = model.parts[1];
    const baseObject = {
      field: false, shape: false, fieldId: null, z: 0, readOnly: false,
      binding: '', content: '', props: '', objectStyle: '', textStyle: '',
      label: '', value: '', shapeStyle: '',
    };
    body.objects.push(
      { ...baseObject, id: 203, kind: 'portal', x: 40, y: 56, w: 240, h: 24, binding: 'orders', portalRowHeight: 24, portalRowCount: 5 },
      { ...baseObject, id: 204, parentObjectId: 203, kind: 'text', x: 48, y: 32, w: 96, h: 24, content: 'Amount' },
      { ...baseObject, id: 205, parentObjectId: 203, kind: 'field', field: true, fieldId: 9, x: 48, y: 56, w: 96, h: 24, binding: 'orders.Amount' },
      { ...baseObject, id: 206, kind: 'text', x: 8, y: 8, w: 40, h: 24, content: 'Outside' },
    );
    const d = new EditorDoc();
    d.hydrate(model);
    d.selectOnly([203]);
    eq('portal movement: portal expands to both owned objects', d.movementObjectIds(), [203, 204, 205]);
    ok('portal movement: expansion does not alter visible selection', d.selection.size === 1 && d.isSelected(203));

    d.selectOnly([203, 205]);
    eq('portal movement: explicitly selected child is de-duplicated', d.movementObjectIds(), [203, 204, 205]);

    d.selectOnly([203]);
    const before = new Map([203, 204, 205, 206].map((id) => [id, geom(obj(d.renderModel, id))]));
    for (const id of d.movementObjectIds()) d.moveObject(id, 17, 11);
    d.mark();
    for (const id of [203, 204, 205]) {
      const moved = obj(d.renderModel, id);
      ok(`portal movement: object ${id} receives the common delta`,
        moved.x === before.get(id).x + 17 && moved.y === before.get(id).y + 11);
    }
    eq('portal movement: unrelated object stays fixed', geom(obj(d.renderModel, 206)), before.get(206));
    ok('portal movement: portal remains the only visible selection', d.selection.size === 1 && d.isSelected(203));
    d.undo();
    for (const id of [203, 204, 205]) {
      eq(`portal movement: undo restores object ${id}`, geom(obj(d.renderModel, id)), before.get(id));
    }

    const childBeforeResize = geom(obj(d.renderModel, 205));
    d.resizeObject(203, { w: 300, h: 30 });
    d.mark();
    eq('portal movement: resizing the portal leaves its child geometry alone', geom(obj(d.renderModel, 205)), childBeforeResize);
  }

  // 9d. #117 Table column projection: explicit order wins over geometry, hidden
  // fields move to Available, and LayoutPreview suppresses the hidden field
  // object while leaving unrelated authored objects alone.
  {
    const { projectTableColumns, withTableColumnSettings } =
      await vite.ssrLoadModule('/src/lib/table-columns.ts');
    const model = fresh();
    model.view = 'table';
    const name = obj(model, 2);
    const email = obj(model, 4);
    email.props = JSON.stringify(withTableColumnSettings(email.props, { visible: true, order: 0 }));
    name.props = JSON.stringify(withTableColumnSettings(name.props, { visible: true, order: 1 }));

    let projected = projectTableColumns(model);
    eq('table columns: explicit order drives Visible', projected.visible.map((row) => row.field.name), ['Email', 'Name']);
    eq('table columns: unplaced schema fields are Available', projected.available.map((row) => row.field.name), ['ID']);

    name.props = JSON.stringify(withTableColumnSettings(name.props, { visible: false }));
    projected = projectTableColumns(model);
    eq('table columns: hidden field leaves Visible', projected.visible.map((row) => row.field.name), ['Email']);
    eq('table columns: hidden field joins Available in schema order', projected.available.map((row) => row.field.name), ['ID', 'Name']);

    const mod = await vite.ssrLoadModule('/src/lib/LayoutPreview.svelte');
    const { render } = await vite.ssrLoadModule('svelte/server');
    const { body } = render(mod.default, { props: { model } });
    ok('table columns: hidden field object is absent from Table canvas', !body.includes('data-object-id="2"'));
    ok('table columns: independent caption object remains inspectable', body.includes('data-object-id="1"'));
  }

  // 10. Full loop: store.renderModel → <LayoutPreview> SSR === committed golden.
  {
    const mod = await vite.ssrLoadModule('/src/lib/LayoutPreview.svelte');
    const { render } = await vite.ssrLoadModule('svelte/server');
    const d = new EditorDoc();
    d.hydrate(fresh());
    const { body } = render(mod.default, { props: { model: d.renderModel } });
    const a = normalize(body);
    const e = normalize(golden);
    ok('parity: store-rendered DOM equals the golden', a === e, a === e ? '' : `first diff at ${[...a].findIndex((c, i) => c !== e[i])}`);
  }

  // 11. Pure interaction helpers (#46) — snap, paint-order ids, element mapping.
  {
    const { snapToGrid, clampOrigin, objectIdsInPaintOrder, elementsToObjectIds, DEFAULT_GRID_SIZE } =
      await vite.ssrLoadModule('/src/lib/canvas-edit.ts');

    ok('snap: rounds to the nearest grid line', snapToGrid(19, 8) === 16 && snapToGrid(20, 8) === 24);
    ok('snap: 1px default + grid<=0 both preserve whole-pixel flow', snapToGrid(11.4) === DEFAULT_GRID_SIZE * Math.round(11.4 / DEFAULT_GRID_SIZE) && snapToGrid(7.4, 0) === 7);
    ok('clampOrigin: never negative, rounds', clampOrigin(-3) === 0 && clampOrigin(4.6) === 5);

    // Paint order mirrors the fixture's (z,id) ordering: the z=0 objects by id
    // (Name label 1, Name value 2, Email label 3, Note 13, rect 14) then the z=5
    // Email value (4) last → [1, 2, 3, 13, 14, 4].
    eq('paintOrder: ids match LayoutPreview order', objectIdsInPaintOrder(fixture), [1, 2, 3, 13, 14, 4]);

    // Element→id mapping is index-based; fake elements (identity only) suffice.
    const painted = [{ n: 'a' }, { n: 'b' }, { n: 'c' }, { n: 'd' }, { n: 'e' }, { n: 'f' }];
    const ids = objectIdsInPaintOrder(fixture); // [1, 2, 3, 13, 14, 4]
    eq('elementsToIds: maps selected elements by index', elementsToObjectIds([painted[2], painted[0]], painted, ids), [3, 1]);
    eq('elementsToIds: drops unknown elements', elementsToObjectIds([{ n: 'x' }], painted, ids), []);
  }

  // 12. Echo planning for grouped history geometry (#88).
  {
    const { buildHistoryEchoSpecs } = await vite.ssrLoadModule('/src/lib/echo.ts');
    const d = new EditorDoc();
    d.hydrate(fresh());
    const before = obj(d.renderModel, 1);
    d.moveObject(1, 8, 0);
    d.moveObject(1, 8, 0);
    d.mark();
    const undoStep = d.undo();
    const undoSpec = buildHistoryEchoSpecs(d, undoStep, 'undo')[0];
    eq('echo: undo starts at grouped final x and lands at prior x', {
      from: undoSpec.from.x,
      to: undoSpec.to.x,
    }, {
      from: before.x + 16,
      to: before.x,
    });
    const redoStep = d.redo();
    const redoSpec = buildHistoryEchoSpecs(d, redoStep, 'redo')[0];
    eq('echo: redo starts at grouped prior x and lands at final x', {
      from: redoSpec.from.x,
      to: redoSpec.to.x,
    }, {
      from: before.x,
      to: before.x + 16,
    });
  }

  // 13. Shared object props and line geometry helpers (#92).
  {
    const { parseProps, normalizeAngle, lineLength, linePropsForBox, lineGeometryForAngle, lineAngle } =
      await vite.ssrLoadModule('/src/lib/object-props.ts');

    eq('parseProps: valid object parses', parseProps('{"fill":"#fff","strokeWidth":2}'), {
      fill: '#fff',
      strokeWidth: 2,
    });
    eq('parseProps: invalid JSON/arrays/non-objects become empty bags', [
      parseProps('{'),
      parseProps('[1,2]'),
      parseProps('"x"'),
    ], [{}, {}, {}]);
    ok('angle: normalization wraps and rounds', normalizeAngle(-45) === 315 && normalizeAngle(720.126) === 0.13);
    ok('line: angle from endpoints normalizes', lineAngle(0, 0, 0, -10) === 270);
    ok('line: explicit length prop wins', lineLength({ w: 3, h: 4 }, { length: 12 }) === 12);
    eq('line: resized box derives visible angle and length', linePropsForBox({ w: 30, h: 40 }, { stroke: '#123456' }), {
      stroke: '#123456',
      angle: 53.13,
      length: 50,
    });
    eq('line: thin horizontal box stays horizontal', linePropsForBox({ w: 80, h: 1 }, { angle: 0 }), {
      angle: 0,
      length: 80,
    });
    eq('line: geometry rotates around center', lineGeometryForAngle({ x: 10, y: 20, w: 30, h: 10 }, 90, 40), {
      x: 25,
      y: 5,
      w: 1,
      h: 40,
    });
  }

  // 14. Canvas press intent and per-kind behavior registry (#99/#205).
  {
    const { classifyPress } = await vite.ssrLoadModule('/src/lib/canvas/press-intent.ts');
    const basePress = {
      activeTool: 'pointer',
      ctrlKey: false,
      metaKey: false,
      shiftKey: false,
      objectId: null,
      objectIsTargeted: false,
      moveableChrome: false,
    };
    eq('press intent: armed tool owns placement', classifyPress({ ...basePress, activeTool: 'rect' }), { kind: 'place' });
    eq('press intent: Control owns containment marquee', classifyPress({ ...basePress, ctrlKey: true, objectId: 7 }), {
      kind: 'containment-marquee',
    });
    eq('press intent: Shift object press stays pending', classifyPress({ ...basePress, shiftKey: true, objectId: 7 }), {
      kind: 'toggle', id: 7,
    });
    eq('press intent: unselected object selects and drags', classifyPress({ ...basePress, objectId: 7 }), {
      kind: 'drag', id: 7, select: true,
    });

    const { objectBehavior } = await vite.ssrLoadModule('/src/lib/canvas/object-behavior.ts');
    const text = objectBehavior('text');
    const line = objectBehavior('line');
    ok('behavior registry: generic text owns default content', text.defaultContent === 'Text' && !text.rotatable);
    ok('behavior registry: line owns rotation and resize persistence', line.rotatable && line.persistAfterResize);
    eq('behavior registry: line draw resolves geometry and props', {
      draw: line.drawGeometry({ startX: 10, startY: 20, endX: 40, endY: 60, snap: (value) => value }),
      props: line.placementProps({
        dragged: true,
        box: { x: 10, y: 20, w: 30, h: 40 },
        partTop: 0,
        line: { angle: 53.13, length: 50 },
      }),
    }, {
      draw: { x: 10, yGlobal: 20, w: 30, h: 40, line: { angle: 53.13, length: 50 } },
      props: { stroke: '#888888', strokeWidth: 2, angle: 53.13, length: 50 },
    });

    const { buildGuideIndex, candidatesNearGuideBox, resolveMoveGuides, resolveResizeGuides, unionGuideBoxes } =
      await vite.ssrLoadModule('/src/lib/canvas/smart-guides.ts');
    const candidates = [{ id: 2, box: { x: 300, y: 80, w: 100, h: 40 } }];
    eq('smart guides: move geometry and chrome share one resolution',
      resolveMoveGuides({ x: 196, y: 38, w: 100, h: 40 }, candidates, 5), {
        box: { x: 200, y: 40, w: 100, h: 40 },
        guides: [{ axis: 'x', position: 300 }, { axis: 'y', position: 80 }],
      });
    eq('smart guides: resize constrains only active edges',
      resolveResizeGuides({ x: 100, y: 20, w: 196, h: 58 }, [1, 1], candidates, 5), {
        box: { x: 100, y: 20, w: 200, h: 60 },
        guides: [{ axis: 'x', position: 300 }, { axis: 'y', position: 80 }],
      });
    eq('smart guides: group union is deterministic', unionGuideBoxes([
      { x: 10, y: 30, w: 20, h: 10 },
      { x: 50, y: 10, w: 15, h: 25 },
    ]), { x: 10, y: 10, w: 55, h: 30 });
    const indexedCandidates = [
      ...candidates,
      ...Array.from({ length: 200 }, (_, index) => ({
        id: index + 10,
        box: { x: 1_000 + index * 20, y: 1_000 + index * 20, w: 10, h: 10 },
      })),
    ];
    eq('smart guides: spatial index returns only capable nearby candidates',
      candidatesNearGuideBox(
        buildGuideIndex(indexedCandidates),
        { x: 196, y: 38, w: 100, h: 40 },
        5,
      ).map((candidate) => candidate.id),
      [2]);

    const { edgeVelocity } = await vite.ssrLoadModule('/src/lib/canvas/autoscroll.ts');
    eq('autoscroll: center has no velocity', edgeVelocity(50, 0, 100, 20), 0);
    ok('autoscroll: edge direction and proximity ramp are deterministic',
      edgeVelocity(1, 0, 100, 20) < edgeVelocity(15, 0, 100, 20) &&
      edgeVelocity(99, 0, 100, 20) > edgeVelocity(85, 0, 100, 20));
  }
} finally {
  await vite.close();
}

if (failures > 0) {
  console.error(`\ndoc-check: ${failures} assertion(s) FAILED`);
  process.exit(1);
}
console.log('\ndoc-check OK');
