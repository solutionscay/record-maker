// #220 real-browser acceptance for transform visuals, cursors, and hit targets.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import playwright from 'playwright-core';

const uiDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoDir = resolve(uiDir, '..');
const port = Number(process.env.RM_AFFORDANCE_PORT || 4336);
const base = `http://127.0.0.1:${port}`;
const dataDir = await mkdtemp(resolve(tmpdir(), 'record-maker-affordance-'));
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
    if (current === percent) {
      await page.waitForTimeout(50);
      return;
    }
    await page.getByRole('button', { name: current < percent ? 'Zoom in' : 'Zoom out' }).click();
  }
  assert.fail(`could not set canvas zoom to ${percent}%`);
}

async function activeControl(page, direction) {
  const control = page.locator(`.moveable-control[data-direction="${direction}"]`).last();
  await control.waitFor();
  return control;
}

let browser;
try {
  await waitForServer();
  const table = await postJson('/schema/tables', {
    name: 'Transform Affordance Acceptance',
    notes: '',
    fields: [{ name: 'Name', kind: 'text' }],
  });
  const layout = (await request('/layouts/all'))
    .find((candidate) => candidate.tableId === table.id && candidate.view === 'form');
  assert.ok(layout, 'Form layout exists');
  const model = await request(`/design/${layout.id}/model`);
  const body = model.parts.find((part) => part.kind === 'body');
  assert.ok(body, 'Body band exists');
  const create = (kind, x, y, w, h, props) => postJson(`/design/${layout.id}/object`, {
    partId: body.id, kind, x, y, w, h, props,
  });
  const dark = (await create('rect', 48, 80, 104, 64, { fill: '#050505' }))[0];
  const light = (await create('rect', 250, 80, 104, 64, { fill: '#ffffff' }))[0];
  const saturated = (await create('rect', 452, 80, 104, 64, { fill: '#ff1744' }))[0];
  const line = (await create('line', 80, 260, 140, 12, { stroke: '#111111', strokeWidth: 2 }))[0];
  await postJson(`/design/${layout.id}/part/${body.id}/height`, { height: 900 });

  browser = await playwright.chromium.launch({
    headless: true,
    executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
  });
  const context = await browser.newContext({ viewport: { width: 1_000, height: 700 } });
  const page = await context.newPage();
  await page.addInitScript(() => { window.RM_LOG = false; });
  await page.goto(`${base}/design/${layout.id}`);
  await page.locator('.fm-canvas').waitFor();
  const stage = page.locator('.le-stage');
  const object = (id) => page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${id}"]`);
  const darkObject = object(dark.id);

  // Handles and band boundaries stay fixed in SCREEN pixels across every canvas
  // zoom. Pseudo-element measurements prove the visible centre remains smaller.
  // Visit the opposite clamp before returning to 100%; 25% plus 10-point steps
  // intentionally does not land on 100 without first clamping at 400.
  for (const percent of [25, 400, 100]) {
    await setCanvasZoom(page, percent);
    await darkObject.scrollIntoViewIfNeeded();
    await darkObject.click();
    await page.waitForTimeout(60);
    const lineThicknesses = await page.locator('.moveable-line').evaluateAll((elements) => elements.map((element) => {
      const rect = element.getBoundingClientRect();
      return Math.min(rect.width, rect.height);
    }));
    assert.ok(lineThicknesses.every((thickness) => thickness >= 0.9 && thickness <= 1.1),
      `${percent}% selection bounds retain a one-screen-pixel core: ${JSON.stringify(lineThicknesses)}`);
    const se = await activeControl(page, 'se');
    const controlBox = await se.boundingBox();
    assert.ok(controlBox.width >= 23.5 && controlBox.height >= 23.5,
      `${percent}% resize handle keeps a 24px screen-space hit target: ${JSON.stringify(controlBox)}`);
    const visual = await se.evaluate((element) => {
      const style = getComputedStyle(element, '::after');
      return { width: Number.parseFloat(style.width), height: Number.parseFloat(style.height) };
    });
    assert.ok(
      Math.abs(visual.width * percent / 100 - 7) <= 1.1
        && Math.abs(visual.height * percent / 100 - 7) <= 1.1,
      `${percent}% handle visual centre stays 7 screen px: ${JSON.stringify(visual)}`,
    );
    const ownsOuterHit = await se.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      return document.elementFromPoint(rect.right - 2, rect.top + rect.height / 2)?.closest('.moveable-control') === element;
    });
    assert.equal(ownsOuterHit, true, `${percent}% invisible outer handle area is pointer-acquirable`);

    const band = page.locator(`.le-part-resize[data-overlay-part-id="${body.id}"]`);
    await band.scrollIntoViewIfNeeded();
    await stage.evaluate((element) => { element.scrollLeft = 0; });
    const bandBox = await band.boundingBox();
    assert.ok(bandBox.height >= 7.5 && bandBox.height <= 8.5,
      `${percent}% canvas-side band strip stays at its non-conflicting 8px target: ${JSON.stringify(bandBox)}`);
    const bandHit = await band.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.left - 10, rect.top - 2);
      return { owns: hit === element, hit: hit?.className ?? hit?.tagName, rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height } };
    });
    assert.equal(bandHit.owns, true, `${percent}% gutter expands band acquisition beyond the canvas strip: ${JSON.stringify(bandHit)}`);
  }

  await setCanvasZoom(page, 100);
  await darkObject.scrollIntoViewIfNeeded();
  await darkObject.click();
  await page.waitForTimeout(60);

  // Directional and rotation cursors are application-owned rather than library
  // defaults; the object itself advertises grab before a drag begins.
  assert.equal(await darkObject.evaluate((element) => getComputedStyle(element).cursor), 'grab', 'object cursor is grab');
  for (const [direction, expected] of [['n', 'ns-resize'], ['e', 'ew-resize'], ['ne', 'nesw-resize'], ['se', 'nwse-resize']]) {
    assert.equal(await (await activeControl(page, direction)).evaluate((element) => getComputedStyle(element).cursor), expected,
      `${direction} handle uses ${expected}`);
  }
  await object(line.id).click();
  await page.waitForTimeout(60);
  const rotate = page.locator('.moveable-rotation-control').last();
  await rotate.waitFor();
  const rotateBox = await rotate.boundingBox();
  assert.ok(rotateBox.width >= 23.5 && rotateBox.height >= 23.5, 'line rotation handle has the minimum hit target');
  assert.equal(await rotate.evaluate((element) => getComputedStyle(element).cursor), 'grab', 'line rotation cursor is grab');

  await page.getByRole('button', { name: 'Rectangle' }).click();
  assert.equal(await page.locator('.fm-canvas').evaluate((element) => getComputedStyle(element).cursor), 'crosshair',
    'an armed draw tool owns the crosshair cursor');
  await page.keyboard.press('Escape');
  await stage.evaluate((element) => element.scrollTo(0, 0));
  const panProbeBox = await darkObject.boundingBox();
  await page.keyboard.down('Space');
  assert.equal(await darkObject.evaluate((element) => getComputedStyle(element).cursor), 'grab',
    'Space hand-pan advertises grab before pointer-down');
  await page.mouse.move(panProbeBox.x + panProbeBox.width / 2, panProbeBox.y + panProbeBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(panProbeBox.x + panProbeBox.width / 2 + 8, panProbeBox.y + panProbeBox.height / 2 + 8);
  assert.equal(await darkObject.evaluate((element) => getComputedStyle(element).cursor), 'grabbing',
    'active hand-pan keeps a grabbing cursor across authored objects');
  await page.mouse.up();
  await page.keyboard.up('Space');

  // Keyboard selection gets the same visible Moveable controls.
  await page.locator('.fm-canvas').click({ position: { x: 700, y: 500 } });
  await page.keyboard.press('Control+a');
  await page.waitForTimeout(60);
  assert.ok(await page.locator('.moveable-control').count() >= 8, 'keyboard Select All exposes transform controls');

  // Adversarial authored fills never alter the two-tone selection centre. Hover
  // is a distinct dashed accent + contrast outline.
  for (const id of [dark.id, light.id, saturated.id]) {
    await page.locator('.fm-canvas').dispatchEvent('click');
    await object(id).dispatchEvent('click');
    await page.waitForTimeout(40);
    const se = await activeControl(page, 'se');
    const chrome = await se.evaluate((element) => {
      const style = getComputedStyle(element, '::after');
      return { background: style.backgroundColor, border: style.borderColor, outline: style.outlineColor };
    });
    assert.notEqual(chrome.background, chrome.border, `selection centre contrasts on object ${id}`);
  }
  await page.locator('.fm-canvas').dispatchEvent('click');
  const lightHoverBox = await object(light.id).boundingBox();
  await page.mouse.move(lightHoverBox.x + lightHoverBox.width / 2, lightHoverBox.y + lightHoverBox.height / 2);
  await page.waitForTimeout(30);
  const hover = page.locator('.le-hover-outline');
  const hoverStyle = await hover.evaluate((element) => {
    const style = getComputedStyle(element);
    return { borderStyle: style.borderStyle, borderColor: style.borderColor, outlineColor: style.outlineColor };
  });
  assert.equal(hoverStyle.borderStyle, 'dashed', 'hover remains visually distinct from solid selection bounds');
  assert.notEqual(hoverStyle.borderColor, hoverStyle.outlineColor, 'hover carries a contrast outline');

  // Drag active state owns a stable grabbing cursor, shows only resolver-backed
  // snap guides, and removes all transient feedback synchronously on Escape.
  await stage.evaluate((element) => element.scrollTo(0, 0));
  await page.locator('.fm-canvas').dispatchEvent('click');
  await darkObject.dispatchEvent('click');
  await page.waitForTimeout(60);
  const darkBox = await darkObject.boundingBox();
  const lightBox = await object(light.id).boundingBox();
  const dragStartX = darkBox.x + darkBox.width / 2;
  const dragStartY = darkBox.y + darkBox.height / 2;
  await page.mouse.move(dragStartX, dragStartY);
  await page.mouse.down();
  await page.mouse.move(
    dragStartX + lightBox.x - darkBox.x - 3,
    dragStartY + lightBox.y - darkBox.y,
    { steps: 8 },
  );
  await page.waitForTimeout(40);
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-object-dragging')), true,
    'active object drag owns stage presentation');
  assert.equal(await darkObject.evaluate((element) => getComputedStyle(element).cursor), 'grabbing',
    'drag cursor remains grabbing across overlay handoff');
  assert.match(await page.locator('.le-geometry-badge').textContent(), /^Δ -?\d+, -?\d+$/,
    'drag badge reports the applied authored delta');
  const activeGuides = page.locator('.le-smart-guide-active');
  assert.ok(await activeGuides.count() > 0, 'aligned drag exposes an active snapped guide');
  assert.equal(await activeGuides.first().getAttribute('data-rm-snap-active'), 'true',
    'active guide is explicitly resolver-backed');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  assert.equal(await page.locator('.le-geometry-badge').count(), 0, 'Escape removes drag geometry feedback');
  assert.equal(await activeGuides.count(), 0, 'Escape removes active snap feedback');
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-transforming')), false,
    'Escape restores idle cursor state');

  // Resize badge matches live screen geometry and disappears on pointer-up.
  await page.locator('.fm-canvas').dispatchEvent('click');
  await darkObject.dispatchEvent('click');
  await page.waitForTimeout(50);
  const se = await activeControl(page, 'se');
  const seBox = await se.boundingBox();
  await page.mouse.move(seBox.x + seBox.width / 2, seBox.y + seBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(seBox.x + seBox.width / 2 + 32, seBox.y + seBox.height / 2 + 16, { steps: 6 });
  await page.waitForTimeout(40);
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-object-resizing')), true,
    'resize owns active stage state');
  assert.equal(await stage.evaluate((element) => getComputedStyle(element).cursor), 'nwse-resize',
    'active resize cursor remains directional');
  const badgeText = await page.locator('.le-geometry-badge').textContent();
  const badgeSize = badgeText.split('×').map((part) => Number(part.trim()));
  const liveRect = await darkObject.boundingBox();
  assert.ok(Math.abs(badgeSize[0] - liveRect.width) <= 1 && Math.abs(badgeSize[1] - liveRect.height) <= 1,
    `resize badge matches live geometry: ${JSON.stringify({ badgeSize, liveRect })}`);
  await page.mouse.up();
  assert.equal(await page.locator('.le-geometry-badge').count(), 0, 'pointer-up removes resize geometry feedback');
  assert.equal(await stage.evaluate((element) => element.classList.contains('is-transforming')), false,
    'pointer-up restores idle transform cursor state');

  // Reduced-motion keeps all feedback static. Forced colors hands chrome to
  // system colors without allowing authored fills to erase it.
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.locator('.fm-canvas').dispatchEvent('click');
  await darkObject.dispatchEvent('click');
  const reduced = await (await activeControl(page, 'se')).evaluate((element) => {
    const style = getComputedStyle(element);
    return { animation: style.animationName, transition: style.transitionDuration };
  });
  assert.equal(reduced.animation, 'none', 'handle animation is disabled under reduced motion');
  assert.equal(reduced.transition, '0s', 'handle transition is disabled under reduced motion');
  await page.emulateMedia({ reducedMotion: 'no-preference', forcedColors: 'active' });
  const forced = await (await activeControl(page, 'se')).evaluate((element) => {
    const style = getComputedStyle(element, '::after');
    return { adjust: style.forcedColorAdjust, borderWidth: style.borderWidth };
  });
  assert.equal(forced.adjust, 'none', 'handle opts into explicit forced-color styling');
  assert.equal(forced.borderWidth, '2px', 'forced-color handle retains a strong system outline');
  await page.emulateMedia({ forcedColors: 'none' });

  // A coarse-pointer page receives larger screen-space targets with no geometry
  // changes or alternate interaction implementation.
  const touchContext = await browser.newContext({ viewport: { width: 1_000, height: 700 }, hasTouch: true });
  const touchPage = await touchContext.newPage();
  await touchPage.addInitScript(() => { window.RM_LOG = false; });
  await touchPage.goto(`${base}/design/${layout.id}`);
  await touchPage.locator('.fm-canvas').waitFor();
  await touchPage.locator(`.fm-obj[data-object-id="${dark.id}"]`).click();
  await touchPage.waitForTimeout(60);
  const coarseHandle = await touchPage.locator('.moveable-control[data-direction="se"]').last().boundingBox();
  const coarseBandLocator = touchPage.locator(`.le-part-resize[data-overlay-part-id="${body.id}"]`);
  await coarseBandLocator.scrollIntoViewIfNeeded();
  await touchPage.locator('.le-stage').evaluate((element) => { element.scrollLeft = 0; });
  const coarseBand = await coarseBandLocator.boundingBox();
  assert.ok(coarseHandle.width >= 31.5 && coarseHandle.height >= 31.5, 'touch handle expands to 32 screen px');
  assert.ok(coarseBand.height >= 7.5, 'touch band keeps the non-conflicting canvas strip');
  const coarseGutterHit = await coarseBandLocator.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return document.elementFromPoint(rect.left - 10, rect.top - 6) === element;
  });
  assert.equal(coarseGutterHit, true, 'touch band gutter expands to a 28px screen-space target');
  await touchContext.close();

  console.log('transform affordance browser acceptance passed');
} catch (error) {
  if (serverLog) console.error(serverLog);
  throw error;
} finally {
  if (browser) await browser.close();
  if (!server.killed) server.kill('SIGTERM');
  await rm(dataDir, { recursive: true, force: true });
}
