// #191 Inspector border-placement interaction state. Loads the exact TypeScript
// helper through Vite (the same path the Svelte Inspector uses) and exercises
// legacy fallback, independent edge toggles, All, and portal-only Middle.

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
let failures = 0;

function eq(name, actual, expected) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a === e) console.log(`  ok   ${name}`);
  else {
    failures++;
    console.error(`  FAIL ${name}\n       expected ${e}\n       actual   ${a}`);
  }
}

function ok(name, condition, detail = '') {
  if (condition) console.log(`  ok   ${name}`);
  else {
    failures++;
    console.error(`  FAIL ${name}${detail ? `\n       ${detail}` : ''}`);
  }
}

const vite = await createServer({
  root,
  server: { middlewareMode: true },
  appType: 'custom',
  logLevel: 'error',
});

try {
  const {
    strokeSides,
    withAllOuterStrokeSides,
    withStrokeSide,
  } = await vite.ssrLoadModule('/src/lib/border-sides.ts');
  const StyleSection = (await vite.ssrLoadModule('/src/lib/inspector/StyleSection.svelte')).default;
  const { render } = await vite.ssrLoadModule('svelte/server');

  eq('legacy metadata resolves to all four outer edges', strokeSides({}), [
    'top', 'right', 'bottom', 'left',
  ]);
  eq('explicit empty placement renders no lines', strokeSides({ strokeSides: [] }), []);
  eq(
    'turning Middle on preserves the outer frame',
    withStrokeSide({}, 'middle', true),
    ['top', 'right', 'bottom', 'left', 'middle'],
  );
  eq(
    'turning All off preserves independent Middle',
    withAllOuterStrokeSides({ strokeSides: ['top', 'right', 'bottom', 'left', 'middle'] }, false),
    ['middle'],
  );
  eq(
    'turning All on preserves independent Middle',
    withAllOuterStrokeSides({ strokeSides: ['middle'] }, true),
    ['top', 'right', 'bottom', 'left', 'middle'],
  );
  eq(
    'one edge toggle preserves every other authored placement',
    withStrokeSide({ strokeSides: ['right', 'bottom', 'middle'] }, 'left', true),
    ['right', 'bottom', 'left', 'middle'],
  );
  eq(
    'writes are canonical and ignore unknown values',
    strokeSides({ strokeSides: ['middle', 'bogus', 'top', 'top'] }),
    ['top', 'middle'],
  );

  const object = (id, kind, props) => ({ id, kind, props: JSON.stringify(props) });
  const fakeDoc = {};
  const portalHtml = render(StyleSection, {
    props: {
      doc: fakeDoc,
      objects: [
        object(1, 'portal', { strokeSides: ['left', 'middle'] }),
        object(2, 'portal', { strokeSides: ['left'] }),
      ],
    },
  }).body;
  ok(
    'portal Inspector renders All, four outer edges, and independent Middle',
    ['All outer borders', 'Left border', 'Right border', 'Top border', 'Bottom border', 'Middle row separators']
      .every((label) => portalHtml.includes(`aria-label="${label}"`)),
    portalHtml,
  );
  ok(
    'portal multi-selection renders mixed Middle state',
    portalHtml.includes('aria-label="Middle row separators" aria-pressed="mixed"'),
    portalHtml,
  );

  const fieldHtml = render(StyleSection, {
    props: { doc: fakeDoc, objects: [object(3, 'field', {})] },
  }).body;
  ok(
    'field Inspector has outer placements but no portal Middle',
    fieldHtml.includes('aria-label="All outer borders"') && !fieldHtml.includes('Middle row separators'),
    fieldHtml,
  );

  const ellipseHtml = render(StyleSection, {
    props: { doc: fakeDoc, objects: [object(4, 'ellipse', {})] },
  }).body;
  ok(
    'ellipse Inspector keeps uniform stroke without edge placement controls',
    !ellipseHtml.includes('aria-label="Border placement"'),
    ellipseHtml,
  );
} finally {
  await vite.close();
}

if (failures) {
  console.error(`\n${failures} border-placement check(s) failed`);
  process.exit(1);
}
console.log('\nborder-placement checks passed');
