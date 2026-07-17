// #193 browser acceptance: the layout-owned grid renders across every band and
// drives live pointer snapping at configurable 1px/coarser steps.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import playwright from 'playwright-core';

const uiDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoDir = resolve(uiDir, '..');
const port = Number(process.env.RM_GRID_PORT || 4332);
const base = `http://127.0.0.1:${port}`;
const dataDir = await mkdtemp(resolve(tmpdir(), 'record-maker-grid-'));
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

async function postJson(path, value) {
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

async function objectX(object) {
  return object.evaluate((element) => Number.parseFloat(element.style.left));
}

async function dragBy(page, object, dx, dy = 0) {
  const box = await object.boundingBox();
  assert.ok(box, 'object has a drag box');
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx, y + dy, { steps: 5 });
  await page.mouse.up();
  await page.waitForTimeout(150);
}

async function fastDragAligned(page, objectCount, dx, label) {
  const objects = page.locator('.fm-canvas .fm-obj');
  const dragTarget = objects.first();
  const fastBox = await dragTarget.boundingBox();
  assert.ok(fastBox, `${label} has a fast-drag box`);
  await page.mouse.move(fastBox.x + fastBox.width / 2, fastBox.y + fastBox.height / 2);
  await page.mouse.down();
  const sampledDrift = page.evaluate(({ count }) => new Promise((resolveDrift) => {
    let max = 0;
    let detail = null;
    let frames = 0;
    const sample = () => {
      const objectElements = [...document.querySelectorAll('.fm-canvas .fm-obj')].slice(0, count);
      const boundsElement = document.querySelector('.moveable-control-box[data-rm-drag-bounds]');
      if (objectElements.length === count && boundsElement) {
        const objectRects = objectElements.map((element) => element.getBoundingClientRect());
        const objectLeft = Math.min(...objectRects.map((rect) => rect.left));
        const objectTop = Math.min(...objectRects.map((rect) => rect.top));
        const boundsRect = boundsElement.getBoundingClientRect();
        const drift = Math.max(Math.abs(objectLeft - boundsRect.left), Math.abs(objectTop - boundsRect.top));
        if (drift > max) {
          max = drift;
          detail = { frame: frames, objectLeft, objectTop, boundsLeft: boundsRect.left, boundsTop: boundsRect.top };
        }
      }
      frames += 1;
      if (frames < 30) requestAnimationFrame(() => setTimeout(sample, 0));
      else resolveDrift({ max, detail });
    };
    requestAnimationFrame(() => setTimeout(sample, 0));
  }), { count: objectCount });
  await page.mouse.move(fastBox.x + fastBox.width / 2 + dx, fastBox.y + fastBox.height / 2, { steps: 80 });
  const fastDrift = await sampledDrift;
  const objectBoxes = await Promise.all(
    Array.from({ length: objectCount }, (_, index) => objects.nth(index).boundingBox()),
  );
  const liveBoundsBox = await page.locator('.moveable-control-box[data-rm-drag-bounds]').boundingBox();
  await page.mouse.up();
  assert.ok(fastDrift.max <= 0.5, `${label} never splits across fast-drag frames (${JSON.stringify(fastDrift)})`);
  assert.ok(objectBoxes.every(Boolean) && liveBoundsBox, `${label} and transform bounds render during fast drag`);
  const liveLeft = Math.min(...objectBoxes.map((box) => box.x));
  const liveTop = Math.min(...objectBoxes.map((box) => box.y));
  assert.ok(
    Math.abs(liveLeft - liveBoundsBox.x) <= 0.5 && Math.abs(liveTop - liveBoundsBox.y) <= 0.5,
    `${label} finishes aligned: object ${liveLeft},${liveTop}; bounds ${liveBoundsBox.x},${liveBoundsBox.y}`,
  );
}

