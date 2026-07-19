// #193/#194/#195 browser acceptance: layout-owned snapping plus frame-sampled
// object/bounds alignment and pointer-to-style commit latency under burst drag.

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

async function dragBy(page, object, dx, dy = 0, grabY = 0.5) {
  const box = await object.boundingBox();
  assert.ok(box, 'object has a drag box');
  const x = box.x + box.width / 2;
  const y = box.y + box.height * grabY;
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
  const sampledDrift = page.evaluate(({ count, pointerX, grabOffsetX }) => new Promise((resolveDrift) => {
    let maxBounds = 0;
    let boundsDetail = null;
    let activationBounds = null;
    let maxCursor = 0;
    let cursorDetail = null;
    let frames = 0;
    let latestPointerX = pointerX;
    let latestPointerAt = performance.now();
    let pointerSequence = 0;
    let sampledSequence = -1;
    let committedSequence = -1;
    const commitLatencies = [];
    const observedObject = document.querySelector('.fm-canvas .fm-obj');
    const observer = new MutationObserver(() => {
      if (pointerSequence === committedSequence) return;
      committedSequence = pointerSequence;
      commitLatencies.push(performance.now() - latestPointerAt);
    });
    if (observedObject) observer.observe(observedObject, { attributes: true, attributeFilter: ['style'] });
    const onPointerMove = (event) => {
      latestPointerX = event.clientX;
      latestPointerAt = performance.now();
      pointerSequence += 1;
    };
    window.addEventListener('pointermove', onPointerMove, { capture: true });
    const sample = () => {
      const objectElements = [...document.querySelectorAll('.fm-canvas .fm-obj')].slice(0, count);
      const boundsElement = document.querySelector('.moveable-control-box[data-rm-drag-bounds]');
      if (objectElements.length === count && boundsElement) {
        const objectRects = objectElements.map((element) => element.getBoundingClientRect());
        const objectLeft = Math.min(...objectRects.map((rect) => rect.left));
        const objectTop = Math.min(...objectRects.map((rect) => rect.top));
        const boundsRect = boundsElement.getBoundingClientRect();
        const drift = Math.max(Math.abs(objectLeft - boundsRect.left), Math.abs(objectTop - boundsRect.top));
        if (activationBounds === null) {
          activationBounds = drift;
        } else if (drift > maxBounds) {
          maxBounds = drift;
          boundsDetail = { frame: frames, objectLeft, objectTop, boundsLeft: boundsRect.left, boundsTop: boundsRect.top };
        }
        if (pointerSequence !== sampledSequence) {
          sampledSequence = pointerSequence;
          const expectedLeft = latestPointerX - grabOffsetX;
          const cursorLag = expectedLeft - objectLeft;
          if (Math.abs(cursorLag) > Math.abs(maxCursor)) {
            maxCursor = cursorLag;
            cursorDetail = { frame: frames, latestPointerX, expectedLeft, objectLeft, cursorLag };
          }
        }
      }
      frames += 1;
      if (frames < 30) requestAnimationFrame(() => setTimeout(sample, 0));
      else {
        window.removeEventListener('pointermove', onPointerMove, { capture: true });
        observer.disconnect();
        const sortedLatencies = commitLatencies.toSorted((a, b) => a - b);
        resolveDrift({
          maxBounds,
          boundsDetail,
          activationBounds: activationBounds ?? 0,
          maxCursor,
          cursorDetail,
          commitSamples: sortedLatencies.length,
          commitMedianMs: sortedLatencies[Math.floor(sortedLatencies.length / 2)] ?? 0,
          commitP95Ms: sortedLatencies[Math.floor(sortedLatencies.length * 0.95)] ?? 0,
          commitMaxMs: sortedLatencies.at(-1) ?? 0,
        });
      }
    };
    requestAnimationFrame(() => setTimeout(sample, 0));
  }), { count: objectCount, pointerX: fastBox.x + fastBox.width / 2, grabOffsetX: fastBox.width / 2 });
  // Install the pointer timestamp probe before pointer-down, so it runs before
  // the editor's drag-start listener registers its compositor feedback path.
  await page.mouse.down();
  await page.mouse.move(fastBox.x + fastBox.width / 2 + dx, fastBox.y + fastBox.height / 2, { steps: 80 });
  const fastDrift = await sampledDrift;
  const objectBoxes = await Promise.all(
    Array.from({ length: objectCount }, (_, index) => objects.nth(index).boundingBox()),
  );
  const feedbackStyles = await objects.evaluateAll((elements, count) => elements.slice(0, count).map((element) => ({
    left: element.style.left,
    transform: element.style.transform,
    willChange: element.style.willChange,
  })), objectCount);
  const liveBoundsBox = await page.locator('.moveable-control-box[data-rm-drag-bounds]').boundingBox();
  await page.mouse.up();
  assert.ok(fastDrift.maxBounds <= 0.5, `${label} never splits across fast-drag frames (${JSON.stringify(fastDrift)})`);
  assert.ok(fastDrift.activationBounds <= 3.01, `${label} activates within one small frame tolerance (${JSON.stringify(fastDrift)})`);
  assert.ok(Math.abs(fastDrift.maxCursor) <= 1.01, `${label} stays within whole-pixel cursor geometry (${JSON.stringify(fastDrift)})`);
  assert.ok(objectBoxes.every(Boolean) && liveBoundsBox, `${label} and transform bounds render during fast drag`);
  assert.ok(
    feedbackStyles.every((style) => style.transform.startsWith('translate3d(') && style.willChange === 'transform'),
    `${label} uses compositor-only feedback until pointer-up (${JSON.stringify(feedbackStyles)})`,
  );
  assert.ok(
    await objects.evaluateAll((elements, count) => elements.slice(0, count).every((element) => !element.style.transform), objectCount),
    `${label} clears temporary transforms after authored geometry commits`,
  );
  const liveLeft = Math.min(...objectBoxes.map((box) => box.x));
  const liveTop = Math.min(...objectBoxes.map((box) => box.y));
  assert.ok(
    Math.abs(liveLeft - liveBoundsBox.x) <= 0.5 && Math.abs(liveTop - liveBoundsBox.y) <= 0.5,
    `${label} finishes aligned: object ${liveLeft},${liveTop}; bounds ${liveBoundsBox.x},${liveBoundsBox.y}`,
  );
  console.log(`${label} latency: ${JSON.stringify(fastDrift)}`);
}

