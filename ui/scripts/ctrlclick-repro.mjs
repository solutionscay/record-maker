// Deterministic Ctrl/Cmd-click multi-select toggle repro for Layout Mode.
//
// Prereqs: a dev server on $BASE (default http://127.0.0.1:4317) and Chrome at
// $CHROME. Run: cd ui && npm run build && npm run ctrlclick

import { createHarness } from './harness.mjs';

const { OUT, browser, page, obs, step, consoleMsgs, waitForCanvas, writeJson } = await createHarness({
  outDir: '.ctrlclick',
});

function assert(condition, message, detail = undefined) {
  if (!condition) {
    const suffix = detail === undefined ? '' : ` ${JSON.stringify(detail)}`;
    throw new Error(`${message}${suffix}`);
  }
}

const clearLogs = () => page.evaluate(() => { window.__rmLogs = []; });
const getLogs = () => page.evaluate(() => window.__rmLogs || []);
const latestSelection = (logs) => {
  const target = logs.findLast((entry) => entry.cat === 'target' && entry.message === 'moveable target set');
  const selectOnly = logs.findLast((entry) => entry.cat === 'select' && entry.message === 'selectOnly');
  const ids = target?.data?.selection ?? selectOnly?.data?.ids;
  return Array.isArray(ids) ? ids : [];
};
const latestToggleId = (logs) => logs.findLast((entry) => entry.cat === 'select' && entry.message === 'toggle membership')?.data?.id;
const sameIds = (a, b) => a.length === b.length && a.every((id, i) => id === b[i]);

try {
  await waitForCanvas();
  await step('canvasBox', () => page.locator('.fm-canvas').boundingBox());
  const cb = obs.canvasBox;
  assert(cb && typeof cb.x === 'number', 'canvas box was not resolved', cb);
  const initialObjectCount = await page.$$eval('.fm-canvas .fm-obj', (els) => els.length);
  obs.initialObjectCount = initialObjectCount;

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
  assert(obs.placeRectA === initialObjectCount + 1, 'rectangle placement should add one object', {
    initialObjectCount,
    afterRect: obs.placeRectA,
  });
  assert(obs.placeEllipseB === initialObjectCount + 2, 'ellipse placement should add a second object', {
    initialObjectCount,
    afterEllipse: obs.placeEllipseB,
  });
  await page.screenshot({ path: `${OUT}/01-two-objects-placed.png` });

  await step('objectBoxes', async () => {
    const handles = await page.$$('.fm-canvas .fm-obj');
    const last2 = handles.slice(-2);
    const boxes = [];
    for (const h of last2) boxes.push(await h.boundingBox());
    return boxes;
  });
  const boxes = obs.objectBoxes;
  assert(Array.isArray(boxes) && boxes.length === 2 && boxes.every(Boolean), 'new object boxes were not resolved', boxes);
  const centerOf = (b) => ({ x: b.x + b.width / 2, y: b.y + b.height / 2 });
  const A = centerOf(boxes[0]);
  const B = centerOf(boxes[1]);
  obs.centers = { A, B };

  await clearLogs();
  await page.mouse.click(A.x, A.y);
  await page.waitForTimeout(200);
  obs.plainClickA = { logs: await getLogs(), controlBoxes: await page.$$eval('.moveable-control-box', (els) => els.length) };
  const afterA = latestSelection(obs.plainClickA.logs);
  assert(afterA.length === 1, 'plain click on A should select exactly A', afterA);
  const aId = afterA[0];
  await page.screenshot({ path: `${OUT}/02-after-plain-click-A.png` });

  await clearLogs();
  await page.keyboard.down('Control');
  await page.mouse.click(B.x, B.y);
  await page.keyboard.up('Control');
  await page.waitForTimeout(200);
  obs.ctrlClickB = { logs: await getLogs(), controlBoxes: await page.$$eval('.moveable-control-box', (els) => els.length) };
  const bId = latestToggleId(obs.ctrlClickB.logs);
  const afterB = latestSelection(obs.ctrlClickB.logs);
  assert(typeof bId === 'number' && bId !== aId, 'Ctrl-click on B should toggle B into the selection', { aId, bId });
  assert(sameIds(afterB, [aId, bId]), 'Ctrl-click on B should leave A and B selected', afterB);
  await page.screenshot({ path: `${OUT}/03-after-ctrl-click-B.png` });

  await clearLogs();
  await page.keyboard.down('Meta');
  await page.mouse.click(A.x, A.y);
  await page.keyboard.up('Meta');
  await page.waitForTimeout(200);
  obs.metaClickA_toggleOff = { logs: await getLogs(), controlBoxes: await page.$$eval('.moveable-control-box', (els) => els.length) };
  const toggledOff = latestToggleId(obs.metaClickA_toggleOff.logs);
  const afterMeta = latestSelection(obs.metaClickA_toggleOff.logs);
  assert(toggledOff === aId, 'Meta-click on A should toggle A off', { aId, toggledOff });
  assert(sameIds(afterMeta, [bId]), 'Meta-click on A should leave only B selected', afterMeta);
  await page.screenshot({ path: `${OUT}/04-after-meta-click-A-toggle-off.png` });

  obs.assertions = { aId, bId, passed: true };
} finally {
  const rmLogsFinal = await getLogs().catch(() => []);
  writeJson('rmlogs-last-step.json', rmLogsFinal);
  writeJson('console.json', consoleMsgs);
  writeJson('obs.json', obs);
  await browser.close();
}

console.log(`ctrlclick repro passed · serverUp:${obs.serverUp} · out:${OUT}`);
console.log(JSON.stringify(obs.assertions, null, 2));
