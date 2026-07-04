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

import { createHarness } from './harness.mjs';

const { OUT, browser, page, obs, step, consoleMsgs, waitForCanvas, writeJson } = await createHarness();

// readiness: retry until the design page + canvas are up
await waitForCanvas();

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
await step('outsideCanvasClickWhileArmed', async () => {
  const before = await page.$$eval('.fm-canvas .fm-obj', (els) => els.length);
  await page.mouse.click(cb.x + cb.width + 260, cb.y + 120);
  await page.waitForTimeout(250);
  const after = await page.$$eval('.fm-canvas .fm-obj', (els) => els.length);
  const active = await page.$eval('#layout-tools button[title="Ellipse"]', (e) => e.classList.contains('active'));
  return { before, after, active };
});
const clickRel = { x: 620, y: 60 };
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

// Add a band, select it, resize its bottom edge, change its kind, then delete it.
await step('addBand', async () => {
  await page.click('#layout-tools button[title="Add band"]');
  await page.waitForTimeout(300);
  return page.$$eval('.fm-canvas .fm-part', (els) => els.length);
});
await step('partsAfterAdd', () =>
  page.$$eval('.fm-canvas .fm-part', (els) =>
    els.map((e) => {
      const r = e.getBoundingClientRect();
      return { height: e.style.height, rectH: Math.round(r.height) };
    }),
  ),
);
await step('bandInspectorAfterAdd', () =>
  page.$eval('#layout-tools .le-danger-btn', (e) => ({ disabled: e.disabled, title: e.title })),
);
await page.screenshot({ path: `${OUT}/05-after-band-add.png` });

await step('resizeBand', async () => {
  const handles = await page.$$('.le-part-resize');
  const h = handles.at(-1);
  if (!h) return { handleFound: false };
  const hb = await h.boundingBox();
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2 + 42, { steps: 5 });
  await page.mouse.up();
  await page.waitForTimeout(350);
  return { handleFound: true, handleBox: hb };
});
await step('partsAfterBandResize', () =>
  page.$$eval('.fm-canvas .fm-part', (els) =>
    els.map((e) => {
      const r = e.getBoundingClientRect();
      return { height: e.style.height, rectH: Math.round(r.height) };
    }),
  ),
);

await step('setBandKind', async () => {
  await page.$eval('#layout-tools .le-compact-select', (select) => {
    select.value = 'footer';
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await page.waitForTimeout(250);
  return page.$eval('#layout-tools .le-compact-select', (select) => select.value);
});
await step('deleteBand', async () => {
  await page.click('#layout-tools button[title="Delete selected band"]');
  await page.waitForTimeout(350);
  return page.$$eval('.fm-canvas .fm-part', (els) => els.length);
});
await page.screenshot({ path: `${OUT}/06-after-band-delete.png` });

// Zoomed placement + transform: CSS scale must not distort model coordinates.
await step('zoomIn', async () => {
  await page.click('#layout-tools button[title="Zoom in"]');
  await page.click('#layout-tools button[title="Zoom in"]');
  await page.waitForTimeout(150);
  return page.$eval('#layout-tools .le-zoom-num', (e) => e.textContent);
});
await step('canvasBoxZoomed', () => page.locator('.fm-canvas').boundingBox());
const zbox = obs.canvasBoxZoomed;
const zoomClickRel = { x: 230, y: 100 };
obs.zoomClickRel = zoomClickRel;
await step('placeRectZoomed', async () => {
  await page.click('#layout-tools button[title="Rectangle"]');
  await page.waitForTimeout(100);
  await page.mouse.click(zbox.x + zoomClickRel.x * 1.2, zbox.y + zoomClickRel.y * 1.2);
  await page.waitForTimeout(500);
  return page.$$eval('.fm-canvas .fm-obj', (els) => els.at(-1)?.getAttribute('style'));
});
await step('dragRectZoomed', async () => {
  const obj = (await page.$$('.fm-canvas .fm-obj')).at(-1);
  if (!obj) return { objectFound: false };
  const b = await obj.boundingBox();
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width / 2 + 48, b.y + b.height / 2 + 24, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(350);
  return page.$$eval('.fm-canvas .fm-obj', (els) => els.at(-1)?.getAttribute('style'));
});
await step('resizeRectZoomed', async () => {
  const h = await page.$('.moveable-control.moveable-se');
  if (!h) return { handleFound: false };
  const hb = await h.boundingBox();
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  await page.mouse.move(hb.x + hb.width / 2 + 48, hb.y + hb.height / 2 + 36, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(350);
  return {
    handleFound: true,
    style: await page.$$eval('.fm-canvas .fm-obj', (els) => els.at(-1)?.getAttribute('style')),
  };
});
await page.screenshot({ path: `${OUT}/07-after-zoom-transform.png` });

// ─────────────────────────────── DUMP ────────────────────────────────────────
const rmLogs = await page.evaluate(() => window.__rmLogs || []);
writeJson('rmlogs.json', rmLogs);
writeJson('console.json', consoleMsgs);
writeJson('obs.json', obs);

await browser.close();
console.log(`monitor done · serverUp:${obs.serverUp} · logs:${rmLogs.length} · out:${OUT}`);
