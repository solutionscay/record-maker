import { chromium } from 'playwright-core';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const scriptRoot = dirname(fileURLToPath(import.meta.url));
export const uiRoot = resolve(scriptRoot, '..');

export async function createHarness({ outDir = '.monitor', viewport = { width: 1500, height: 950 } } = {}) {
  const OUT = process.env.MONITOR_OUT || resolve(uiRoot, outDir);
  const BASE = process.env.BASE || 'http://127.0.0.1:4317';
  const DESIGN = process.env.DESIGN_PATH || '/design/1';
  const CHROME = process.env.CHROME || '/usr/bin/google-chrome-stable';
  mkdirSync(OUT, { recursive: true });

  const obs = {};
  const step = async (name, fn) => {
    try {
      obs[name] = await fn();
    } catch (e) {
      obs[name] = { ERROR: String(e) };
    }
  };

  const browser = await chromium.launch({
    executablePath: CHROME,
    headless: true,
    args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage({ viewport });
  const consoleMsgs = [];
  page.on('console', (m) => consoleMsgs.push({ type: m.type(), text: m.text() }));
  page.on('pageerror', (e) => consoleMsgs.push({ type: 'pageerror', text: e.message }));

  async function waitForCanvas() {
    let up = false;
    for (let i = 0; i < 20 && !up; i++) {
      try {
        await page.goto(`${BASE}${DESIGN}`, { waitUntil: 'load', timeout: 5000 });
        await page.waitForSelector('.fm-canvas', { timeout: 2000 });
        up = true;
      } catch {
        await page.waitForTimeout(500);
      }
    }
    obs.serverUp = up;
    await page.waitForTimeout(400);
    return up;
  }

  function writeJson(name, value) {
    writeFileSync(resolve(OUT, name), JSON.stringify(value, null, 2));
  }

  return { OUT, BASE, DESIGN, CHROME, browser, page, obs, step, consoleMsgs, waitForCanvas, writeJson };
}
