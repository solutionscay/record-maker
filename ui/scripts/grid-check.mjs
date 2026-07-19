// #193/#194/#195/#214 browser acceptance: layout-owned snapping plus
// frame-sampled drag/resize alignment, ephemeral preview latency, and one final
// authored geometry commit under burst input.

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

async function shiftDragBy(page, object, dx, dy = 0) {
  const box = await object.boundingBox();
  assert.ok(box, 'modifier-drag object has a drag box');
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.keyboard.down('Shift');
  await page.mouse.down();
  await page.mouse.move(x + dx, y + dy, { steps: 5 });
  await page.mouse.up();
  await page.keyboard.up('Shift');
  await page.waitForTimeout(150);
}

async function shiftClick(page, object) {
  const box = await object.boundingBox();
  assert.ok(box, 'modifier-click object has a click box');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.keyboard.down('Shift');
  await page.mouse.down();
  await page.mouse.up();
  await page.keyboard.up('Shift');
  await page.waitForTimeout(100);
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
  assert.ok(Math.abs(fastDrift.maxCursor) <= 5.51,
    `${label} stays within the five-pixel smart-snap threshold (${JSON.stringify(fastDrift)})`);
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

async function beginFastResizePreview(page, object, handle, dx, dy, label, edgeObjects = [object]) {
  const objectId = await object.getAttribute('data-object-id');
  const objectSelector = `[data-object-id="${objectId}"]`;
  const edgeSelectors = await Promise.all(edgeObjects.map(async (locator) =>
    `[data-object-id="${await locator.getAttribute('data-object-id')}"]:not(.le-echo-ghost)`));
  const handleBox = await handle.boundingBox();
  assert.ok(handleBox, `${label} has a resize handle`);
  const widthInput = page.locator('#layout-inspector input[aria-label="Width in pixels"]');
  const heightInput = page.locator('#layout-inspector input[aria-label="Height in pixels"]');
  const inspectorBefore = { width: await widthInput.inputValue(), height: await heightInput.inputValue() };
  await page.mouse.move(handleBox.x + handleBox.width / 2, handleBox.y + handleBox.height / 2);
  const sampled = page.evaluate(({ selector, edgeSelectors, inspectorBefore }) => new Promise((resolveSample) => {
    let pointerSequence = 0;
    let previousFrameSequence = -1;
    let latestPointerAt = performance.now();
    let observedSequence = -1;
    let maxStableEdgeGap = 0;
    let reactiveChanges = 0;
    let frames = 0;
    const styleLatencies = [];
    const target = document.querySelector(selector);
    const width = document.querySelector('#layout-inspector input[aria-label="Width in pixels"]');
    const height = document.querySelector('#layout-inspector input[aria-label="Height in pixels"]');
    const observer = new MutationObserver(() => {
      if (observedSequence === pointerSequence) return;
      observedSequence = pointerSequence;
      styleLatencies.push(performance.now() - latestPointerAt);
    });
    if (target) observer.observe(target, { attributes: true, attributeFilter: ['style'] });
    const onPointerMove = () => {
      pointerSequence += 1;
      latestPointerAt = performance.now();
    };
    window.addEventListener('pointermove', onPointerMove, { capture: true });
    const sample = () => {
      const controls = [...document.querySelectorAll('.moveable-control[data-direction="se"]')];
      const control = controls.at(-1);
      if (target && control && pointerSequence > 0 && pointerSequence === previousFrameSequence) {
        const boxes = edgeSelectors
          .map((edgeSelector) => document.querySelector(edgeSelector)?.getBoundingClientRect())
          .filter(Boolean);
        const handleBox = control.getBoundingClientRect();
        if (boxes.length > 0) {
          maxStableEdgeGap = Math.max(
            maxStableEdgeGap,
            Math.abs(Math.max(...boxes.map((box) => box.right)) - (handleBox.left + handleBox.width / 2)),
            Math.abs(Math.max(...boxes.map((box) => box.bottom)) - (handleBox.top + handleBox.height / 2)),
          );
        }
      }
      if ((width && width.value !== inspectorBefore.width) || (height && height.value !== inspectorBefore.height)) {
        reactiveChanges += 1;
      }
      previousFrameSequence = pointerSequence;
      frames += 1;
      if (frames < 35) requestAnimationFrame(sample);
      else {
        observer.disconnect();
        window.removeEventListener('pointermove', onPointerMove, { capture: true });
        resolveSample({
          maxStableEdgeGap,
          reactiveChanges,
          styleSamples: styleLatencies.length,
          maxStyleLatencyMs: styleLatencies.length ? Math.max(...styleLatencies) : 0,
        });
      }
    };
    requestAnimationFrame(sample);
  }), { selector: objectSelector, edgeSelectors, inspectorBefore });
  await page.mouse.down();
  await page.mouse.move(
    handleBox.x + handleBox.width / 2 + dx,
    handleBox.y + handleBox.height / 2 + dy,
    { steps: 80 },
  );
  const result = await sampled;
  assert.equal(result.reactiveChanges, 0,
    `${label} leaves the reactive document/Inspector unchanged during preview (${JSON.stringify(result)})`);
  assert.ok(result.styleSamples > 0 && result.maxStyleLatencyMs <= 34.5,
    `${label} paints feedback within one display frame (${JSON.stringify(result)})`);
  assert.ok(result.maxStableEdgeGap <= 1.01,
    `${label} keeps the southeast handle attached after one display frame (${JSON.stringify(result)})`);
  assert.deepEqual(
    { width: await widthInput.inputValue(), height: await heightInput.inputValue() },
    inspectorBefore,
    `${label} has no authored geometry before pointer-up`,
  );
  console.log(`${label} resize preview: ${JSON.stringify(result)}`);
  return { inspectorBefore, result };
}

async function setCanvasZoom(page, percent) {
  const readout = page.locator('.le-zoom-num');
  for (let attempts = 0; attempts < 50; attempts += 1) {
    const current = Number((await readout.textContent()).replace('%', ''));
    if (current === percent) return;
    await page.getByRole('button', { name: current < percent ? 'Zoom in' : 'Zoom out' }).click();
  }
  assert.fail(`could not set canvas zoom to ${percent}%`);
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
  const createdModifierTarget = await postJson(`/design/${layout.id}/object`, {
    partId: bodyPart.id,
    kind: 'rect',
    x: 570,
    y: 96,
    w: 60,
    h: 40,
  });
  const modifierTargetId = createdModifierTarget[0].id;
  const createdLine = await postJson(`/design/${layout.id}/object`, {
    partId: bodyPart.id,
    kind: 'line',
    x: 300,
    y: 130,
    w: 100,
    h: 2,
    props: { stroke: '#888888', strokeWidth: 2, angle: 0, length: 100 },
  });
  const lineId = createdLine[0].id;
  const createdSnapSource = await postJson(`/design/${layout.id}/object`, {
    partId: bodyPart.id,
    kind: 'rect',
    x: 520,
    y: 96,
    w: 40,
    h: 20,
  });
  const snapSourceId = createdSnapSource[0].id;
  const createdSnapCandidate = await postJson(`/design/${layout.id}/object`, {
    partId: bodyPart.id,
    kind: 'rect',
    x: 700,
    y: 96,
    w: 40,
    h: 20,
  });
  const snapCandidateId = createdSnapCandidate[0].id;
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
  const writeRequests = [];
  page.on('pageerror', (error) => pageErrors.push(error.message));
  page.on('request', (request) => {
    if (request.method() === 'POST') writeRequests.push(new URL(request.url()).pathname);
  });
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

  // #216: sibling geometry is resolved numerically before paint. The source's
  // right edge enters the candidate's left-edge threshold, so both the live box
  // and the only active vertical guide land at model x=700; pointer-up persists
  // that exact live result and removes the guide.
  const snapSource = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${snapSourceId}"]`);
  const snapCandidate = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${snapCandidateId}"]`);
  const snapSourceBox = await snapSource.boundingBox();
  const snapCandidateBox = await snapCandidate.boundingBox();
  const smartSnapWrites = writeRequests.length;
  await page.mouse.move(snapSourceBox.x + snapSourceBox.width / 2, snapSourceBox.y + snapSourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(snapSourceBox.x + snapSourceBox.width / 2 + 136, snapSourceBox.y + snapSourceBox.height / 2, { steps: 5 });
  await page.waitForTimeout(50);
  const liveSnapBox = await snapSource.boundingBox();
  assert.ok(Math.abs(liveSnapBox.x + liveSnapBox.width - snapCandidateBox.x) <= 0.5,
    `live drag geometry adheres to the candidate edge: ${JSON.stringify({ snapSourceBox, snapCandidateBox, liveSnapBox })}`);
  assert.equal(await page.locator('.le-smart-guide-x').count(), 1, 'one applied vertical guide is visible');
  assert.equal(await page.locator('.le-smart-guide-x').evaluate((element) => Number.parseFloat(element.style.left)), 700,
    'guide chrome uses the same resolved model coordinate');
  await page.mouse.up();
  await page.waitForTimeout(150);
  assert.ok(writeRequests.slice(smartSnapWrites).some((path) =>
    path === `/design/${layout.id}/geometry` || path === `/design/${layout.id}/object/${snapSourceId}/part`),
    `smart snap persists geometry: ${JSON.stringify(writeRequests.slice(smartSnapWrites))}`);
  assert.equal(await objectX(snapSource), 660, 'pointer-up persists the live smart-snapped coordinate');
  assert.equal(await page.locator('.le-smart-guide').count(), 0, 'smart guides clear on pointer-up');
  await dragBy(page, snapSource, -140);
  assert.equal(await objectX(snapSource), 520, 'source resets for resize snapping acceptance');

  const outsideThresholdBox = await snapSource.boundingBox();
  await page.mouse.move(
    outsideThresholdBox.x + outsideThresholdBox.width / 2,
    outsideThresholdBox.y + outsideThresholdBox.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    outsideThresholdBox.x + outsideThresholdBox.width / 2 + 130,
    outsideThresholdBox.y + outsideThresholdBox.height / 2,
    { steps: 4 },
  );
  await page.waitForTimeout(50);
  assert.equal(await page.locator('.le-smart-guide-x').count(), 0,
    'leaving the five-pixel threshold releases the vertical snap and guide');
  const outsideThresholdLiveBox = await snapSource.boundingBox();
  assert.equal(Math.round(outsideThresholdLiveBox.x + outsideThresholdLiveBox.width - snapCandidateBox.x), -10,
    'outside-threshold live geometry follows raw/grid intent');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.waitForTimeout(50);
  assert.equal(await objectX(snapSource), 520, 'threshold probe cancellation restores the source');

  await snapSource.dispatchEvent('click');
  await page.waitForTimeout(100);
  const smartResizeHandle = page.locator('.moveable-control[data-direction="se"]').last();
  const smartResizeHandleBox = await smartResizeHandle.boundingBox();
  assert.ok(smartResizeHandleBox, 'smart-snap source exposes a southeast resize handle');
  const smartResizeSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/geometry` && response.request().method() === 'POST');
  const singleResizeWrites = writeRequests.filter((path) => path === `/design/${layout.id}/geometry`).length;
  await beginFastResizePreview(page, snapSource, smartResizeHandle, 136, 0, 'single object');
  await page.waitForTimeout(50);
  assert.equal(await snapSource.evaluate((element) => Number.parseFloat(element.style.width)), 180,
    'live resize edge adheres to the sibling guide');
  assert.equal(await page.locator('.le-smart-guide-x').evaluate((element) => Number.parseFloat(element.style.left)), 700,
    'resize guide and applied edge share one coordinate');
  await page.mouse.up();
  assert.equal((await smartResizeSaved).status(), 200);
  assert.equal(writeRequests.filter((path) => path === `/design/${layout.id}/geometry`).length, singleResizeWrites + 1,
    '80-sample single resize emits exactly one final bulk geometry request');
  assert.equal(await snapSource.evaluate((element) => Number.parseFloat(element.style.width)), 180,
    'pointer-up persists the live smart-snapped width');
  assert.equal(await page.locator('.le-smart-guide').count(), 0, 'resize guides clear on pointer-up');
  assert.equal(await page.locator('#layout-inspector input[aria-label="Width in pixels"]').inputValue(), '180',
    'single resize publishes final authored width only after pointer-up');

  await page.keyboard.press('Control+z');
  await page.waitForTimeout(150);
  assert.equal(await snapSource.evaluate((element) => Number.parseFloat(element.style.width)), 40,
    'one undo restores the exact pre-resize width');
  await page.keyboard.press('Control+Shift+z');
  await page.waitForTimeout(150);
  assert.equal(await snapSource.evaluate((element) => Number.parseFloat(element.style.width)), 180,
    'one redo reapplies the final resize width');
  await page.keyboard.press('Control+z');
  await page.waitForTimeout(150);
  assert.equal(await snapSource.evaluate((element) => Number.parseFloat(element.style.width)), 40,
    'source returns to its equal-width group fixture');

  // #214 group preview scales one captured union without rebuilding the document
  // per target/sample. Equal-width members keep the Inspector at 40px throughout
  // the 80-sample preview, then land in one bulk geometry request and undo step.
  await snapSource.click();
  await shiftClick(page, snapCandidate);
  const groupResizeHandle = page.locator('.moveable-control[data-direction="se"]').last();
  const groupWidthsBefore = await Promise.all([snapSource, snapCandidate].map((locator) =>
    locator.evaluate((element) => Number.parseFloat(element.style.width))));
  assert.deepEqual(groupWidthsBefore, [40, 40], 'group resize fixture starts with equal authored widths');
  const groupResizeWrites = writeRequests.filter((path) => path === `/design/${layout.id}/geometry`).length;
  await beginFastResizePreview(page, snapSource, groupResizeHandle, 80, 20, 'group object', [snapSource, snapCandidate]);
  const liveGroupWidths = await Promise.all([snapSource, snapCandidate].map((locator) =>
    locator.evaluate((element) => Number.parseFloat(element.style.width))));
  assert.ok(liveGroupWidths.every((width) => width > 40) && liveGroupWidths[0] === liveGroupWidths[1],
    `group preview scales only its targets with one common factor: ${JSON.stringify(liveGroupWidths)}`);
  await page.mouse.up();
  await page.waitForTimeout(150);
  assert.equal(writeRequests.filter((path) => path === `/design/${layout.id}/geometry`).length, groupResizeWrites + 1,
    '80-sample group resize emits one bulk geometry request');
  await page.keyboard.press('Control+z');
  await page.waitForTimeout(150);
  assert.deepEqual(await Promise.all([snapSource, snapCandidate].map((locator) =>
    locator.evaluate((element) => Number.parseFloat(element.style.width)))), groupWidthsBefore,
  'one undo restores every group member');

  // Pointer deltas remain model-correct at both zoom limits. Pick a requested
  // right edge that is clear of every sibling anchor even at 25%'s 20-model-px
  // smart-guide threshold, so this isolates zoom conversion from guide snapping.
  const zoomObjectId = await object.getAttribute('data-object-id');
  const zoomDelta = await page.evaluate((targetId) => {
    const targets = [...document.querySelectorAll('.fm-part > .fm-obj:not(.le-echo-ghost)')];
    const target = targets.find((element) => element.getAttribute('data-object-id') === targetId);
    const right = Number.parseFloat(target.style.left) + Number.parseFloat(target.style.width);
    const anchors = targets.filter((element) => element !== target).flatMap((element) => {
      const left = Number.parseFloat(element.style.left);
      const width = Number.parseFloat(element.style.width);
      return [left, left + width / 2, left + width];
    });
    return Array.from({ length: 18 }, (_, index) => (index + 3) * 4)
      .find((delta) => anchors.every((anchor) => Math.abs(right + delta - anchor) > 21));
  }, zoomObjectId);
  assert.ok(zoomDelta, 'zoom acceptance finds a guide-free resize delta');
  for (const percent of [25, 400]) {
    await setCanvasZoom(page, percent);
    await page.locator('.fm-canvas').dispatchEvent('click');
    await object.dispatchEvent('click');
    await page.waitForTimeout(50);
    const zoomHandle = page.locator('.moveable-control[data-direction="se"]').last();
    const zoomHandleBox = await zoomHandle.boundingBox();
    const zoomWidthBefore = await object.evaluate((element) => Number.parseFloat(element.style.width));
    const zoomWrites = writeRequests.filter((path) => path === `/design/${layout.id}/geometry`).length;
    const zoomHandleX = Math.round(zoomHandleBox.x + zoomHandleBox.width / 2);
    const zoomHandleY = Math.round(zoomHandleBox.y + zoomHandleBox.height / 2);
    await page.mouse.move(zoomHandleX, zoomHandleY);
    await page.mouse.down();
    await page.mouse.move(zoomHandleX + zoomDelta * percent / 100, zoomHandleY, { steps: 5 });
    const zoomWidthLive = await object.evaluate((element) => Number.parseFloat(element.style.width));
    assert.equal(zoomWidthLive, zoomWidthBefore + zoomDelta,
      `${percent}% zoom maps client resize delta to authored pixels`);
    await page.mouse.up();
    await page.waitForTimeout(150);
    assert.equal(writeRequests.filter((path) => path === `/design/${layout.id}/geometry`).length, zoomWrites + 1,
      `${percent}% zoom persists one final geometry request`);
    await page.keyboard.press('Control+z');
    await page.waitForTimeout(150);
    assert.equal(await object.evaluate((element) => Number.parseFloat(element.style.width)), zoomWidthBefore,
      `${percent}% zoom resize undoes exactly`);
  }
  await setCanvasZoom(page, 100);

  // #217: Escape cancels a live object drag, restores the exact authored box,
  // and emits no geometry persistence. Releasing the physical pointer afterward
  // must not revive or commit the cancelled gesture.
  await object.click();
  const cancelDragBefore = await objectX(object);
  const cancelDragWrites = writeRequests.filter((path) => path.endsWith('/geometry')).length;
  const cancelDragBox = await object.boundingBox();
  await page.mouse.move(cancelDragBox.x + cancelDragBox.width / 2, cancelDragBox.y + cancelDragBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(cancelDragBox.x + cancelDragBox.width / 2 + 19, cancelDragBox.y + cancelDragBox.height / 2 + 7, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.equal(await objectX(object), cancelDragBefore, 'Escape restores drag start geometry');
  assert.equal(writeRequests.filter((path) => path.endsWith('/geometry')).length, cancelDragWrites,
    'cancelled drag performs no geometry request');
  assert.equal(await page.locator('[data-rm-drag-bounds], .le-draw-preview').count(), 0,
    'cancelled drag leaves no live preview or bounds correction');

  const pointerCancelBefore = await objectX(object);
  const pointerCancelWrites = writeRequests.filter((path) => path.endsWith('/geometry')).length;
  const pointerCancelBox = await object.boundingBox();
  await page.mouse.move(pointerCancelBox.x + pointerCancelBox.width / 2, pointerCancelBox.y + pointerCancelBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(pointerCancelBox.x + pointerCancelBox.width / 2 + 14, pointerCancelBox.y + pointerCancelBox.height / 2, { steps: 3 });
  await page.evaluate(() => window.dispatchEvent(new PointerEvent('pointercancel', { pointerId: 1, bubbles: true })));
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.equal(await objectX(object), pointerCancelBefore, 'pointercancel restores drag start geometry');
  assert.equal(writeRequests.filter((path) => path.endsWith('/geometry')).length, pointerCancelWrites,
    'pointercancel performs no geometry request');

  const lostCaptureBefore = await objectX(object);
  const lostCaptureWrites = writeRequests.filter((path) => path.endsWith('/geometry')).length;
  const lostCaptureBox = await object.boundingBox();
  await page.mouse.move(lostCaptureBox.x + lostCaptureBox.width / 2, lostCaptureBox.y + lostCaptureBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(lostCaptureBox.x + lostCaptureBox.width / 2 + 12, lostCaptureBox.y + lostCaptureBox.height / 2, { steps: 3 });
  const releasedCapture = await page.evaluate(() => {
    const owner = [...document.querySelectorAll('*')].find((element) =>
      element instanceof HTMLElement && element.hasPointerCapture(1));
    if (!(owner instanceof HTMLElement)) return false;
    owner.releasePointerCapture(1);
    return true;
  });
  assert.equal(releasedCapture, true, 'test gesture owns pointer capture');
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.equal(await objectX(object), lostCaptureBefore, 'lostpointercapture restores drag start geometry');
  assert.equal(writeRequests.filter((path) => path.endsWith('/geometry')).length, lostCaptureWrites,
    'lostpointercapture performs no geometry request');

  // Resize uses the same transaction and finalizer. The southeast handle is a
  // Moveable control, so this also exercises cancellation through control chrome.
  const resizeBefore = await object.evaluate((element) => ({
    x: Number.parseFloat(element.style.left),
    y: Number.parseFloat(element.style.top),
    w: Number.parseFloat(element.style.width),
    h: Number.parseFloat(element.style.height),
  }));
  const resizeHandle = page.locator('.moveable-control[data-direction="se"]').last();
  const resizeHandleBox = await resizeHandle.boundingBox();
  assert.ok(resizeHandleBox, 'selected object exposes a southeast resize handle');
  const cancelResizeWrites = writeRequests.filter((path) => path.endsWith('/geometry')).length;
  await page.mouse.move(resizeHandleBox.x + resizeHandleBox.width / 2, resizeHandleBox.y + resizeHandleBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(resizeHandleBox.x + resizeHandleBox.width / 2 + 23, resizeHandleBox.y + resizeHandleBox.height / 2 + 17, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.deepEqual(await object.evaluate((element) => ({
    x: Number.parseFloat(element.style.left),
    y: Number.parseFloat(element.style.top),
    w: Number.parseFloat(element.style.width),
    h: Number.parseFloat(element.style.height),
  })), resizeBefore, 'Escape restores resize start geometry');
  assert.equal(writeRequests.filter((path) => path.endsWith('/geometry')).length, cancelResizeWrites,
    'cancelled resize performs no geometry request');

  // #99/#205: modifier presses stay pending until the movement threshold. A
  // Shift-press on an unselected object immediately hands the live stream to
  // Moveable and drags the expanded selection. A stationary Shift-click still
  // toggles, while Shift-dragging an already selected object preserves the
  // group instead of removing it at pointer-down.
  const modifierTarget = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${modifierTargetId}"]`);
  await object.click();
  const addDragBefore = [await objectX(object), await objectX(modifierTarget)];
  await shiftDragBy(page, modifierTarget, 11);
  const addDragAfter = [await objectX(object), await objectX(modifierTarget)];
  const addDragDelta = addDragAfter.map((x, index) => x - addDragBefore[index]);
  assert.ok(addDragDelta[0] === addDragDelta[1] && addDragDelta[0] !== 0,
    `Shift-select and drag expands and moves the selection in one pointer gesture: ${JSON.stringify(addDragDelta)}`);

  await shiftClick(page, object);
  const toggledBefore = [await objectX(object), await objectX(modifierTarget)];
  await dragBy(page, modifierTarget, 7);
  const toggledAfter = [await objectX(object), await objectX(modifierTarget)];
  const toggledDelta = toggledAfter.map((x, index) => x - toggledBefore[index]);
  assert.ok(toggledDelta[0] === 0 && toggledDelta[1] !== 0,
    `stationary Shift-click removes a selected object without swallowing the next drag: ${JSON.stringify(toggledDelta)}`);

  await shiftClick(page, object);
  const keepDragBefore = [await objectX(object), await objectX(modifierTarget)];
  await shiftDragBy(page, object, 9);
  const keepDragAfter = [await objectX(object), await objectX(modifierTarget)];
  const keepDragDelta = keepDragAfter.map((x, index) => x - keepDragBefore[index]);
  assert.ok(keepDragDelta[0] === keepDragDelta[1] && keepDragDelta[0] !== 0,
    `Shift-drag on a selected object preserves and moves the existing selection: ${JSON.stringify(keepDragDelta)}`);

  await page.mouse.click(canvas.x + canvas.width - 10, canvas.y + canvas.height - 10);
  await object.click();

  // Group drag snaps one anchor and applies one common delta. The generated
  // Form supplies a caption and value field, giving this check two real objects.
  const secondObject = page.locator('.fm-canvas .fm-obj').nth(1);
  assert.ok(await secondObject.count(), 'generated layout has a second object for group drag');
  await shiftClick(page, secondObject);
  const groupBefore = [await objectX(object), await objectX(secondObject)];
  await dragBy(page, object, 7);
  const groupAfter = [await objectX(object), await objectX(secondObject)];
  assert.equal(groupAfter[0] - groupBefore[0], groupAfter[1] - groupBefore[1], 'group members preserve their offset');
  await fastDragAligned(page, 2, 211, 'group object and bounds');

  await page.mouse.click(canvas.x + canvas.width - 10, canvas.y + canvas.height - 10);
  await object.click();
  await page.waitForTimeout(50);
  await fastDragAligned(page, 1, 411, 'single object and bounds');

  // A cancelled marquee restores the exact prior selection. Prove it by
  // dragging the prior single target afterward: the unrelated rectangle stays.
  const marqueeObjectBefore = await objectX(object);
  const marqueeOtherBefore = await objectX(modifierTarget);
  const liveCanvas = await page.locator('.fm-canvas').boundingBox();
  await page.mouse.move(liveCanvas.x + liveCanvas.width - 8, liveCanvas.y + liveCanvas.height - 8);
  await page.mouse.down();
  await page.mouse.move(liveCanvas.x + 5, liveCanvas.y + 5, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.waitForTimeout(100);
  await dragBy(page, object, 5);
  assert.notEqual(await objectX(object), marqueeObjectBefore, 'a new drag starts immediately after marquee cancel');
  assert.equal(await objectX(modifierTarget), marqueeOtherBefore, 'marquee cancel restores the prior single selection');

  // Band resize participates in the same explicit lifecycle and history
  // transaction. Cancel restores both height and prior object selection.
  const bodyBand = page.locator(`[data-part-id="${bodyPart.id}"]`);
  const bodyResize = page.getByTitle('Resize Body band');
  const bodyBeforeCancel = await bodyBand.boundingBox();
  const bodyResizeBox = await bodyResize.boundingBox();
  const cancelBandWrites = writeRequests.filter((path) => path.endsWith(`/part/${bodyPart.id}/height`)).length;
  await page.mouse.move(bodyResizeBox.x + bodyResizeBox.width / 2, bodyResizeBox.y + bodyResizeBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(bodyResizeBox.x + bodyResizeBox.width / 2, bodyResizeBox.y + bodyResizeBox.height / 2 + 20, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.equal((await bodyBand.boundingBox()).height, bodyBeforeCancel.height, 'Escape restores band height');
  assert.equal(writeRequests.filter((path) => path.endsWith(`/part/${bodyPart.id}/height`)).length, cancelBandWrites,
    'cancelled band resize performs no height request');

  const noOpBandHandle = await bodyResize.boundingBox();
  await page.mouse.move(noOpBandHandle.x + noOpBandHandle.width / 2, noOpBandHandle.y + noOpBandHandle.height / 2);
  await page.mouse.down();
  await page.mouse.up();
  await page.waitForTimeout(50);
  assert.equal((await bodyBand.boundingBox()).height, bodyBeforeCancel.height,
    'stationary band-handle click does not jump a short band to its content minimum');
  assert.equal(writeRequests.filter((path) => path.endsWith(`/part/${bodyPart.id}/height`)).length, cancelBandWrites,
    'stationary band-handle click performs no height request');

  const bodyResizeAfterCancel = await bodyResize.boundingBox();
  await page.mouse.move(bodyResizeAfterCancel.x + bodyResizeAfterCancel.width / 2, bodyResizeAfterCancel.y + bodyResizeAfterCancel.height / 2);
  await page.mouse.down();
  const bandSection = page.locator('#layout-inspector .insp-sec').filter({ has: page.getByText('Band', { exact: true }) });
  await bandSection.waitFor();
  const partHeightInput = bandSection.locator('.insp-row').filter({ hasText: 'Height' }).locator('input[type="number"]');
  await partHeightInput.waitFor();
  const authoredBandHeight = await partHeightInput.inputValue();
  const minimumBandHeight = Number(await partHeightInput.getAttribute('min'));
  const expectedBandHeight = Math.max(minimumBandHeight, Number(authoredBandHeight) + 10);
  assert.equal(Number(authoredBandHeight), Math.round(bodyBeforeCancel.height), 'band Inspector starts at authored DOM height');
  await page.mouse.move(bodyResizeAfterCancel.x + bodyResizeAfterCancel.width / 2, bodyResizeAfterCancel.y + bodyResizeAfterCancel.height / 2 + 10, { steps: 80 });
  await page.waitForTimeout(40);
  const liveBandBox = await bodyBand.boundingBox();
  const liveBandHandle = await bodyResize.boundingBox();
  assert.equal(await partHeightInput.inputValue(), authoredBandHeight,
    '80-sample band preview leaves the reactive authored height unchanged');
  assert.equal(Math.round(liveBandBox.height), expectedBandHeight,
    'band DOM previews the final clamped height before pointer-up');
  assert.ok(Math.abs(liveBandBox.y + liveBandBox.height - (liveBandHandle.y + liveBandHandle.height / 2)) <= 1.51,
    'band resize handle stays attached to its preview edge');
  await page.mouse.up();
  await page.waitForTimeout(150);
  assert.equal(writeRequests.filter((path) => path.endsWith(`/part/${bodyPart.id}/height`)).length, cancelBandWrites + 1,
    'successful band resize emits exactly one height request');
  assert.equal(await partHeightInput.inputValue(), String(expectedBandHeight),
    'band publishes one final authored height on pointer-up');

  // Draw cancellation removes its preview, creates no object, and leaves the
  // tool armed. A second Escape disarms it, establishing the documented priority.
  const rectangleTool = page.locator('button[aria-label="Rectangle"]');
  await rectangleTool.click();
  const countBeforeDrawCancel = await page.locator('.fm-canvas .fm-obj').count();
  const objectWritesBeforeDrawCancel = writeRequests.filter((path) => path === `/design/${layout.id}/object`).length;
  const liveBodyBox = await bodyBand.boundingBox();
  const drawX = liveBodyBox.x + liveBodyBox.width - 170;
  const drawY = liveBodyBox.y + liveBodyBox.height - 45;
  await page.mouse.move(drawX, drawY);
  await page.mouse.down();
  await page.mouse.move(drawX + 35, drawY - 25, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.equal(await page.locator('.le-draw-preview').count(), 0, 'draw cancel removes its preview');
  assert.equal(await page.locator('.fm-canvas .fm-obj').count(), countBeforeDrawCancel, 'draw cancel creates no object');
  assert.equal(writeRequests.filter((path) => path === `/design/${layout.id}/object`).length, objectWritesBeforeDrawCancel,
    'draw cancel performs no create request');
  assert.equal(await rectangleTool.getAttribute('aria-pressed'), 'true', 'first Escape cancels but keeps the draw tool armed');
  await page.keyboard.press('Escape');
  assert.equal(await rectangleTool.getAttribute('aria-pressed'), 'false', 'second Escape disarms the draw tool');

  // Window focus loss follows the same draw recovery path.
  await rectangleTool.click();
  const countBeforeBlurCancel = await page.locator('.fm-canvas .fm-obj').count();
  await page.mouse.move(drawX, drawY);
  await page.mouse.down();
  await page.mouse.move(drawX + 20, drawY - 15, { steps: 3 });
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.equal(await page.locator('.fm-canvas .fm-obj').count(), countBeforeBlurCancel,
    'window blur cancels draw without creating an object');
  assert.equal(await page.locator('.le-draw-preview').count(), 0, 'window blur removes draw preview');

  const countBeforeHiddenCancel = await page.locator('.fm-canvas .fm-obj').count();
  await page.mouse.move(drawX, drawY);
  await page.mouse.down();
  await page.mouse.move(drawX + 18, drawY - 12, { steps: 3 });
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
    delete document.visibilityState;
  });
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.equal(await page.locator('.fm-canvas .fm-obj').count(), countBeforeHiddenCancel,
    'visibility loss cancels draw without creating an object');
  assert.equal(await page.locator('.le-draw-preview').count(), 0, 'visibility loss removes draw preview');
  await page.keyboard.press('Escape');

  // Rotation is another authored-geometry family and must roll both its box and
  // behavior props back together.
  const line = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${lineId}"]`);
  await line.click({ force: true });
  await page.waitForTimeout(50);
  const lineBeforeRotateCancel = await line.evaluate((element) => ({
    x: Number.parseFloat(element.style.left),
    y: Number.parseFloat(element.style.top),
    w: Number.parseFloat(element.style.width),
    h: Number.parseFloat(element.style.height),
  }));
  const rotationControl = page.locator('.moveable-rotation-control').last();
  const rotationBox = await rotationControl.boundingBox();
  assert.ok(rotationBox, 'selected line exposes a rotation control');
  const lineWritesBeforeCancel = writeRequests.length;
  await page.mouse.move(rotationBox.x + rotationBox.width / 2, rotationBox.y + rotationBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(rotationBox.x + rotationBox.width / 2 + 30, rotationBox.y + rotationBox.height / 2 + 20, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.waitForTimeout(100);
  assert.deepEqual(await line.evaluate((element) => ({
    x: Number.parseFloat(element.style.left),
    y: Number.parseFloat(element.style.top),
    w: Number.parseFloat(element.style.width),
    h: Number.parseFloat(element.style.height),
  })), lineBeforeRotateCancel, 'Escape restores line rotation geometry');
  assert.equal(writeRequests.length, lineWritesBeforeCancel, 'cancelled line rotation performs no persistence');

  // #203: a portal's owned header + field move implicitly while the portal stays
  // the sole visible selection. Pointer drag, keyboard nudge, cross-band settle,
  // and reload persistence all operate on the same expanded movement set.
  const portal = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${portalId}"]`);
  const portalChildren = portalChildIds.map((id) => page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${id}"]`));
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

  // Pointer capture keeps a drag alive when the pointer leaves the canvas. The
  // outside release commits normally and the next gesture remains available.
  const outsideBox = await modifierTarget.boundingBox();
  const outsideSaved = page.waitForResponse((response) =>
    new URL(response.url()).pathname === `/design/${layout.id}/geometry` && response.request().method() === 'POST');
  await page.mouse.move(outsideBox.x + outsideBox.width / 2, outsideBox.y + outsideBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(canvas.x - 20, outsideBox.y + outsideBox.height / 2, { steps: 5 });
  await page.mouse.up();
  assert.equal((await outsideSaved).status(), 200, 'release outside canvas commits through pointer capture');
  assert.equal(await objectX(modifierTarget), 0, 'outside release persists the live clamped geometry');

  await page.waitForTimeout(300);
  // Component teardown during an active draw runs the same cancel finalizer.
  await rectangleTool.click();
  const countBeforeTeardown = await page.locator('.fm-canvas .fm-obj').count();
  const objectWritesBeforeTeardown = writeRequests.filter((path) => path === `/design/${layout.id}/object`).length;
  const teardownBodyBox = await bodyBand.boundingBox();
  const teardownX = teardownBodyBox.x + teardownBodyBox.width - 150;
  const teardownY = teardownBodyBox.y + teardownBodyBox.height - 40;
  await page.mouse.move(teardownX, teardownY);
  await page.mouse.down();
  await page.mouse.move(teardownX + 25, teardownY - 15, { steps: 3 });
  await page.reload();
  await page.mouse.up();
  await page.locator('.fm-canvas').waitFor();
  assert.equal(await page.locator('.fm-canvas .fm-obj').count(), countBeforeTeardown,
    'teardown during draw creates no object');
  assert.equal(writeRequests.filter((path) => path === `/design/${layout.id}/object`).length, objectWritesBeforeTeardown,
    'teardown during draw performs no create request');
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
