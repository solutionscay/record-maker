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
//     through <LayoutPreview> reproduces the committed canvas golden byte-for-
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
const golden = readFileSync(resolve(root, 'tests/canvas.parity.html'), 'utf8');
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
    const { snapToGrid, clampOrigin, objectIdsInPaintOrder, elementsToObjectIds, GRID } =
      await vite.ssrLoadModule('/src/lib/canvas-edit.ts');

    ok('snap: rounds to the nearest grid line', snapToGrid(19, 8) === 16 && snapToGrid(20, 8) === 24);
    ok('snap: default grid + grid<=0 just rounds', snapToGrid(11) === GRID * Math.round(11 / GRID) && snapToGrid(7.4, 0) === 7);
    ok('clampOrigin: never negative, rounds', clampOrigin(-3) === 0 && clampOrigin(4.6) === 5);

    // Paint order mirrors the fixture's (z,id) ordering: ids 1, 3, 2.
    eq('paintOrder: ids match LayoutPreview order', objectIdsInPaintOrder(fixture), [1, 3, 2]);

    // Element→id mapping is index-based; fake elements (identity only) suffice.
    const painted = [{ n: 'a' }, { n: 'b' }, { n: 'c' }];
    const ids = objectIdsInPaintOrder(fixture); // [1,3,2]
    eq('elementsToIds: maps selected elements by index', elementsToObjectIds([painted[2], painted[0]], painted, ids), [2, 1]);
    eq('elementsToIds: drops unknown elements', elementsToObjectIds([{ n: 'x' }], painted, ids), []);
  }
} finally {
  await vite.close();
}

if (failures > 0) {
  console.error(`\ndoc-check: ${failures} assertion(s) FAILED`);
  process.exit(1);
}
console.log('\ndoc-check OK');
