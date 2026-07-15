// #188/#189 focused checks for the object inspector's edge-coordinate math.

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
let failures = 0;

function eq(name, actual, expected) {
  const pass = JSON.stringify(actual) === JSON.stringify(expected);
  console[pass ? 'log' : 'error'](`  ${pass ? 'ok  ' : 'FAIL'} ${name}`);
  if (!pass) {
    failures++;
    console.error(`       expected ${JSON.stringify(expected)}\n       actual   ${JSON.stringify(actual)}`);
  }
}

const vite = await createServer({
  root,
  server: { middlewareMode: true },
  appType: 'custom',
  logLevel: 'error',
});

try {
  const { edgeValue, geometryForEdge } = await vite.ssrLoadModule('/src/lib/inspector/position.ts');
  const box = { x: 20, y: 30, w: 80, h: 40 };

  eq('edge values derive from x/y/w/h', ['left', 'right', 'top', 'bottom'].map((edge) => edgeValue(box, edge)), [20, 100, 30, 70]);
  eq('left moves without changing size', geometryForEdge(box, 'left', 35), { x: 35, y: 30, w: 80, h: 40 });
  eq('right moves without changing size', geometryForEdge(box, 'right', 125), { x: 45, y: 30, w: 80, h: 40 });
  eq('top moves without changing size', geometryForEdge(box, 'top', 45), { x: 20, y: 45, w: 80, h: 40 });
  eq('bottom moves without changing size', geometryForEdge(box, 'bottom', 95), { x: 20, y: 55, w: 80, h: 40 });
  eq('right cannot move the left edge before origin', geometryForEdge(box, 'right', 79), null);
  eq('bottom cannot move the top edge before origin', geometryForEdge(box, 'bottom', 39), null);
  eq('coordinates cannot leave the grid origin', geometryForEdge(box, 'top', -1), null);
} finally {
  await vite.close();
}

if (failures) {
  console.error(`\n${failures} position check(s) failed`);
  process.exit(1);
}
console.log('\nposition checks passed');
