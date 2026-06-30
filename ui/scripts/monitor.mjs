// Headless Layout-Mode monitor — automated debugging harness (#62).
//
// Drives the live design canvas in a real (headless) browser, runs a repro, and
// captures BOTH the structured in-page log buffer (window.__rmLogs, see
// src/lib/log.ts) and the resulting DOM/geometry state + screenshots. This turns
// "the canvas is clunky, I don't know where to start" into a deterministic trace
// you can read top-to-bottom: every tool arm, click→model mapping, create
// round-trip, store mutation, selection, and moveable target/resize step.
//
// Prereqs:
//   • a dev server on $BASE (default http://127.0.0.1:4317) serving the CURRENT
//     ui/dist — rebuild with `npm run build` after editing the UI;
//   • a Chrome/Chromium at $CHROME (default /usr/bin/google-chrome-stable);
//   • `playwright-core` (devDependency) — no bundled browser, uses $CHROME.
//
// Run:  cd ui && npm run build && npm run monitor
// Out:  ui/.monitor/{rmlogs.json, obs.json, console.json, 0*-*.png}
//
// Extend: add steps to the REPRO section. `step(name, fn)` records fn()'s result
// (or its error) under obs[name]; `page` is a Playwright Page. Read rmlogs.json to
// see what the app logged during each step.

import { chromium } from 'playwright-core';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = process.env.MONITOR_OUT || resolve(root, '.monitor');
const BASE = process.env.BASE || 'http://127.0.0.1:4317';
const DESIGN = process.env.DESIGN_PATH || '/design/1';
const CHROME = process.env.CHROME || '/usr/bin/google-chrome-stable';
mkdirSync(OUT, { recursive: true });

const obs = {};
const step = async (name, fn) => {
  try {
    obs[name] = await fn();
  } catch (e) {
    obs[name] = { ERROR: String(e) };
  }
};

const browser = await chromium.launch({
  executablePath: CHROME,
  headless: true,
  args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage'],
});
const page = await browser.newPage({ viewport: { width: 1500, height: 950 } });
const consoleMsgs = [];
page.on('console', (m) => consoleMsgs.push({ type: m.type(), text: m.text() }));
page.on('pageerror', (e) => consoleMsgs.push({ type: 'pageerror', text: e.message }));

// readiness: retry until the design page + canvas are up
let up = false;
for (let i = 0; i < 20 && !up; i++) {
  try {
    await page.goto(`${BASE}${DESIGN}`, { waitUntil: 'load', timeout: 5000 });
    await page.waitForSelector('.fm-canvas', { timeout: 2000 });
    up = true;
  } catch {
    await page.waitForTimeout(500);
  }
}
obs.serverUp = up;
await page.waitForTimeout(400);

// ─────────────────────────────── REPRO ───────────────────────────────────────
// Initial state.
await step('railButtons', () => page.$$eval('#layout-tools button[title]', (els) => els.map((e) => e.title)));
await step('canvasBox', () => page.locator('.fm-canvas').boundingBox());
await step('parts', () =>
  page.$$eval('.fm-canvas .fm-part', (els) =>
    els.map((e) => {
      const r = e.getBoundingClientRect();
      const cs = getComputedStyle(e);
      return { height: e.style.height, rectH: Math.round(r.height), outline: cs.outline };
    }),
  ),
);
await step('initialObjects', () =>
  page.$$eval('.fm-canvas .fm-obj', (els) => els.map((e) => e.getAttribute('style'))),
);
await page.screenshot({ path: `${OUT}/01-initial.png` });

const cb = obs.canvasBox;

// Arm the Ellipse tool and click the canvas (over an existing field, on purpose —
// the bug only showed when the click landed over another object).
await step('pickEllipse', async () => {
  await page.click('#layout-tools button[title="Ellipse"]');
  await page.waitForTimeout(120);
  return page.$eval('#layout-tools button[title="Ellipse"]', (e) => e.classList.contains('active'));
});
const clickRel = { x: 150, y: 60 };
obs.clickRel = clickRel;
await step('placeEllipse', async () => {
  await page.mouse.click(cb.x + clickRel.x, cb.y + clickRel.y);
  await page.waitForTimeout(600);
  return true;
});
await step('objectsAfterPlace', () =>
  page.$$eval('.fm-canvas .fm-obj', (els) => els.map((e) => e.getAttribute('style'))),
);
await step('moveableControlBoxes', () => page.$$eval('.moveable-control-box', (els) => els.length));
// Where moveable actually put its box (which object it targeted).
await step('moveableBox', async () => {
  const b = await page.$('.moveable-control-box');
  return b ? b.boundingBox() : null;
});
await page.screenshot({ path: `${OUT}/03-after-place.png` });

// Attempt a resize via the SE handle.
await step('resize', async () => {
  const h = await page.$('.moveable-control.moveable-se');
  if (!h) return { handleFound: false };
  const hb = await h.boundingBox();
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  await page.mouse.move(hb.x + 50, hb.y + 40, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(250);
  return { handleFound: true, handleBox: hb };
});
await step('objectsAfterResize', () =>
  page.$$eval('.fm-canvas .fm-obj', (els) => els.map((e) => e.getAttribute('style'))),
);
await page.screenshot({ path: `${OUT}/04-after-resize.png` });

// ─────────────────────────────── DUMP ────────────────────────────────────────
const rmLogs = await page.evaluate(() => window.__rmLogs || []);
writeFileSync(`${OUT}/rmlogs.json`, JSON.stringify(rmLogs, null, 2));
writeFileSync(`${OUT}/console.json`, JSON.stringify(consoleMsgs, null, 2));
writeFileSync(`${OUT}/obs.json`, JSON.stringify(obs, null, 2));

await browser.close();
console.log(`monitor done · serverUp:${obs.serverUp} · logs:${rmLogs.length} · out:${OUT}`);
