// #218 real-browser acceptance for frame-driven Layout viewport autoscroll.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import playwright from 'playwright-core';

const uiDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoDir = resolve(uiDir, '..');
const port = Number(process.env.RM_AUTOSCROLL_PORT || 4334);
const base = `http://127.0.0.1:${port}`;
const dataDir = await mkdtemp(resolve(tmpdir(), 'record-maker-autoscroll-'));
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

async function setCanvasZoom(page, percent) {
  const readout = page.locator('.le-zoom-num');
  for (let attempts = 0; attempts < 60; attempts += 1) {
    const current = Number((await readout.textContent()).replace('%', ''));
    if (current === percent) return;
    await page.getByRole('button', { name: current < percent ? 'Zoom in' : 'Zoom out' }).click();
  }
  assert.fail(`could not set canvas zoom to ${percent}%`);
}

async function stageScroll(page) {
  return page.locator('.le-stage').evaluate((stage) => ({
    left: stage.scrollLeft,
    top: stage.scrollTop,
    maxLeft: stage.scrollWidth - stage.clientWidth,
    maxTop: stage.scrollHeight - stage.clientHeight,
  }));
}

async function resetViewport(page) {
  await page.locator('.le-stage').evaluate((stage) => stage.scrollTo(0, 0));
  await page.waitForTimeout(40);
}

async function viewportSnapshot(locator) {
  return locator.evaluate((element) => {
    const stage = element.closest('.le-stage');
    const rect = element.getBoundingClientRect();
    return {
      scroll: { left: stage.scrollLeft, top: stage.scrollTop },
      rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
    };
  });
}

