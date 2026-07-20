// #224 cross-engine acceptance: a normal pointer release must commit marquee
// selection after cancellation, viewport scrolling, and canvas zoom. WebKit
// releases pointer capture before Selecto receives its compatibility mouseup,
// so this test must run in both Chromium and WebKit.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import playwright from 'playwright-core';

const uiDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoDir = resolve(uiDir, '..');
const browserArg = process.argv.find((argument) => argument.startsWith('--browser='));
const browserEngine = browserArg?.slice('--browser='.length) || 'chromium';
assert.ok(['chromium', 'webkit'].includes(browserEngine), `unsupported marquee browser: ${browserEngine}`);
const port = Number(process.env.RM_MARQUEE_PORT || 4334);
const base = `http://127.0.0.1:${port}`;
const dataDir = await mkdtemp(resolve(tmpdir(), `record-maker-marquee-${browserEngine}-`));
let serverLog = '';
const server = spawn('cargo', ['run', '-p', 'record-maker-server'], {
  cwd: repoDir,
  env: { ...process.env, RM_DATA_DIR: dataDir, RM_PORT: String(port) },
  stdio: ['ignore', 'pipe', 'pipe'],
});
const capture = (chunk) => { serverLog = (serverLog + chunk.toString()).slice(-16_000); };
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
    name: 'Marquee Acceptance',
    notes: '',
    fields: [{ name: 'Name', kind: 'text' }],
  });
  const layouts = await request('/layouts/all');
  const layout = layouts.find((candidate) => candidate.tableId === table.id && candidate.view === 'form');
  assert.ok(layout, 'generated Form layout exists');
  const model = await request(`/design/${layout.id}/model`);
  const body = model.parts.find((part) => part.kind === 'body');
  assert.ok(body, 'generated Form has a Body band');
  const created = await postJson(`/design/${layout.id}/object`, {
    partId: body.id,
    kind: 'rect',
    x: 500,
    y: 4,
    w: 60,
    h: 24,
  });
  const targetId = created[0].id;

  browser = browserEngine === 'webkit'
    ? await playwright.webkit.launch({ headless: true })
    : await playwright.chromium.launch({
        headless: true,
        executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome',
      });
  const page = await browser.newPage({ viewport: { width: 1200, height: 720 } });
  const pageErrors = [];
  page.on('pageerror', (error) => pageErrors.push(error.message));
  await page.goto(`${base}/design/${layout.id}`);
  await page.locator('.fm-canvas').waitFor();
  await setCanvasZoom(page, 110);

  const stage = page.locator('.le-stage');
  const workspace = page.locator('.le-workspace');
  await workspace.evaluate((element) => {
    element.style.minWidth = '1300px';
    element.style.minHeight = '700px';
  });
  await stage.evaluate((element) => {
    element.style.width = '800px';
    element.style.height = '420px';
    void element.offsetWidth;
    element.scrollLeft = 40;
    element.scrollTop = 20;
  });
  await page.waitForTimeout(100);
  assert.deepEqual(await stage.evaluate((element) => ({ left: element.scrollLeft, top: element.scrollTop })), {
    left: 40,
    top: 20,
  }, 'fixture uses nonzero horizontal and vertical scroll');

  const target = page.locator(`.fm-obj:not(.le-echo-ghost)[data-object-id="${targetId}"]`);
  const targetBox = await target.boundingBox();
  const stageBox = await stage.boundingBox();
  assert.ok(targetBox && stageBox, 'isolated marquee target is visible');

  // First cancel a harmless empty-space marquee. This exercises the private
  // Selecto/Gesto cleanup before the successful gesture below.
  const cancelStart = { x: stageBox.x + stageBox.width - 120, y: stageBox.y + stageBox.height - 120 };
  await page.mouse.move(cancelStart.x, cancelStart.y);
  await page.mouse.down();
  await page.mouse.move(cancelStart.x + 30, cancelStart.y + 20, { steps: 3 });
  assert.equal(await page.locator('.selecto-selection').isVisible(), true, 'cancel fixture paints a marquee');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  assert.equal(await page.locator('.selecto-selection').isVisible(), false, 'Escape clears the cancelled marquee');

  // Scroll after cancellation: stale Selecto DragScroll state used to throw here.
  await stage.evaluate((element) => { element.scrollTop += 20; });
  await page.waitForTimeout(50);
  const liveTargetBox = await target.boundingBox();
  const start = {
    x: liveTargetBox.x + liveTargetBox.width + 4,
    y: liveTargetBox.y + liveTargetBox.height + 4,
  };
  const end = { x: liveTargetBox.x - 4, y: liveTargetBox.y - 4 };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 5 });
  const marqueeBox = await page.locator('.selecto-selection').boundingBox();
  assert.ok(marqueeBox, 'successful marquee is visible during the gesture');
  assert.ok(
    Math.abs(marqueeBox.x - end.x) <= 1.01 && Math.abs(marqueeBox.y - end.y) <= 1.01,
    `marquee remains under the pointer: ${JSON.stringify({ start, end, marqueeBox })}`,
  );
  await page.locator(`#insp-left-${targetId}`).waitFor();
  await page.mouse.up();
  await page.waitForTimeout(100);

  assert.equal(await page.locator(`#insp-left-${targetId}`).inputValue(), '500',
    'normal pointer release preserves the exact marquee selection');
  assert.equal(await page.locator('.selecto-selection').isVisible(), false,
    'normal pointer release clears only the marquee visual');
  assert.equal(await stage.evaluate((element) => element.classList.contains('no-object-selection')), false,
    'normal pointer release leaves object selection active');
  assert.equal(await page.locator('.moveable-line').evaluateAll((elements) => elements.some((element) => {
    const rect = element.getBoundingClientRect();
    return getComputedStyle(element).display !== 'none' && (rect.width > 0 || rect.height > 0);
  })), true, 'normal pointer release exposes transform chrome');
  assert.deepEqual(pageErrors, [], `${browserEngine} emits no page errors`);

  console.log(`${browserEngine} marquee browser acceptance passed`);
} catch (error) {
  if (serverLog) console.error(serverLog);
  throw error;
} finally {
  if (browser) await browser.close();
  server.kill('SIGTERM');
  await rm(dataDir, { recursive: true, force: true });
}
