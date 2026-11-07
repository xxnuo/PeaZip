import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { access, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve, sep, extname } from 'node:path';
import { once } from 'node:events';
import { test } from 'node:test';
import { chromium, expect as playwrightExpect } from '@playwright/test';

const expect = playwrightExpect.configure({ timeout: 15_000 });
const root = fileURLToPath(new URL('../../target/web-dist/', import.meta.url));
const fixture = (name) => fileURLToPath(new URL(`./fixtures/${name}`, import.meta.url));

async function serve(t) {
  if (process.env.BASE_URL) return process.env.BASE_URL;
  await Promise.all(['index.html', 'nsh.js', 'peazip.wasm'].map((name) => access(resolve(root, name))));
  const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };
  const server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
      if (pathname === '/favicon.ico') {
        response.writeHead(204).end();
        return;
      }
      const path = resolve(root, `.${pathname === '/' ? '/index.html' : pathname}`);
      if (!path.startsWith(root.endsWith(sep) ? root : root + sep)) {
        response.writeHead(403).end();
        return;
      }
      const bytes = await readFile(path);
      response.writeHead(200, { 'Content-Type': types[extname(path)] || 'application/octet-stream' });
      response.end(bytes);
    } catch {
      response.writeHead(404).end();
    }
  });
  t.after(() => new Promise((done) => {
    server.close(done);
    server.closeAllConnections();
  }));
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return `http://127.0.0.1:${server.address().port}`;
}

test('Chromium runs WASM: open ZIP, select entries, recover from invalid ZIP', { timeout: 120_000 }, async (t) => {
  const baseURL = await serve(t);
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage();
  page.setDefaultTimeout(15_000);
  page.setDefaultNavigationTimeout(30_000);
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  t.after(() => assert.deepEqual(errors, [], 'browser console/pageerror must be clean'));
  const wasm = page.waitForResponse((response) => new URL(response.url()).pathname.endsWith('/peazip.wasm'));
  await page.goto(baseURL);
  assert.equal((await wasm).ok(), true, 'WASM must load successfully');
  await expect(page).toHaveTitle('PeaZip');
  const open = page.getByRole('button', { name: 'Open ZIP…', exact: true });
  const all = page.getByRole('button', { name: 'Select all', exact: true });
  const clear = page.getByRole('button', { name: 'Clear selection', exact: true });
  const selected = (count) => page.getByText(`${count} selected`, { exact: true });
  await expect(open).toBeEnabled();
  await expect(page.getByText('Open a ZIP archive to view its contents.', { exact: true })).toBeVisible();
  await expect(all).toBeDisabled();
  await expect(clear).toBeDisabled();
  await expect(selected(0)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Create ZIP…', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Extract selected…', exact: true })).toHaveCount(0);

  async function choose(name) {
    const chooser = page.waitForEvent('filechooser');
    await open.click();
    await (await chooser).setFiles(fixture(name));
    await expect(open).toBeEnabled();
  }

  async function rows() {
    await expect(page.getByText('3 entries', { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox')).toHaveCount(3);
    for (const [name, size] of [['你好.txt', '7'], ['notes.txt', '6'], ['空目录/', '—']]) {
      const row = page.getByRole('checkbox', { name, exact: true });
      await expect(row).toBeVisible();
      await expect(row.getByText(size, { exact: true })).toBeVisible();
    }
  }

  await choose('sample.zip');
  await rows();
  await expect(page.getByText('sample.zip', { exact: true })).toBeVisible();
  const chinese = page.getByRole('checkbox', { name: '你好.txt', exact: true });
  await chinese.click();
  await expect(chinese).toBeChecked();
  await expect(selected(1)).toBeVisible();
  await chinese.click();
  await expect(chinese).not.toBeChecked();
  await expect(selected(0)).toBeVisible();
  await all.click();
  await expect(selected(3)).toBeVisible();
  for (const row of await page.getByRole('checkbox').all()) await expect(row).toBeChecked();
  await clear.click();
  await expect(selected(0)).toBeVisible();
  for (const row of await page.getByRole('checkbox').all()) await expect(row).not.toBeChecked();
  await expect(clear).toBeDisabled();

  await chinese.click();
  await expect(selected(1)).toBeVisible();
  await choose('invalid.zip');
  await expect(page.getByText(/^Could not open ZIP:/)).toBeVisible();
  const oversized = page.waitForEvent('filechooser');
  await open.click();
  await (await oversized).setFiles({
    name: 'oversized.zip', mimeType: 'application/zip', buffer: Buffer.alloc(32 * 1024 * 1024 + 1),
  });
  await expect(page.getByText('Could not open ZIP: Archive exceeds the 32 MiB limit.', { exact: true })).toBeVisible();
  await expect(selected(1)).toBeVisible();
  await expect(open).toBeEnabled();
  await choose('sample.zip');
  await rows();
  await expect(selected(0)).toBeVisible();
  await all.click();
  await expect(selected(3)).toBeVisible();
  const emptyChooser = page.waitForEvent('filechooser');
  await open.click();
  // Empty input exercises the no-files result, not the native dialog's Cancel button.
  await (await emptyChooser).setFiles([]);
  await expect(open).toBeEnabled();
  await rows();
  await expect(selected(3)).toBeVisible();
  await clear.click();
  await expect(selected(0)).toBeVisible();
});