let browser;
try {
  await waitForServer();
  const table = await postJson('/schema/tables', {
    name: 'Autoscroll Acceptance',
    notes: '',
    fields: [{ name: 'Name', kind: 'text' }],
  });
  const layouts = await request('/layouts/all');
  const layout = layouts.find((candidate) => candidate.tableId === table.id && candidate.view === 'form');
  assert.ok(layout, 'Form layout exists');
  const model = await request(`/design/${layout.id}/model`);
  const body = model.parts.find((part) => part.kind === 'body');
  const header = model.parts.find((part) => part.kind === 'header');
  assert.ok(body, 'Body band exists');
  assert.ok(header, 'Header band exists');
  const createRect = (x, y, w = 80, h = 40) => postJson(`/design/${layout.id}/object`, {
    partId: body.id, kind: 'rect', x, y, w, h,
  });
  const first = (await createRect(40, 40))[0];
  const second = (await createRect(150, 70))[0];
  const crossBand = (await postJson(`/design/${layout.id}/object`, {
    partId: header.id, kind: 'rect', x: 280, y: 8, w: 70, h: 30,
  }))[0];
  await createRect(3_000, 3_500, 20, 20);
  await postJson(`/design/${layout.id}/part/${body.id}/height`, { height: 4_000 });
  await postJson(`/design/${layout.id}/group`, { objectIds: [first.id, second.id] });

  browser = await playwright.chromium.launch({
    headless: true,
    executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
  });
  const page = await browser.newPage({ viewport: { width: 900, height: 600 } });
  await page.addInitScript(() => { window.RM_LOG = false; });
  const writes = [];
  page.on('request', (event) => {
    if (event.method() === 'POST') writes.push(new URL(event.url()).pathname);
  });
  await page.goto(`${base}/design/${layout.id}`);
  await page.locator('.fm-canvas').waitFor();
  const object = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${first.id}"]`);
  const stage = page.locator('.le-stage');

  // Visit the clamped 25% endpoint before 400%; returning from 400% lands on
  // exact 10% steps, whereas incrementing from the clamped 25% value does not.
  for (const percent of [25, 400, 100]) {
    await setCanvasZoom(page, percent);
    await resetViewport(page);
    await object.scrollIntoViewIfNeeded();
    await page.waitForTimeout(40);
    await object.dispatchEvent('click');
    await page.waitForTimeout(80);
    const objectBox = await object.boundingBox();
    const stageBox = await stage.boundingBox();
    const startX = objectBox.x + objectBox.width / 2;
    const startY = objectBox.y + objectBox.height / 2;
    const edgeX = stageBox.x + stageBox.width - 10;
    const edgeY = percent === 100 ? stageBox.y + stageBox.height - 10 : startY;
    const writesBefore = writes.filter((path) => path === `/design/${layout.id}/geometry`).length;
    await page.mouse.move(startX, startY);
    await page.mouse.down();
    await page.mouse.move(edgeX, edgeY, { steps: 8 });
    await page.waitForTimeout(120);
    const beforeHold = await viewportSnapshot(object);
    await page.waitForTimeout(320);
    const afterHold = await viewportSnapshot(object);
    assert.ok(afterHold.scroll.left > beforeHold.scroll.left + 10,
      `${percent}% held edge drag scrolls horizontally: ${JSON.stringify({ beforeHold, afterHold })}`);
    if (percent === 100) {
      assert.ok(afterHold.scroll.top > beforeHold.scroll.top + 10,
        `100% held diagonal drag scrolls vertically: ${JSON.stringify({ beforeHold, afterHold })}`);
    }
    assert.ok(Math.abs(afterHold.rect.x - beforeHold.rect.x) <= 5 && Math.abs(afterHold.rect.y - beforeHold.rect.y) <= 5,
      `${percent}% group grab remains visually stable while viewport moves: ${JSON.stringify({ beforeHold, afterHold })}`);
    await page.mouse.up();
    await page.waitForTimeout(120);
    assert.equal(writes.filter((path) => path === `/design/${layout.id}/geometry`).length, writesBefore + 1,
      `${percent}% autoscroll drag persists once`);
    const stoppedAt = await stageScroll(page);
    await page.waitForTimeout(100);
    assert.deepEqual(await stageScroll(page), stoppedAt, `${percent}% pointer-up leaves no residual scroll frame`);
    await page.keyboard.press('Control+z');
    await page.waitForTimeout(300);
  }

  // Scroll compensation participates in normal cross-band settlement.
  await setCanvasZoom(page, 100);
  await resetViewport(page);
  const crossBandObject = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${crossBand.id}"]`);
  await crossBandObject.scrollIntoViewIfNeeded();
  await crossBandObject.dispatchEvent('click');
  await page.waitForTimeout(80);
  const crossBox = await crossBandObject.boundingBox();
  const crossStageBox = await stage.boundingBox();
  await page.mouse.move(crossBox.x + crossBox.width / 2, crossBox.y + crossBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(crossBox.x + crossBox.width / 2, crossStageBox.y + crossStageBox.height - 10, { steps: 8 });
  await page.waitForTimeout(320);
  assert.ok((await stageScroll(page)).top > 10, 'cross-band drag advances the viewport');
  await page.mouse.up();
  await page.waitForTimeout(160);
  const settledModel = await request(`/design/${layout.id}/model`);
  const settledPart = settledModel.parts.find((part) => part.objects.some((candidate) => candidate.id === crossBand.id));
  const settledObject = settledPart?.objects.find((candidate) => candidate.id === crossBand.id);
  assert.equal(settledPart?.id, body.id, 'autoscroll drag settles into the revealed Body band');
  assert.equal(await crossBandObject.evaluate((element) => Number.parseFloat(element.style.top)), settledObject.y,
    'persisted local y matches the live settled geometry');

  // Resize owns the same actual-scroll compensation and compositor bounds path.
  await resetViewport(page);
  await object.dispatchEvent('click');
  await page.waitForTimeout(80);
  const resizeHandle = page.locator('.moveable-control[data-direction="se"]').last();
  const handleBox = await resizeHandle.boundingBox();
  const stageBox = await stage.boundingBox();
  await page.mouse.move(handleBox.x + handleBox.width / 2, handleBox.y + handleBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(stageBox.x + stageBox.width - 10, handleBox.y + handleBox.height / 2, { steps: 8 });
  await page.waitForTimeout(100);
  const resizeBefore = await viewportSnapshot(resizeHandle);
  await page.waitForTimeout(260);
  const resizeAfter = await viewportSnapshot(resizeHandle);
  assert.ok(resizeAfter.scroll.left > resizeBefore.scroll.left + 10, 'held resize edge autoscrolls');
  const resizeCenterBefore = resizeBefore.rect.x + resizeBefore.rect.width / 2;
  const resizeCenterAfter = resizeAfter.rect.x + resizeAfter.rect.width / 2;
  assert.ok(Math.abs(resizeCenterAfter - resizeCenterBefore) <= 2,
    `resize control remains under the stationary pointer while scrolling: ${JSON.stringify({ resizeBefore, resizeAfter })}`);
  assert.ok(Math.abs(resizeAfter.rect.width - resizeBefore.rect.width) <= 0.5,
    'resize control keeps a constant screen-space visual size');
  await page.mouse.up();
  await page.waitForTimeout(120);
  await page.keyboard.press('Control+z');
  await page.waitForTimeout(120);

  // Selecto marquee continues without pointer events, and Escape stops the loop.
  await resetViewport(page);
  const canvasBox = await page.locator('.fm-canvas').boundingBox();
  const freshStageBox = await stage.boundingBox();
  await page.keyboard.down('Control');
  await page.mouse.move(freshStageBox.x + 100, canvasBox.y + 180);
  await page.mouse.down();
  await page.mouse.move(freshStageBox.x + 140, freshStageBox.y + freshStageBox.height - 10, { steps: 5 });
  await page.waitForTimeout(100);
  const marqueeBefore = await stageScroll(page);
  const marqueeBoxBefore = await page.locator('.selecto-selection').boundingBox();
  await page.waitForTimeout(240);
  const marqueeAfter = await stageScroll(page);
  const marqueeBoxAfter = await page.locator('.selecto-selection').boundingBox();
  assert.ok(marqueeAfter.top > marqueeBefore.top + 10,
    `held marquee edge autoscrolls without more pointer input: ${JSON.stringify({ marqueeBefore, marqueeAfter })}`);
  assert.ok(marqueeBoxAfter.height > marqueeBoxBefore.height + 10,
    'marquee geometry extends into newly revealed content');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.keyboard.up('Control');
  const marqueeStopped = await stageScroll(page);
  await page.waitForTimeout(100);
  assert.deepEqual(await stageScroll(page), marqueeStopped, 'marquee cancellation stops autoscroll');

  // Draw preview uses the cached coordinate frame plus actual stage scroll.
  await resetViewport(page);
  await page.locator('button[aria-label="Rectangle"]').click();
  const drawStageBox = await stage.boundingBox();
  const drawCanvasBox = await page.locator('.fm-canvas').boundingBox();
  await page.mouse.move(drawStageBox.x + 100, drawCanvasBox.y + 140);
  await page.mouse.down();
  await page.mouse.move(drawStageBox.x + 140, drawStageBox.y + drawStageBox.height - 10, { steps: 5 });
  await page.waitForTimeout(100);
  const drawBefore = await stageScroll(page);
  const drawBoxBefore = await page.locator('.le-draw-preview').boundingBox();
  await page.waitForTimeout(240);
  const drawAfter = await stageScroll(page);
  const drawBoxAfter = await page.locator('.le-draw-preview').boundingBox();
  assert.ok(drawAfter.top > drawBefore.top + 10, 'held draw edge autoscrolls and extends the preview');
  assert.ok(drawBoxAfter.height > drawBoxBefore.height + 10, 'draw preview geometry extends into revealed content');
  assert.equal(await page.locator('.le-draw-preview').count(), 1, 'draw preview remains live while scrolling');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  assert.equal(await page.locator('.le-draw-preview').count(), 0, 'draw cancellation removes preview and stops scrolling');
  await page.keyboard.press('Escape'); // disarm the still-selected Rectangle tool
  assert.equal(await page.locator('button[aria-label="Rectangle"]').getAttribute('aria-pressed'), 'false',
    'Rectangle tool is disarmed before band resize');

  // The App-owned band resize path consumes the same actual scroll deltas.
  await resetViewport(page);
  const bandHandle = page.locator(`.le-part-resize[data-overlay-part-id="${header.id}"]`);
  const bandHandleBox = await bandHandle.boundingBox();
  const bandStageBox = await stage.boundingBox();
  const bandPointerX = bandHandleBox.x + 20;
  await page.mouse.move(bandPointerX, bandHandleBox.y + bandHandleBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(bandPointerX, bandStageBox.y + bandStageBox.height - 10, { steps: 6 });
  await page.waitForTimeout(100);
  const bandBefore = await viewportSnapshot(bandHandle);
  await page.waitForTimeout(240);
  const bandAfter = await viewportSnapshot(bandHandle);
  assert.ok(bandAfter.scroll.top > bandBefore.scroll.top + 10,
    `held band resize autoscrolls: ${JSON.stringify({ bandBefore, bandAfter })}`);
  assert.ok(Math.abs((bandAfter.rect.y + bandAfter.rect.height / 2) - (bandBefore.rect.y + bandBefore.rect.height / 2)) <= 5,
    'band resize handle remains visually stable while scrolling');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  const cancelledBandModel = await request(`/design/${layout.id}/model`);
  assert.equal(cancelledBandModel.parts.find((part) => part.id === header.id).height, header.height,
    'band resize cancellation restores authored height');

  // A boundary produces no runaway frame/timer.
  await stage.evaluate((element) => element.scrollTo(element.scrollWidth, element.scrollHeight));
  const boundaryBefore = await stageScroll(page);
  const boundaryStageBox = await stage.boundingBox();
  await page.keyboard.down('Control');
  await page.mouse.move(boundaryStageBox.x + 100, boundaryStageBox.y + 100);
  await page.mouse.down();
  await page.mouse.move(boundaryStageBox.x + boundaryStageBox.width - 8, boundaryStageBox.y + boundaryStageBox.height - 8, { steps: 3 });
  await page.waitForTimeout(200);
  assert.deepEqual(await stageScroll(page), boundaryBefore, 'right/bottom scroll boundary remains clamped');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await page.keyboard.up('Control');

  console.log('viewport autoscroll browser acceptance passed');
} catch (error) {
  if (serverLog) console.error(serverLog);
  throw error;
} finally {
  if (browser) await browser.close();
  if (!server.killed) server.kill('SIGTERM');
  await rm(dataDir, { recursive: true, force: true });
}