let browser;
try {
  await waitForServer();
  const table = await postJson('/schema/tables', {
    name: 'Grid Acceptance',
    notes: '',
    fields: [{ name: 'Name', kind: 'text' }],
  });
  const portalRows = await postJson('/schema/tables', {
    name: 'Grid Portal Rows',
    notes: '',
    fields: [
      { name: 'Amount', kind: 'number' },
      { name: 'Parent Id', kind: 'text' },
    ],
  });
  const baseFields = await request(`/schema/tables/${table.id}/fields`);
  const relatedFields = await request(`/schema/tables/${portalRows.id}/fields`);
  const relationship = await postJson('/schema/relationships', {
    name: 'grid_rows',
    fromTable: portalRows.id,
    toTable: table.id,
    fromField: relatedFields.find((field) => field.name === 'Parent Id').id,
    toField: baseFields.find((field) => field.options?.system).id,
  });
  assert.ok(relationship.id, 'portal acceptance relationship exists');
  const layouts = await request('/layouts/all');
  const layout = layouts.find((candidate) => candidate.tableId === table.id && candidate.view === 'form');
  assert.ok(layout, 'generated Form layout exists');
  const initialModel = await request(`/design/${layout.id}/model`);
  const headerPart = initialModel.parts.find((part) => part.kind === 'header');
  const bodyPart = initialModel.parts.find((part) => part.kind === 'body');
  const createdPortal = await postJson(`/design/${layout.id}/object`, {
    partId: bodyPart.id,
    kind: 'portal',
    x: 340,
    y: 48,
    w: 280,
    h: 24,
    binding: 'grid_rows',
  });
  const portalId = createdPortal[0].id;
  const createdColumn = await postJson(`/design/${layout.id}/object`, {
    partId: bodyPart.id,
    kind: 'field',
    x: 360,
    y: 48,
    w: 80,
    h: 24,
    fieldId: relatedFields.find((field) => field.name === 'Amount').id,
    createLabel: true,
    parentObjectId: portalId,
  });
  const portalChildIds = createdColumn.map((object) => object.id);

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
  const portalModelObjects = model.parts.flatMap((part) => part.objects);
  assert.ok(portalChildIds.every((id) =>
    portalModelObjects.find((object) => object.id === id)?.parentObjectId === portalId),
  'portal acceptance children retain explicit ownership in the design model');
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

  // #203: a portal's owned header + field move implicitly while the portal stays
  // the sole visible selection. Pointer drag, keyboard nudge, cross-band settle,
  // and reload persistence all operate on the same expanded movement set.
  const portal = page.locator(`[data-object-id="${portalId}"]`);
  const portalChildren = portalChildIds.map((id) => page.locator(`[data-object-id="${id}"]`));
  const beforePortalDrag = await Promise.all([portal, ...portalChildren].map((locator) => locator.boundingBox()));
  await dragBy(page, portal, 17, 9);
  const afterPortalDrag = await Promise.all([portal, ...portalChildren].map((locator) => locator.boundingBox()));
  const portalDx = afterPortalDrag[0].x - beforePortalDrag[0].x;
  const portalDy = afterPortalDrag[0].y - beforePortalDrag[0].y;
  assert.ok(afterPortalDrag.slice(1).every((box, index) =>
    Math.abs((box.x - beforePortalDrag[index + 1].x) - portalDx) <= 0.5 &&
    Math.abs((box.y - beforePortalDrag[index + 1].y) - portalDy) <= 0.5),
  `portal pointer drag applies one common delta to owned children: ${JSON.stringify({ beforePortalDrag, afterPortalDrag, portalDx, portalDy })}`);

  const beforeNudge = await Promise.all([portal, ...portalChildren].map(objectX));
  const nudgeSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/geometry` && response.request().method() === 'POST');
  await page.keyboard.press('ArrowRight');
  assert.equal((await nudgeSaved).status(), 200);
  const afterNudge = await Promise.all([portal, ...portalChildren].map(objectX));
  assert.ok(afterNudge.every((x, index) => x === beforeNudge[index] + 1), 'portal keyboard nudge moves owned children once');

  const beforeBandDrag = await Promise.all([portal, ...portalChildren].map((locator) => locator.boundingBox()));
  const portalBox = beforeBandDrag[0];
  const destinationBox = await page.locator(`[data-part-id="${headerPart.id}"]`).boundingBox();
  // Land the portal top at 25px: its 24px-high header remains inside the band,
  // and the interior grab point remains inside the canvas.
  await dragBy(page, portal, 0, destinationBox.y + 25 - portalBox.y, 0.35);
  const afterBandDrag = await Promise.all([portal, ...portalChildren].map((locator) => locator.boundingBox()));
  const bandDx = afterBandDrag[0].x - beforeBandDrag[0].x;
  const bandDy = afterBandDrag[0].y - beforeBandDrag[0].y;
  assert.ok(afterBandDrag.slice(1).every((box, index) =>
    Math.abs((box.x - beforeBandDrag[index + 1].x) - bandDx) <= 0.5 &&
    Math.abs((box.y - beforeBandDrag[index + 1].y) - bandDy) <= 0.5),
  `cross-band portal drag preserves child offsets: ${JSON.stringify({ beforeBandDrag, afterBandDrag, bandDx, bandDy, portalBox, destinationBox })}`);
  const settledPartIds = await Promise.all([portal, ...portalChildren].map((locator) =>
    locator.evaluate((element) => Number(element.closest('.fm-part')?.getAttribute('data-part-id')))));
  assert.ok(settledPartIds.every((partId) => partId === headerPart.id),
    `portal and children settle into the same destination band: ${JSON.stringify({ settledPartIds, destinationPart: headerPart.id, portalBox, destinationBox, afterBandDrag })}`);

  await page.waitForTimeout(300);
  await page.reload();
  await page.locator('.fm-canvas').waitFor();
  assert.ok(await Promise.all([portal, ...portalChildren].map((locator) =>
    locator.evaluate((element, partId) => Number(element.closest('.fm-part')?.getAttribute('data-part-id')) === partId, headerPart.id)))
    .then((values) => values.every(Boolean)), 'portal child movement persists across reload');
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
