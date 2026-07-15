import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import playwright from 'playwright-core';

const uiDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoDir = resolve(uiDir, '..');
const port = Number(process.env.RM_BROWSER_PORT || 4331);
const externalBase = process.env.RM_BROWSER_BASE_URL;
const base = externalBase || `http://127.0.0.1:${port}`;
let server;
let dataDir;
let serverLog = '';

function capture(chunk) {
  serverLog = (serverLog + chunk.toString()).slice(-24_000);
}

async function waitForServer() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${base}/`);
      if (response.ok) return;
    } catch {}
    await new Promise((resolveWait) => setTimeout(resolveWait, 100));
  }
  throw new Error(`server did not start at ${base}`);
}

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
    body: JSON.stringify(value)
  });
}

async function canonicalTotal(layoutId) {
  const response = await fetch(`${base}/browse/${layoutId}`);
  const html = await response.text();
  const match = html.match(/class="rec-tnum">(\d+)</);
  assert.ok(match, 'canonical record total is rendered');
  return Number(match[1]);
}

async function dialog(page) {
  const element = page.locator('#app-dialog[open]');
  await element.waitFor({ state: 'visible' });
  return page.locator('#app-dialog-message').textContent();
}

async function returnToRecord(page, fieldName) {
  await page.locator('#app-dialog-cancel').click();
  await page.waitForFunction(() => !document.querySelector('#app-dialog')?.open);
  await page.waitForFunction(
    (name) => document.activeElement?.getAttribute('name') === name,
    fieldName
  );
}

async function run() {
  if (!externalBase) {
    dataDir = await mkdtemp(resolve(tmpdir(), 'record-maker-182-'));
    server = spawn('cargo', ['run', '-p', 'record-maker-server'], {
      cwd: repoDir,
      env: { ...process.env, RM_DATA_DIR: dataDir, RM_PORT: String(port) },
      stdio: ['ignore', 'pipe', 'pipe']
    });
    server.stdout.on('data', capture);
    server.stderr.on('data', capture);
  }
  await waitForServer();

  const invoices = await postJson('/schema/tables', {
    name: 'Browser Edit Sessions',
    notes: '',
    fields: [
      { name: 'Number', kind: 'text' },
      { name: 'Note', kind: 'text' }
    ]
  });
  const second = await postJson('/schema/tables', {
    name: 'Browser Exit Target',
    notes: '',
    fields: [{ name: 'Name', kind: 'text' }]
  });
  const fields = await request(`/schema/tables/${invoices.id}/fields`);
  const number = fields.find((field) => field.name === 'Number');
  const note = fields.find((field) => field.name === 'Note');
  await postJson(`/schema/tables/${invoices.id}/fields/${number.id}`, {
    name: 'Number',
    kind: 'text',
    options: { validation: { required: true, unique: true } }
  });
  const layouts = await request('/layouts/all');
  const layout = (tableId, view) => layouts.find(
    (candidate) => candidate.tableId === tableId && candidate.view === view
  ).id;
  const formLayout = layout(invoices.id, 'form');
  const listLayout = layout(invoices.id, 'list');
  const otherLayout = layout(second.id, 'form');

  const browser = await playwright.chromium.launch({
    headless: true,
    executablePath: process.env.CHROME_BIN || '/usr/bin/google-chrome'
  });
  const page = await browser.newPage();
  const pageErrors = [];
  page.on('pageerror', (error) => pageErrors.push(error.message));

  try {
    // A new record is an overlay only. Invalid navigation stays put, preserves
    // values, and returns focus; Revert then resumes the original navigation.
    await page.goto(`${base}/browse/${formLayout}`);
    await page.locator('#rm-new-record').click();
    await page.waitForURL((url) => url.searchParams.has('edit'));
    assert.equal(await canonicalTotal(formLayout), 0);
    await page.locator(`input[name="f${note.id}"]`).fill('kept working value');
    await page.locator(`.view-switch a[href="/browse/${listLayout}"]`).click();
    assert.match(await dialog(page), /Number.*required/i);
    assert.match(page.url(), /edit=/);
    assert.equal(await page.locator(`input[name="f${note.id}"]`).inputValue(), 'kept working value');
    await returnToRecord(page, `f${number.id}`);
    await page.locator(`.view-switch a[href="/browse/${listLayout}"]`).click();
    await dialog(page);
    await page.locator('#app-dialog-ok').click();
    await page.waitForURL((url) => url.pathname === `/browse/${listLayout}`);
    assert.equal(await canonicalTotal(formLayout), 0);

    // A valid pending record commits exactly once and becomes canonical.
    let commitRequests = 0;
    page.on('request', (req) => {
      if (req.method() === 'POST' && /\/browse\/\d+\/-?\d+$/.test(new URL(req.url()).pathname)) {
        commitRequests += 1;
      }
    });
    await page.locator('#rm-new-record').click();
    await page.waitForURL((url) => url.searchParams.has('edit'));
    await page.locator(`input[name="f${number.id}"]`).fill('INV-001');
    await page.locator(`input[name="f${note.id}"]`).fill('first');
    await page.locator(`.view-switch a[href="/browse/${formLayout}"]`).click();
    await page.waitForURL((url) => url.pathname === `/browse/${formLayout}` && !url.searchParams.has('edit'));
    assert.equal(commitRequests, 1, 'blur plus click performs one insert commit');
    assert.equal(await canonicalTotal(formLayout), 1);

    // Existing-record validation retains the working copy. Revert restores the
    // exact canonical value and then executes the pending exit intent.
    await page.locator(`input[name="f${number.id}"]`).fill('');
    await page.locator(`.view-switch a[href="/browse/${listLayout}"]`).click();
    assert.match(await dialog(page), /required/i);
    assert.equal(await page.locator(`input[name="f${number.id}"]`).inputValue(), '');
    await returnToRecord(page, `f${number.id}`);
    await page.locator(`.view-switch a[href="/browse/${listLayout}"]`).click();
    await dialog(page);
    await page.locator('#app-dialog-ok').click();
    await page.waitForURL((url) => url.pathname === `/browse/${listLayout}`);
    await page.goto(`${base}/browse/${formLayout}`);
    assert.equal(await page.locator(`input[name="f${number.id}"]`).inputValue(), 'INV-001');

    // Layout selection, mode shortcut, reload, New, and ordinary links all use
    // the same validation gate. Sampling the non-anchor paths here protects the
    // coordinator wiring as well as its core anchor path above.
    await page.locator(`input[name="f${number.id}"]`).fill('');
    await page.locator('[data-layout-select]').selectOption(String(otherLayout));
    assert.match(await dialog(page), /required/i);
    await returnToRecord(page, `f${number.id}`);
    await page.keyboard.press('Control+R');
    assert.match(await dialog(page), /required/i);
    await returnToRecord(page, `f${number.id}`);
    await page.locator('#rm-new-record').click();
    assert.match(await dialog(page), /required/i);
    await returnToRecord(page, `f${number.id}`);
    await page.keyboard.press('Control+L');
    assert.match(await dialog(page), /required/i);
    await returnToRecord(page, `f${number.id}`);
    await page.waitForTimeout(150);
    await page.evaluate(() => history.back());
    assert.match(await dialog(page), /required/i);
    await returnToRecord(page, `f${number.id}`);
    assert.equal(new URL(page.url()).pathname, `/browse/${formLayout}`);

    // Correcting the invalid value allows one update and one replayed intent.
    commitRequests = 0;
    await page.locator(`input[name="f${number.id}"]`).fill('INV-001');
    await page.locator(`input[name="f${note.id}"]`).fill('updated once');
    await page.locator(`.view-switch a[href="/browse/${listLayout}"]`).dblclick();
    await page.waitForURL((url) => url.pathname === `/browse/${listLayout}`);
    assert.equal(commitRequests, 1, 'double-click navigation performs one update commit');

    // Native close calls the same coordinator. Return cancels authorization;
    // after correction, one successful whole-record commit authorizes close.
    await page.goto(`${base}/browse/${formLayout}`);
    await page.locator(`input[name="f${number.id}"]`).fill('');
    let closeAuthorizations = 0;
    page.on('request', (req) => {
      if (new URL(req.url()).pathname === '/app/close-authorized') closeAuthorizations += 1;
    });
    await page.evaluate(() => { window.rmRequestClose(); });
    assert.match(await dialog(page), /required/i);
    await returnToRecord(page, `f${number.id}`);
    assert.equal(closeAuthorizations, 0, 'Return to record cancels native close');
    await page.locator(`input[name="f${number.id}"]`).fill('INV-001');
    const authorized = page.waitForResponse((response) =>
      new URL(response.url()).pathname === '/app/close-authorized'
    );
    await page.evaluate(() => { window.rmRequestClose(); });
    assert.equal((await authorized).status(), 204);
    assert.equal(closeAuthorizations, 1);
    assert.equal(pageErrors.length, 0, `page errors: ${pageErrors.join('; ')}`);

    console.log('record edit browser acceptance passed');
  } finally {
    await browser.close();
  }
}

try {
  await run();
} catch (error) {
  if (serverLog) console.error(serverLog);
  throw error;
} finally {
  if (server && !server.killed) server.kill('SIGTERM');
  if (dataDir) await rm(dataDir, { recursive: true, force: true });
}
