// #215 deterministic dense-canvas acceptance. Timing is reported, but the
// structural guards (target-rect reads, one request, final geometry) make the
// test useful on hardware with different frame pacing.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import playwright from 'playwright-core';

const uiDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoDir = resolve(uiDir, '..');
const port = Number(process.env.RM_DENSE_PORT || 4333);
const sizes = (process.env.RM_DENSE_SIZES || '1,10,50,200').split(',').map(Number);
const recordedDenseFrameP95Ms = 83;
const base = `http://127.0.0.1:${port}`;
const dataDir = await mkdtemp(resolve(tmpdir(), 'record-maker-dense-'));
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

async function createObjects(layoutId, partId, count) {
  const ids = [];
  for (let start = 0; start < count; start += 25) {
    const batch = await Promise.all(Array.from({ length: Math.min(25, count - start) }, (_, offset) => {
      const index = start + offset;
      return postJson(`/design/${layoutId}/object`, {
        partId,
        kind: 'rect',
        x: 8 + (index % 10) * 70,
        y: 8 + Math.floor(index / 10) * 26,
        w: 40,
        h: 18,
      });
    }));
    ids.push(...batch.map((views) => views[0].id));
  }
  return ids;
}

async function createCase(size) {
  const table = await postJson('/schema/tables', {
    name: `Dense ${size}`,
    notes: '',
    fields: [{ name: 'Name', kind: 'text' }],
  });
  const layouts = await request('/layouts/all');
  const layout = layouts.find((candidate) => candidate.tableId === table.id && candidate.view === 'form');
  assert.ok(layout, `${size}-object Form layout exists`);
  const model = await request(`/design/${layout.id}/model`);
  const body = model.parts.find((part) => part.kind === 'body');
  const ids = await createObjects(layout.id, body.id, size);
  await postJson(`/design/${layout.id}/part/${body.id}/height`, {
    height: Math.max(120, 20 + Math.ceil(size / 10) * 26),
  });
  if (ids.length > 1) await postJson(`/design/${layout.id}/group`, { objectIds: ids });
  return { layoutId: layout.id, targetId: ids[0], size };
}

async function measurePointerBurst(page, target, endX, endY) {
  const metrics = page.evaluate((targetId) => new Promise((resolveMetrics) => {
    const target = document.querySelector(`.fm-obj:not(.le-echo-ghost)[data-object-id="${targetId}"]`);
    const tracker = window.__rmPointerProbe;
    const startSequence = tracker.sequence;
    let targetRectReads = 0;
    let controlRectReads = 0;
    let observedSequence = -1;
    let priorFrameAt = performance.now();
    let frames = 0;
    const frameIntervals = [];
    const styleLatencies = [];
    const originalRect = Element.prototype.getBoundingClientRect;
    Element.prototype.getBoundingClientRect = function patchedRect() {
      if (this instanceof Element && this.matches('.fm-obj:not(.le-echo-ghost)')) {
        targetRectReads += 1;
      }
      if (this instanceof Element && this.matches('.moveable-control-box')) controlRectReads += 1;
      return originalRect.call(this);
    };
    const observer = new MutationObserver(() => {
      if (observedSequence === tracker.sequence) return;
      observedSequence = tracker.sequence;
      styleLatencies.push(performance.now() - tracker.at);
    });
    if (target) observer.observe(target, { attributes: true, attributeFilter: ['style'] });
    const sample = (now) => {
      frameIntervals.push(now - priorFrameAt);
      priorFrameAt = now;
      frames += 1;
      if (frames < 95) requestAnimationFrame(sample);
      else {
        Element.prototype.getBoundingClientRect = originalRect;
        observer.disconnect();
        const sortedFrames = frameIntervals.slice(5).toSorted((a, b) => a - b);
        const sortedLatencies = styleLatencies.toSorted((a, b) => a - b);
        const percentile = (values, ratio) => values[Math.min(values.length - 1, Math.floor(values.length * ratio))] ?? 0;
        resolveMetrics({
          pointerSamples: tracker.sequence - startSequence,
          targetRectReads,
          controlRectReads,
          styleSamples: sortedLatencies.length,
          frameP95Ms: percentile(sortedFrames, 0.95),
          frameMaxMs: sortedFrames.at(-1) ?? 0,
          styleP95Ms: percentile(sortedLatencies, 0.95),
        });
      }
    };
    requestAnimationFrame(sample);
  }), await target.getAttribute('data-object-id'));
  await page.mouse.move(endX, endY, { steps: 80 });
  return metrics;
}