let browser;
try {
  await waitForServer();
  const table = await postJson('/schema/tables', {
    name: 'Grid Acceptance',
    notes: '',
    fields: [{ name: 'Name', kind: 'text' }],
  });
  const layouts = await request('/layouts/all');
  const layout = layouts.find((candidate) => candidate.tableId === table.id && candidate.view === 'form');
  assert.ok(layout, 'generated Form layout exists');

  browser = await playwright.chromium.launch({
    headless: true,
    executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
  });
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const pageErrors = [];
  page.on('pageerror', (error) => pageErrors.push(error.message));
  await page.goto(`${base}/design/${layout.id}`);
  await page.locator('.fm-canvas').waitFor();

  const model = await request(`/design/${layout.id}/model`);
  assert.deepEqual(
    { gridSize: model.gridSize, showGrid: model.showGrid, snapToGrid: model.snapToGrid },
    { gridSize: 1, showGrid: true, snapToGrid: true },
  );
  const grid = page.locator('.le-layout-grid');
  await grid.waitFor();
  const gridBox = await grid.boundingBox();
  const partHeight = await page.locator('.fm-canvas .fm-part').evaluateAll((parts) =>
    parts.reduce((sum, part) => sum + part.getBoundingClientRect().height, 0));
  assert.equal(Math.round(gridBox.height), Math.round(partHeight), 'one overlay spans every band');
  assert.equal(await grid.evaluate((element) => getComputedStyle(element).backgroundSize), '10px 10px');

  // The empty canvas exposes Layout Grid; every selected band exposes the same panel.
  await page.locator('#layout-inspector input[aria-label="Layout grid size in pixels"]').waitFor();
  await page.getByTitle('Select Body band').click();
  assert.equal(await page.locator('#layout-inspector').getByText('Layout Grid', { exact: true }).count(), 1);

  // Change to a coarse 5px grid through the Inspector, then verify pointer drag.
  const size = page.locator('#layout-inspector input[aria-label="Layout grid size in pixels"]');
  const gridSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/grid` && response.request().method() === 'POST');
  await size.fill('5');
  await size.press('Tab');
  assert.equal((await gridSaved).status(), 200);
  assert.equal(await grid.evaluate((element) => getComputedStyle(element).backgroundSize), '5px 5px');

  const object = page.locator('.fm-canvas .fm-obj').first();
  const coarseBefore = await objectX(object);
  await dragBy(page, object, 13);
  const coarseAfter = await objectX(object);
  assert.notEqual(coarseAfter, coarseBefore);
  assert.equal(coarseAfter % 5, 0, 'coarse drag lands on the configured grid');

  // Empty-canvas selection exposes the same settings. Visibility and snapping
  // are independent: hiding the 5px grid removes only its chrome, while turning
  // snap off permits a three-pixel whole-number drag.
  const canvas = await page.locator('.fm-canvas').boundingBox();
  await page.mouse.click(canvas.x + canvas.width - 10, canvas.y + canvas.height - 10);
  const snapRow = page.locator('#layout-inspector .insp-row').filter({ hasText: 'Snap to grid' });
  const snapToggle = snapRow.locator('input');
  assert.equal(await snapToggle.isChecked(), true, 'snap starts enabled in the empty-canvas Inspector');
  const snapOffSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/grid` && response.request().method() === 'POST');
  await snapRow.locator('label.toggle').click();
  assert.equal((await snapOffSaved).status(), 200);
  const showRow = page.locator('#layout-inspector .insp-row').filter({ hasText: 'Show grid' });
  const hiddenSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/grid` && response.request().method() === 'POST');
  await showRow.locator('label.toggle').click();
  assert.equal((await hiddenSaved).status(), 200);
  assert.equal(await page.locator('.le-layout-grid').count(), 0, 'visibility off removes the layout grid');
  const unsnappedBefore = await objectX(object);
  await dragBy(page, object, 3);
  assert.equal(await objectX(object), unsnappedBefore + 3, 'snap off permits whole-pixel movement');

  // Restore visibility/snap from the same empty-canvas panel. A 1px grid moves
  // exactly three authored pixels instead of the old 8px staircase.
  await page.mouse.click(canvas.x + canvas.width - 10, canvas.y + canvas.height - 10);
  const restoredSnap = page.locator('#layout-inspector .insp-row').filter({ hasText: 'Snap to grid' });
  const snapOnSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/grid` && response.request().method() === 'POST');
  await restoredSnap.locator('label.toggle').click();
  assert.equal((await snapOnSaved).status(), 200);
  const restoredShow = page.locator('#layout-inspector .insp-row').filter({ hasText: 'Show grid' });
  const shownSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/grid` && response.request().method() === 'POST');
  await restoredShow.locator('label.toggle').click();
  assert.equal((await shownSaved).status(), 200);
  const fineSize = page.locator('#layout-inspector input[aria-label="Layout grid size in pixels"]');
  const fineSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/grid` && response.request().method() === 'POST');
  await fineSize.fill('1');
  await fineSize.press('Tab');
  assert.equal((await fineSaved).status(), 200);
  await page.locator('.le-layout-grid').waitFor();
  const fineBefore = await objectX(object);
  await dragBy(page, object, 3);
  assert.equal(await objectX(object), fineBefore + 3, '1px grid is effectively free-flowing');

  // Group drag snaps one anchor and applies one common delta. The generated
  // Form supplies a caption and value field, giving this check two real objects.
  const secondObject = page.locator('.fm-canvas .fm-obj').nth(1);
  assert.ok(await secondObject.count(), 'generated layout has a second object for group drag');
  await secondObject.click({ modifiers: ['Shift'] });
  const groupBefore = [await objectX(object), await objectX(secondObject)];
  await dragBy(page, object, 7);
  const groupAfter = [await objectX(object), await objectX(secondObject)];
  assert.equal(groupAfter[0] - groupBefore[0], groupAfter[1] - groupBefore[1], 'group members preserve their offset');
  await fastDragAligned(page, 2, 211, 'group object and bounds');

  await page.mouse.click(canvas.x + canvas.width - 10, canvas.y + canvas.height - 10);
  await object.click();
  await page.waitForTimeout(50);
  await fastDragAligned(page, 1, 411, 'single object and bounds');
  assert.deepEqual(pageErrors, []);

  console.log('layout grid browser acceptance passed');
} catch (error) {
  if (serverLog) console.error(serverLog);
  throw error;
} finally {
  if (browser) await browser.close();
  if (!server.killed) server.kill('SIGTERM');
  await rm(dataDir, { recursive: true, force: true });
}
