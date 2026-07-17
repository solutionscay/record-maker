// #190 focused checks for the object inspector's single- and multi-selection
// dimension controls, including their rendered shared/mixed states.

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const fixture = JSON.parse(readFileSync(resolve(root, 'tests/canvas.fixture.json'), 'utf8'));
let failures = 0;

function ok(name, condition, detail = '') {
  console[condition ? 'log' : 'error'](`  ${condition ? 'ok  ' : 'FAIL'} ${name}`);
  if (!condition) {
    failures++;
    if (detail) console.error(`       ${detail}`);
  }
}

function eq(name, actual, expected) {
  const pass = JSON.stringify(actual) === JSON.stringify(expected);
  ok(name, pass, pass ? '' : `expected ${JSON.stringify(expected)}\n       actual   ${JSON.stringify(actual)}`);
}

function inputFor(html, label) {
  return html.match(new RegExp(`<input[^>]*aria-label="${label}"[^>]*>`))?.[0] ?? '';
}

const vite = await createServer({
  root,
  server: { middlewareMode: true },
  appType: 'custom',
  logLevel: 'error',
});

try {
  const { EditorDoc } = await vite.ssrLoadModule('/src/lib/doc.svelte.ts');
  const { applyLiveDimension, dimensionPixels, sharedDimension } = await vite.ssrLoadModule('/src/lib/inspector/size.ts');
  const Inspector = (await vite.ssrLoadModule('/src/lib/Inspector.svelte')).default;
  const { render } = await vite.ssrLoadModule('svelte/server');

  eq('dimension input rounds positive pixels', dimensionPixels('72.6'), 73);
  eq('dimension input rejects empty, zero, and non-numeric drafts',
    ['', '0', '-2', 'wide'].map(dimensionPixels), [null, null, null, null]);

  const sharedDoc = new EditorDoc();
  sharedDoc.hydrate(structuredClone(fixture));
  const emptyHtml = render(Inspector, { props: { doc: sharedDoc, layoutId: '1' } }).body;
  ok('empty canvas Inspector exposes the layout-owned grid',
    emptyHtml.includes('Layout Grid') && inputFor(emptyHtml, 'Layout grid size in pixels').includes('value="1"'));
  sharedDoc.selectPart(2);
  const bandHtml = render(Inspector, { props: { doc: sharedDoc, layoutId: '1' } }).body;
  ok('any selected band exposes the same Layout Grid section',
    bandHtml.includes('>Band</span>') && bandHtml.includes('Layout Grid'));
  sharedDoc.clearSelection();
  const sharedObjects = [sharedDoc.getObject(1), sharedDoc.getObject(3)];
  eq('matching dimensions resolve to a shared value', sharedDimension(sharedObjects, 'w'), { mixed: false, value: 72 });
  sharedDoc.selectOnly([1, 3]);
  const sharedHtml = render(Inspector, { props: { doc: sharedDoc, layoutId: '1' } }).body;
  ok('multi-selection renders the Size card', sharedHtml.includes('Multiple items selected') && sharedHtml.includes('>Size</span>'));
  ok('matching values render in both numeric controls',
    inputFor(sharedHtml, 'Width in pixels').includes('value="72"') &&
    inputFor(sharedHtml, 'Height in pixels').includes('value="24"'));

  const mixedDoc = new EditorDoc();
  mixedDoc.hydrate(structuredClone(fixture));
  const mixedObjects = [mixedDoc.getObject(1), mixedDoc.getObject(2)];
  eq('different dimensions resolve to mixed', sharedDimension(mixedObjects, 'w'), { mixed: true, value: 72 });
  mixedDoc.selectOnly([1, 2]);
  const mixedHtml = render(Inspector, { props: { doc: mixedDoc, layoutId: '1' } }).body;
  ok('mixed width renders an explicit placeholder',
    inputFor(mixedHtml, 'Width in pixels').includes('placeholder="Mixed"'));
  ok('shared height remains visible beside a mixed width',
    inputFor(mixedHtml, 'Height in pixels').includes('value="24"'));

  const beforeWidths = [mixedDoc.getObject(1).w, mixedDoc.getObject(2).w];
  applyLiveDimension(mixedDoc, [1, 2], 'w', 155);
  mixedDoc.mark();
  eq('applying width updates every selected object',
    [mixedDoc.getObject(1).w, mixedDoc.getObject(2).w], [155, 155]);
  mixedDoc.undo();
  eq('one undo restores every selected object',
    [mixedDoc.getObject(1).w, mixedDoc.getObject(2).w], beforeWidths);
} finally {
  await vite.close();
}

if (failures) {
  console.error(`\n${failures} size check(s) failed`);
  process.exit(1);
}
console.log('\nsize checks passed');