function assertMetrics(kind, size, metrics) {
  assert.ok(metrics.pointerSamples >= 70, `${kind} ${size}: burst delivered pointer samples (${JSON.stringify(metrics)})`);
  assert.ok(metrics.styleSamples > 0, `${kind} ${size}: live style feedback observed (${JSON.stringify(metrics)})`);
  assert.ok(metrics.targetRectReads <= Math.max(4, Math.ceil(size * 0.05)),
    `${kind} ${size}: no per-sample whole-selection rect loop (${JSON.stringify(metrics)})`);
  assert.ok(metrics.controlRectReads <= metrics.pointerSamples + 8,
    `${kind} ${size}: control correction is frame-bounded (${JSON.stringify(metrics)})`);
  const frameLimit = size >= 200 ? 33 : 17.5;
  assert.ok(metrics.frameP95Ms <= frameLimit,
    `${kind} ${size}: p95 frame time stays bounded (${JSON.stringify(metrics)})`);
  if (size >= 200) {
    assert.ok(metrics.frameP95Ms <= recordedDenseFrameP95Ms / 2,
      `${kind} ${size}: p95 improves at least 50% from ${recordedDenseFrameP95Ms}ms baseline (${JSON.stringify(metrics)})`);
  }
  assert.ok(metrics.styleP95Ms <= 34.5,
    `${kind} ${size}: p95 pointer-to-style feedback stays within one 30Hz fallback frame (${JSON.stringify(metrics)})`);
}

let browser;
try {
  await waitForServer();
  const cases = [];
  for (const size of sizes) cases.push(await createCase(size));

  browser = await playwright.chromium.launch({
    headless: true,
    executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
  });
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  await page.addInitScript(() => {
    window.RM_LOG = false;
    window.__rmPointerProbe = { sequence: 0, clientX: 0, clientY: 0, at: performance.now() };
    window.addEventListener('pointermove', (event) => {
      window.__rmPointerProbe.sequence += 1;
      window.__rmPointerProbe.clientX = event.clientX;
      window.__rmPointerProbe.clientY = event.clientY;
      window.__rmPointerProbe.at = performance.now();
    }, { capture: true });
  });
  const writes = [];
  page.on('request', (requestEvent) => {
    if (requestEvent.method() === 'POST') writes.push(new URL(requestEvent.url()).pathname);
  });

  for (const denseCase of cases) {
    const { layoutId, targetId, size } = denseCase;
    await page.goto(`${base}/design/${layoutId}`);
    await page.locator('.fm-canvas').waitFor();
    const target = page.locator(`.fm-part > .fm-obj:not(.le-echo-ghost)[data-object-id="${targetId}"]`);
    await target.dispatchEvent('click');
    await page.waitForTimeout(80);

    const dragBox = await target.boundingBox();
    const dragStartX = dragBox.x + dragBox.width / 2;
    const dragStartY = dragBox.y + dragBox.height / 2;
    const dragWrites = writes.filter((path) => path === `/design/${layoutId}/geometry`).length;
    await page.mouse.move(dragStartX, dragStartY);
    await page.mouse.down();
    const dragMetrics = await measurePointerBurst(page, target, dragStartX + 18, dragStartY + 8);
    await page.mouse.up();
    await page.waitForTimeout(120);
    assertMetrics('drag', size, dragMetrics);
    assert.equal(writes.filter((path) => path === `/design/${layoutId}/geometry`).length, dragWrites + 1,
      `drag ${size}: one final bulk persistence request`);

    const resizeHandle = page.locator('.moveable-control[data-direction="se"]').last();
    const handleBox = await resizeHandle.boundingBox();
    const resizeStartX = handleBox.x + handleBox.width / 2;
    const resizeStartY = handleBox.y + handleBox.height / 2;
    const resizeWrites = writes.filter((path) => path === `/design/${layoutId}/geometry`).length;
    await page.mouse.move(resizeStartX, resizeStartY);
    await page.mouse.down();
    const resizeMetrics = await measurePointerBurst(page, target, resizeStartX + 20, resizeStartY + 10);
    await page.mouse.up();
    await page.waitForTimeout(120);
    assertMetrics('resize', size, resizeMetrics);
    assert.equal(writes.filter((path) => path === `/design/${layoutId}/geometry`).length, resizeWrites + 1,
      `resize ${size}: one final bulk persistence request`);

    console.log(`dense ${size}: ${JSON.stringify({ drag: dragMetrics, resize: resizeMetrics })}`);
  }

  console.log('dense transform browser acceptance passed');
} catch (error) {
  if (serverLog) console.error(serverLog);
  throw error;
} finally {
  if (browser) await browser.close();
  if (!server.killed) server.kill('SIGTERM');
  await rm(dataDir, { recursive: true, force: true });
}
