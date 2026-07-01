// #44 shared-renderer parity gate.
//
// Server-renders the Svelte <LayoutPreview> from the committed fixture model
// and asserts it normalizes to the committed Layout golden. Browse has its own
// server-rendered golden because field objects intentionally show record values
// there, while Layout shows field labels.
//
// `normalize()` below MUST stay byte-equivalent to the Rust `normalize_html`
// in crates/server/src/main.rs. Do NOT edit tests/* or relax normalization to
// force a pass — the golden is authoritative.

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

// Replicates Rust `normalize_html` exactly, in order:
//   1. strip HTML comments,
//   2. collapse whitespace runs to a single space,
//   3. drop spaces adjacent to tag boundaries,
//   4. trim.
function normalize(html) {
  return html
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/\s+/g, ' ')
    .replaceAll('> ', '>')
    .replaceAll(' <', '<')
    .trim();
}

function firstDiff(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return i;
  }
  return a.length === b.length ? -1 : n;
}

const model = JSON.parse(readFileSync(resolve(root, 'tests/canvas.fixture.json'), 'utf8'));
const golden = readFileSync(resolve(root, 'tests/canvas.layout.html'), 'utf8');

const { createServer } = await import('vite');
const vite = await createServer({
  root,
  server: { middlewareMode: true },
  appType: 'custom',
  logLevel: 'error',
});

let body;
try {
  const mod = await vite.ssrLoadModule('/src/lib/LayoutPreview.svelte');
  // Load `render` through the SAME vite SSR module graph as the component.
  // vite-plugin-svelte does not externalize svelte for SSR, so the compiled
  // component resolves `svelte/internal/server` via vite's graph; a plain
  // `import('svelte/server')` would be a separate module instance with its own
  // (null) render context, so its `render()` cannot drive the component. Going
  // through ssrLoadModule keeps a single, consistent svelte runtime.
  const { render } = await vite.ssrLoadModule('svelte/server');
  ({ body } = render(mod.default, { props: { model } }));
} finally {
  await vite.close();
}

const expected = normalize(golden);
const actual = normalize(body);

if (expected === actual) {
  console.log('parity OK');
  process.exit(0);
}

const at = firstDiff(expected, actual);
console.error('parity MISMATCH');
console.error(`first difference at index ${at}`);
console.error('--- expected (normalized) ---');
console.error(expected);
console.error('--- actual (normalized) ---');
console.error(actual);
if (at >= 0) {
  const ctx = 40;
  console.error('--- expected @diff ---');
  console.error(JSON.stringify(expected.slice(Math.max(0, at - ctx), at + ctx)));
  console.error('--- actual   @diff ---');
  console.error(JSON.stringify(actual.slice(Math.max(0, at - ctx), at + ctx)));
}
process.exit(1);
