// #219 real-browser acceptance for cursor-anchored zoom and direct viewport pan.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import playwright from 'playwright-core';

const uiDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoDir = resolve(uiDir, '..');
const port = Number(process.env.RM_VIEWPORT_PORT || 4335);
const base = `http://127.0.0.1:${port}`;
const dataDir = await mkdtemp(resolve(tmpdir(), 'record-maker-viewport-'));
let serverLog = '';
const server = spawn('cargo', ['run', '-p', 'record-maker-server'], {
  cwd: repoDir,
  env: { ...process.env, RM_DATA_DIR: dataDir, RM_PORT: String(port) },
  stdio: ['ignore', 'pipe', 'pipe'],
});
const capture = (chunk) => { serverLog = (serverLog + chunk.toString()).slice(-24_000); };
server.stdout.on('data', capture);
server.stderr.on('data', capture);

async function request(path, options = {}) {
  const response = await fetch(`${base}${path}`, options);
  const text = await response.text();
  assert.ok(response.ok, `${options.method || 'GET'} ${path}: ${response.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

function postJson(path, value) {
  return request(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(value),
  });
}

async function waitForServer() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      await request('/layouts/all');
      return;
    } catch {}
    await new Promise((resolveWait) => setTimeout(resolveWait, 100));
  }
  throw new Error(`server did not start at ${base}`);
}

async function readZoom(page) {
  return Number((await page.locator('.le-zoom-num').textContent()).replace('%', ''));
}

async function wheelZoom(page, x, y, deltaY) {
  await page.mouse.move(x, y);
  await page.keyboard.down('Control');
  await page.mouse.wheel(0, deltaY);
  await page.keyboard.up('Control');
  await page.waitForTimeout(80);
}

async function viewport(page) {
  return page.locator('.le-stage').evaluate((stage) => ({
    left: stage.scrollLeft,
    top: stage.scrollTop,
    maxLeft: stage.scrollWidth - stage.clientWidth,
    maxTop: stage.scrollHeight - stage.clientHeight,
  }));
}

function documentGeometry(model) {
  return model.parts.flatMap((part) => part.objects.map((object) => ({
    id: object.id,
    partId: part.id,
    x: object.x,
    y: object.y,
    w: object.w,
    h: object.h,
  }))).sort((a, b) => a.id - b.id);
}

let browser;
try {
  await waitForServer();
  const table = await postJson('/schema/tables', {
    name: 'Viewport Navigation Acceptance',
    notes: '',
    fields: [{ name: 'Name', kind: 'text' }],
  });
  const layouts = await request('/layouts/all');
  const layout = layouts.find((candidate) => candidate.tableId === table.id && candidate.view === 'form');
  assert.ok(layout, 'Form layout exists');
  const initial = await request(`/design/${layout.id}/model`);
  const body = initial.parts.find((part) => part.kind === 'body');
  assert.ok(body, 'Body band exists');
  const object = (await postJson(`/design/${layout.id}/object`, {
    partId: body.id,
    kind: 'rect',
    x: 300,
    y: 350,
    w: 100,
    h: 60,
  }))[0];
  await postJson(`/design/${layout.id}/part/${body.id}/height`, { height: 1_300 });

  browser = await playwright.chromium.launch({
    headless: true,
    executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
  });
  const page = await browser.newPage({ viewport: { width: 1_000, height: 700 } });
  await page.addInitScript(() => { window.RM_LOG = false; });
  const writes = [];
  page.on('request', (event) => {
    if (event.method() === 'POST') writes.push(new URL(event.url()).pathname);
  });
  await page.goto(`${base}/design/${layout.id}`);
  await page.locator('.fm-canvas').waitFor();
  const stage = page.locator('.le-stage');
  const target = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${object.id}"]`);
  const stageBox = await stage.boundingBox();
  const anchor = {
    x: stageBox.x + stageBox.width * 0.55,
    y: stageBox.y + stageBox.height * 0.48,
  };

  // Plain scrolling is untouched; only a modifier wheel inside the canvas is
  // claimed. The same ctrl-wheel event shape is emitted by browser pinch zoom.
  const plainAllowed = await stage.evaluate((element, point) => element.dispatchEvent(new WheelEvent('wheel', {
    ...point, deltaY: 40, bubbles: true, cancelable: true,
  })), anchor);
  assert.equal(plainAllowed, true, 'ordinary wheel remains available to native scrolling');
  const outsideAllowed = await page.locator('.sidebar').evaluate((element) => element.dispatchEvent(new WheelEvent('wheel', {
    clientX: 10, clientY: 10, deltaY: 40, ctrlKey: true, bubbles: true, cancelable: true,
  })));
  assert.equal(outsideAllowed, true, 'modifier wheel outside the canvas is not hijacked');
  const zoomClaimed = await stage.evaluate((element, point) => !element.dispatchEvent(new WheelEvent('wheel', {
    ...point, deltaY: -10, ctrlKey: true, bubbles: true, cancelable: true,
  })), anchor);
  assert.equal(zoomClaimed, true, 'modifier wheel inside the canvas prevents browser zoom');
  await page.waitForTimeout(80);
  await page.getByRole('button', { name: '100%', exact: true }).click();
  await page.waitForTimeout(80);

  // Keep a non-boundary viewport and prove that one model-space point stays at
  // the cursor across a zoom update (within one screen pixel).
  await stage.evaluate((element) => element.scrollTo(100, 300));
  const beforeAnchor = await page.evaluate((point) => {
    const workspace = document.querySelector('.le-workspace');
    const readout = document.querySelector('.le-zoom-num');
    const rect = workspace.getBoundingClientRect();
    const zoom = Number(readout.textContent.replace('%', '')) / 100;
    return { modelX: (point.x - rect.left) / zoom, modelY: (point.y - rect.top) / zoom };
  }, anchor);
  await wheelZoom(page, anchor.x, anchor.y, -346.6);
  assert.equal(await readZoom(page), 200, 'modifier wheel reaches the requested 200% zoom');
  const anchoredScreen = await page.evaluate(({ point, model }) => {
    const workspace = document.querySelector('.le-workspace');
    const readout = document.querySelector('.le-zoom-num');
    const rect = workspace.getBoundingClientRect();
    const zoom = Number(readout.textContent.replace('%', '')) / 100;
    return { x: rect.left + model.modelX * zoom, y: rect.top + model.modelY * zoom, point };
  }, { point: anchor, model: beforeAnchor });
  assert.ok(Math.abs(anchoredScreen.x - anchor.x) <= 1 && Math.abs(anchoredScreen.y - anchor.y) <= 1,
    `zoom anchor remains under cursor: ${JSON.stringify(anchoredScreen)}`);

  await wheelZoom(page, anchor.x, anchor.y, 10_000);
  assert.equal(await readZoom(page), 25, 'wheel zoom clamps at 25%');
  await wheelZoom(page, anchor.x, anchor.y, -10_000);
  assert.equal(await readZoom(page), 400, 'wheel zoom clamps at 400%');

  // Commands use the same stable-center path. Fit Selection is disabled with no
  // selection, then fits and centers the selected bounds after selection.
  await page.getByRole('button', { name: '100%', exact: true }).click();
  await page.waitForTimeout(80);
  assert.equal(await readZoom(page), 100, '100% command restores actual size');
  assert.equal(await page.getByRole('button', { name: 'Fit Selection' }).isDisabled(), true,
    'Fit Selection is unavailable without a selection');
  await target.click();
  await page.waitForTimeout(80);
  const controlsBefore = await page.locator('.moveable-control-box').count();
  assert.ok(controlsBefore > 0, 'object selection owns Moveable controls before navigation');
  await page.getByRole('button', { name: 'Fit Selection' }).click();
  await page.waitForTimeout(100);
  assert.equal(await readZoom(page), 400, 'Fit Selection uses the maximum useful zoom for a small object');
  const [fitTarget, fitStage] = await Promise.all([target.boundingBox(), stage.boundingBox()]);
  assert.ok(Math.abs((fitTarget.x + fitTarget.width / 2) - (fitStage.x + fitStage.width / 2)) <= 2,
    'Fit Selection centers the selected object horizontally');
  assert.ok(Math.abs((fitTarget.y + fitTarget.height / 2) - (fitStage.y + fitStage.height / 2)) <= 2,
    'Fit Selection centers the selected object vertically');
  await page.getByRole('button', { name: 'Fit Layout' }).click();
  await page.waitForTimeout(100);
  const layoutRect = await page.locator('.fm-canvas').boundingBox();
  const fittedStage = await stage.boundingBox();
  assert.ok(layoutRect.width <= fittedStage.width - 60 && layoutRect.height <= fittedStage.height - 60,
    `Fit Layout leaves its viewport gutter: ${JSON.stringify({ layoutRect, fittedStage })}`);

  const geometryBeforePan = documentGeometry(await request(`/design/${layout.id}/model`));
  const geometryWritesBefore = writes.filter((path) => path === `/design/${layout.id}/geometry`).length;

  // Space+drag claims the pointer before object transforms, exposes grab/grabbing
  // cursors, changes only scroll offsets, and exits cleanly on Space release.
  await page.getByRole('button', { name: '100%', exact: true }).click();
  await page.waitForTimeout(80);
  await stage.evaluate((element) => element.scrollTo(0, 350));
  const beforeSpacePan = await viewport(page);
  const panStart = { x: stageBox.x + stageBox.width / 2, y: stageBox.y + stageBox.height / 2 };
  await page.keyboard.down('Space');
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-pan-ready')), true,
    'Space enters grab-ready mode');
  await page.mouse.move(panStart.x, panStart.y);
  await page.mouse.down();
  await page.mouse.move(panStart.x, panStart.y + 90, { steps: 6 });
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-panning')), true,
    'Space+drag enters grabbing mode');
  await page.keyboard.up('Space');
  const afterSpacePan = await viewport(page);
  assert.ok(afterSpacePan.top < beforeSpacePan.top - 70,
    `Space+drag pans viewport: ${JSON.stringify({ beforeSpacePan, afterSpacePan })}`);
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-panning')), false,
    'releasing Space exits pan even before pointer-up');
  await page.mouse.up();

  // Middle drag works without Space and Escape cancels capture immediately.
  await stage.evaluate((element) => element.scrollTo(0, 350));
  const beforeMiddlePan = await viewport(page);
  await page.mouse.move(panStart.x, panStart.y);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(panStart.x, panStart.y - 70, { steps: 5 });
  const afterMiddlePan = await viewport(page);
  assert.ok(afterMiddlePan.top > beforeMiddlePan.top + 50,
    `middle drag pans viewport: ${JSON.stringify({ beforeMiddlePan, afterMiddlePan })}`);
  await page.keyboard.press('Escape');
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-panning')), false,
    'Escape cancels middle-button pan');
  await page.mouse.up({ button: 'middle' });

  // Space in an editable control remains text input, never a canvas shortcut.
  const editable = page.locator('input').first();
  await editable.focus();
  await page.keyboard.down('Space');
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-pan-ready')), false,
    'Space in an input does not arm viewport pan');
  await page.keyboard.up('Space');

  assert.deepEqual(documentGeometry(await request(`/design/${layout.id}/model`)), geometryBeforePan,
    'zoom, fit, and pan never mutate authored geometry');
  assert.equal(writes.filter((path) => path === `/design/${layout.id}/geometry`).length, geometryWritesBefore,
    'viewport navigation emits no geometry persistence request');
  assert.equal(await page.locator('.moveable-control-box').count(), controlsBefore,
    'viewport navigation preserves object selection and transform ownership');

  // Hit testing and authored deltas still agree after both hand-pan paths. The
  // existing grid suite repeats this resize contract at the 25% and 400% limits.
  await target.scrollIntoViewIfNeeded();
  await target.click();
  await page.waitForTimeout(60);
  const dragBox = await target.boundingBox();
  const beforeCoordinateDrag = documentGeometry(await request(`/design/${layout.id}/model`))
    .find((candidate) => candidate.id === object.id);
  const coordinateScrollBefore = await viewport(page);
  const coordinateWrites = writes.filter((path) => path === `/design/${layout.id}/geometry`).length;
  await page.mouse.move(dragBox.x + dragBox.width / 2, dragBox.y + dragBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(dragBox.x + dragBox.width / 2 + 32, dragBox.y + dragBox.height / 2, { steps: 5 });
  await page.mouse.up();
  await page.waitForTimeout(140);
  const afterDragBox = await target.boundingBox();
  const coordinateScrollAfter = await viewport(page);
  const afterCoordinateDrag = documentGeometry(await request(`/design/${layout.id}/model`))
    .find((candidate) => candidate.id === object.id);
  assert.equal(
    afterCoordinateDrag.x - beforeCoordinateDrag.x,
    Math.round(afterDragBox.x - dragBox.x + coordinateScrollAfter.left - coordinateScrollBefore.left),
    'object hit testing maps post-pan screen plus scroll displacement to authored coordinates',
  );
  assert.equal(
    afterCoordinateDrag.y - beforeCoordinateDrag.y,
    Math.round(afterDragBox.y - dragBox.y + coordinateScrollAfter.top - coordinateScrollBefore.top),
    'post-pan vertical screen plus scroll displacement maps to authored coordinates',
  );
  assert.equal(writes.filter((path) => path === `/design/${layout.id}/geometry`).length, coordinateWrites + 1,
    'post-pan coordinate gesture persists exactly once');

  console.log('viewport navigation browser acceptance passed');
} catch (error) {
  if (serverLog) console.error(serverLog);
  throw error;
} finally {
  if (browser) await browser.close();
  if (!server.killed) server.kill('SIGTERM');
  await rm(dataDir, { recursive: true, force: true });
}
