// Standalone repro for Ctrl/Cmd-click multi-select toggle bug (Layout Mode canvas).
// Follows the same pattern as ui/scripts/monitor.mjs.
//
// Run: BASE=http://127.0.0.1:4317 CHROME=/usr/bin/google-chrome-stable node ctrlclick-repro.mjs

import { chromium } from 'playwright-core';
import { mkdirSync, writeFileSync } from 'node:fs';

const OUT = process.env.MONITOR_OUT || '/tmp/claude-1000/-media-jose-DISK2-GitHub-record-maker/89e75169-ea75-484a-a002-532be4538757/scratchpad/ctrlclick-out';
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

await step('canvasBox', () => page.locator('.fm-canvas').boundingBox());
const cb = obs.canvasBox;

// Clear the log buffer helper.
const clearLogs = () => page.evaluate(() => { window.__rmLogs = []; });
const getLogs = () => page.evaluate(() => window.__rmLogs || []);
const selectionSnapshot = () =>
  page.$$eval('.fm-canvas .fm-obj', (els) =>
    els.map((e) => ({ selected: e.classList.contains('selected') || e.getAttribute('data-selected'), style: e.getAttribute('style') })),
  );

// Place object A (rectangle) and object B (ellipse) at distinct, non-overlapping spots.
await step('placeRectA', async () => {
  await page.click('#layout-tools button[title="Rectangle"]');
  await page.waitForTimeout(100);
  await page.mouse.click(cb.x + 120, cb.y + 80);
  await page.waitForTimeout(400);
  return page.$$eval('.fm-canvas .fm-obj', (els) => els.length);
});
await step('placeEllipseB', async () => {
  await page.click('#layout-tools button[title="Ellipse"]');
  await page.waitForTimeout(100);
  await page.mouse.click(cb.x + 400, cb.y + 80);
  await page.waitForTimeout(400);
  return page.$$eval('.fm-canvas .fm-obj', (els) => els.length);
});
await page.screenshot({ path: `${OUT}/01-two-objects-placed.png` });

// Get OUR two newly-placed object elements (the LAST two in DOM/paint order —
// the layout already has 26 pre-existing field objects from seed data, so
// boxes[0]/[1] would be the wrong, pre-existing objects).
await step('objectBoxes', async () => {
  const handles = await page.$$('.fm-canvas .fm-obj');
  const last2 = handles.slice(-2);
  const boxes = [];
  for (const h of last2) boxes.push(await h.boundingBox());
  return boxes;
});
const boxes = obs.objectBoxes;
const centerOf = (b) => ({ x: b.x + b.width / 2, y: b.y + b.height / 2 });
const A = centerOf(boxes[0]);
const B = centerOf(boxes[1]);
obs.centers = { A, B };

// STEP 1: plain click on A -> selects only A.
await clearLogs();
await step('plainClickA', async () => {
  await page.mouse.click(A.x, A.y);
  await page.waitForTimeout(200);
  return {
    logs: await getLogs(),
    controlBoxes: await page.$$eval('.moveable-control-box', (els) => els.length),
  };
});
await page.screenshot({ path: `${OUT}/02-after-plain-click-A.png` });

// STEP 2: Ctrl+click on B -> should ADD B to selection (toggle), giving 2 selected.
// NOTE: page.mouse.click() has NO `modifiers` option (that belongs to
// locator/page.click()) — holding the key via keyboard.down/up around the raw
// mouse click is the only way to get a real ctrlKey:true on the native event.
await clearLogs();
await step('ctrlClickB', async () => {
  await page.keyboard.down('Control');
  await page.mouse.click(B.x, B.y);
  await page.keyboard.up('Control');
  await page.waitForTimeout(200);
  return {
    logs: await getLogs(),
    controlBoxes: await page.$$eval('.moveable-control-box', (els) => els.length),
  };
});
await page.screenshot({ path: `${OUT}/03-after-ctrl-click-B.png` });

// Read the actual store selection size via a debug hook if available, else infer
// from moveable's target elements/control boxes.
await step('moveableControlBoxesFinal', () => page.$$eval('.moveable-control-box', (els) => els.length));
await step('moveableTargetRects', () =>
  page.$$eval('.moveable-control-box .moveable-line, .moveable-control-box', (els) =>
    els.map((e) => e.getBoundingClientRect().toJSON()),
  ),
);

// STEP 3: Cmd (Meta) click on A again while B+A selected -> should toggle A OFF, leaving only B.
await clearLogs();
await step('metaClickA_toggleOff', async () => {
  await page.keyboard.down('Meta');
  await page.mouse.click(A.x, A.y);
  await page.keyboard.up('Meta');
  await page.waitForTimeout(200);
  return {
    logs: await getLogs(),
    controlBoxes: await page.$$eval('.moveable-control-box', (els) => els.length),
  };
});
await page.screenshot({ path: `${OUT}/04-after-meta-click-A-toggle-off.png` });

const rmLogsFinal = await getLogs();
writeFileSync(`${OUT}/rmlogs-last-step.json`, JSON.stringify(rmLogsFinal, null, 2));
writeFileSync(`${OUT}/console.json`, JSON.stringify(consoleMsgs, null, 2));
writeFileSync(`${OUT}/obs.json`, JSON.stringify(obs, null, 2));

await browser.close();
console.log(`repro done · serverUp:${obs.serverUp} · out:${OUT}`);
console.log(JSON.stringify(obs, null, 2));
