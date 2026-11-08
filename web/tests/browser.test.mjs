import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { access, readFile, mkdtemp, rm, writeFile, truncate } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { resolve, sep, extname } from 'node:path';
import { once } from 'node:events';
import { inflateRawSync } from 'node:zlib';
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

async function bootPage(page, url) {
  await page.goto(url);
  await page.getByRole('button', { name: 'All entries', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Folder view', exact: true })).toBeEnabled();
}

async function navigationPage(t, viewport, colorScheme = 'light') {
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport, colorScheme, acceptDownloads: true });
  const errors = [], downloads = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
  page.on('download', (download) => downloads.push(download));
  t.after(() => assert.deepEqual(errors, []));
  await bootPage(page, await serve(t));
  const button = (name) => page.getByRole('button', { name, exact: true });
  await button('Folder view').click();
  await expect(button('All entries')).toBeEnabled();
  async function choose(name = 'sample.tar') {
    const pending = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await pending).setFiles(fileURLToPath(new URL(`../../tests/fixtures/${name}`, import.meta.url)));
    await expect(page.getByText(`${name === 'sample.7z' ? 6 : 5} entries`, { exact: true })).toBeVisible();
    await expect(button('Open archive…')).toBeEnabled();
  }
  const paths = () => page.getByRole('checkbox').evaluateAll((rows) => rows.map((row) => row.getAttribute('aria-label')));
  async function rows(expected) {
    await expect(page.getByRole('checkbox')).toHaveCount(expected.length);
    for (const name of expected) await expect(page.getByRole('checkbox', { name, exact: true })).toBeVisible();
    assert.deepEqual((await paths()).sort(), [...expected].sort());
  }
  return { page, button, choose, paths, rows, downloads };
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 360, height: 800 }]) {
  test(`Folder navigation preserves canonical selections, history and snapshots at ${viewport.width}px`, { timeout: 120_000 }, async (t) => {
    const { page, button, choose, rows, downloads } = await navigationPage(t, viewport);
    const path = page.getByRole('textbox', { name: 'Archive path', exact: true });
    await choose();
    await rows(['docs/', 'other.txt']);
    await expect(path).toHaveValue('/');
    for (const name of ['Up', 'Back', 'Forward']) await expect(button(name)).toBeDisabled();
    const snapshot = await downloadArchive(page, button('Download TAR'));
    await page.getByRole('checkbox', { name: 'other.txt', exact: true }).click();
    const openDocs = button('Open folder docs/');
    await expect(openDocs).toBeVisible();
    assert.equal(await openDocs.evaluate((node) => node.closest('[role="checkbox"]') === null), true);
    await openDocs.click();
    await expect(path).toHaveValue('docs/');
    await rows(['docs/empty/', 'docs/nested/', 'docs/你好.txt']);
    await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
    await button('Select all').click();
    await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
    await button('Clear selection').click();
    await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
    await page.getByRole('checkbox', { name: 'docs/nested/', exact: true }).click();
    await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
    await button('Open folder docs/nested/').click();
    await expect(path).toHaveValue('docs/nested/');
    await rows(['docs/nested/data.bin']);
    await expect(page.getByRole('checkbox', { name: 'docs/nested/data.bin', exact: true })).toBeChecked();
    await button('Clear selection').click();
    await button('Select all').click();
    await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
    await button('Up').click();
    await expect(path).toHaveValue('docs/');
    await button('Back').click();
    await expect(path).toHaveValue('docs/nested/');
    await button('Forward').click();
    await expect(path).toHaveValue('docs/');
    await button('Archive root').click();
    await expect(path).toHaveValue('/');
    await rows(['docs/', 'other.txt']);
    await expect(page.getByRole('checkbox', { name: 'other.txt', exact: true })).not.toBeChecked();
    await button('Back').click();
    await expect(path).toHaveValue('docs/');
    await button('Forward').click();
    await expect(path).toHaveValue('/');
    await path.fill('docs/nested/');
    await expect(path).toBeFocused();
    await path.press('Enter');
    await expect(path).toHaveValue('docs/nested/');
    await expect(path).not.toBeFocused();
    await rows(['docs/nested/data.bin']);
    await path.fill('missing/');
    await path.press('Enter');
    await expect(page.getByText('Archive directory not found', { exact: true })).toBeVisible();
    await expect(path).toBeFocused();
    await button('Go').click();
    await expect(page.getByText('Archive directory not found', { exact: true })).toBeVisible();
    await rows(['docs/nested/data.bin']);
    await expect(page.getByRole('checkbox', { name: 'docs/nested/data.bin', exact: true })).toBeChecked();
    await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
    assert.equal(downloads.length, 1, 'navigation must not start an archive download');
    assert.deepEqual(await downloadArchive(page, button('Download TAR')), snapshot);
    await path.fill('../');
    await button('Go').click();
    await expect(page.getByText('Invalid archive directory', { exact: true })).toBeVisible();
    await rows(['docs/nested/data.bin']);
    await expect(page.getByRole('checkbox', { name: 'docs/nested/data.bin', exact: true })).toBeChecked();
    await button('Back').click();
    await expect(path).toHaveValue('/');
    await button('Forward').click();
    await expect(path).toHaveValue('docs/nested/');
    await button('Archive root').click();
    await path.fill('docs');
    await path.press('Enter');
    await expect(path).toHaveValue('docs/');
    await expect(path).not.toBeFocused();
    await rows(['docs/empty/', 'docs/nested/', 'docs/你好.txt']);
    await button('Back').click();
    await expect(path).toHaveValue('/');
    await expect(button('Forward')).toBeEnabled();
    await path.fill('docs/empty/');
    await button('Go').click();
    await expect(path).toHaveValue('docs/empty/');
    await expect(path).not.toBeFocused();
    await rows([]);
    await expect(button('Select all')).toBeDisabled();
    await expect(button('Forward')).toBeDisabled();
    await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
    await button('Archive root').click();
    await button('All entries').click();
    await rows(['docs/', 'docs/empty/', 'docs/nested/data.bin', 'docs/你好.txt', 'other.txt']);
    await button('Clear selection').click();
    await button('Select all').click();
    await expect(page.getByText('5 selected', { exact: true })).toBeVisible();
    await choose('sample.7z');
    await expect(button('Folder view')).toBeEnabled();
    await rows(['docs/', 'docs/empty/', 'docs/nested/', 'docs/nested/data.bin', 'docs/你好.txt', 'other.txt']);
    await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
    await button('Folder view').click();
    await rows(['docs/', 'other.txt']);
    await expect(path).toHaveValue('/');
  });
}

test('Reopening the identical archive at root resets an invalid path and clears its error', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, rows } = await navigationPage(t, { width: 1024, height: 600 });
  const path = page.getByRole('textbox', { name: 'Archive path', exact: true });
  const error = page.getByText('Archive directory not found', { exact: true });
  await choose();
  await expect(path).toHaveValue('/');
  await rows(['docs/', 'other.txt']);
  await path.fill('missing/');
  await button('Go').click();
  await expect(error).toBeVisible();
  await expect(path).toHaveValue('missing/');
  await rows(['docs/', 'other.txt']);
  await choose();
  await expect(path).toHaveValue('/');
  await expect(error).toHaveCount(0);
  await rows(['docs/', 'other.txt']);
  for (const name of ['Up', 'Back', 'Forward']) await expect(button(name)).toBeDisabled();
});

test('Implicit directory selection includes multiple actual descendants and omits synthetic paths', { timeout: 120_000 }, async (t) => {
  const { page, button, rows } = await navigationPage(t, { width: 1024, height: 600 });
  const { stdout } = await promisify(execFile)('uv', ['run', 'python', '-c', `
import base64, io, zipfile
output = io.BytesIO()
with zipfile.ZipFile(output, 'w') as archive:
    for name in ['docs/deep/a.txt', 'docs/deep/b.txt', 'docs/c.txt', 'other.txt']:
        archive.writestr(name, name.encode())
print(base64.b64encode(output.getvalue()).decode())
`]);
  const pending = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await pending).setFiles({ name: 'implicit.zip', mimeType: 'application/zip', buffer: Buffer.from(stdout.trim(), 'base64') });
  await expect(page.getByText('4 entries', { exact: true })).toBeVisible();
  await rows(['docs/', 'other.txt']);
  await page.getByRole('checkbox', { name: 'docs/', exact: true }).click();
  await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
  await button('Open folder docs/').click();
  await rows(['docs/deep/', 'docs/c.txt']);
  await expect(page.getByRole('checkbox', { name: 'docs/deep/', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'docs/c.txt', exact: true })).toBeChecked();
  await page.getByRole('checkbox', { name: 'docs/deep/', exact: true }).click();
  await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
  await button('Open folder docs/deep/').click();
  await rows(['docs/deep/a.txt', 'docs/deep/b.txt']);
  for (const row of await page.getByRole('checkbox').all()) await expect(row).not.toBeChecked();
  await button('Select all').click();
  await expect(page.getByText('2 selected', { exact: true })).toBeVisible();
  await button('All entries').click();
  await rows(['docs/deep/a.txt', 'docs/deep/b.txt', 'docs/c.txt', 'other.txt']);
  await expect(page.getByRole('checkbox', { name: 'docs/c.txt', exact: true })).not.toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'other.txt', exact: true })).not.toBeChecked();
  for (const name of ['docs/deep/a.txt', 'docs/deep/b.txt']) await expect(page.getByRole('checkbox', { name, exact: true })).toBeChecked();
});

test('The desktop directory sidebar shows immediate folders and keeps an archive-root action', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, rows } = await navigationPage(t, { width: 1440, height: 900 });
  await choose();
  await expect(button('Browse folder /')).toBeVisible();
  await expect(button('Browse folder /docs/')).toBeVisible();
  await expect(button('Browse folder /docs/nested/')).toHaveCount(0);
  await expect(button('Browse folder /docs/empty/')).toHaveCount(0);
  await button('Browse folder /docs/').click();
  await rows(['docs/empty/', 'docs/nested/', 'docs/你好.txt']);
  await expect(button('Browse folder /')).toBeVisible();
  await expect(button('Browse folder /docs/empty/')).toBeVisible();
  await button('Browse folder /docs/nested/').click();
  await rows(['docs/nested/data.bin']);
  await expect(page.getByRole('button', { name: /^Browse folder \// })).toHaveCount(1);
  await button('Browse folder /').click();
  await rows(['docs/', 'other.txt']);
});

test('Name and size buttons sort both directions while preserving selection', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, paths, rows } = await navigationPage(t, { width: 1024, height: 600 });
  await choose();
  await button('All entries').click();
  const selected = page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true });
  await selected.click();
  const files = () => paths().then((names) => names.filter((name) => !name.endsWith('/')));
  await button('Size (bytes)').click();
  assert.deepEqual(await files(), ['docs/nested/data.bin', 'docs/你好.txt', 'other.txt']);
  await button('Size (bytes)').click();
  assert.deepEqual(await files(), ['other.txt', 'docs/你好.txt', 'docs/nested/data.bin']);
  await expect(selected).toBeChecked();
  await button('Folder view').click();
  await button('Open folder docs/').click();
  await rows(['docs/empty/', 'docs/nested/', 'docs/你好.txt']);
  await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
  await button('Archive root').click();
  const pending = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await pending).setFiles(fixture('sample.zip'));
  await expect(page.getByText('3 entries', { exact: true })).toBeVisible();
  const zipSelection = page.getByRole('checkbox', { name: '你好.txt', exact: true });
  await zipSelection.click();
  await button('Name').click();
  assert.deepEqual(await files(), ['notes.txt', '你好.txt']);
  await button('Name').click();
  assert.deepEqual(await files(), ['你好.txt', 'notes.txt']);
  await expect(zipSelection).toBeChecked();
});

for (const viewport of [{ width: 1024, height: 600 }, { width: 1440, height: 900 }, { width: 360, height: 800 }]) {
  for (const colorScheme of ['light', 'dark']) {
    test(`Folder view stays usable at ${viewport.width}x${viewport.height} in ${colorScheme} mode`, { timeout: 120_000 }, async (t) => {
      const { page, button, choose, rows } = await navigationPage(t, viewport, colorScheme);
      await choose();
      await rows(['docs/', 'other.txt']);
      await button('Open folder docs/').click();
      await rows(['docs/empty/', 'docs/nested/', 'docs/你好.txt']);
      await expect(page.getByRole('textbox', { name: 'Archive path', exact: true })).toHaveValue('docs/');
      for (const name of ['Up', 'Back', 'Archive root', 'Go', 'Name', 'Size (bytes)']) await expect(button(name)).toBeVisible();
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'folder view must fit the viewport without horizontal page scrolling');
      const screenshot = resolve(root, '..', `web-folder-${viewport.width}x${viewport.height}-${colorScheme}.png`);
      await page.screenshot({ path: screenshot, fullPage: true, animations: 'disabled' });
      t.diagnostic(`Screenshot: ${screenshot}`);
    });
  }
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
  await bootPage(page, baseURL);
  assert.equal((await wasm).ok(), true, 'WASM must load successfully');
  await expect(page).toHaveTitle('PeaZip');
  const open = page.getByRole('button', { name: 'Open archive…', exact: true });
  const all = page.getByRole('button', { name: 'Select all', exact: true });
  const clear = page.getByRole('button', { name: 'Clear selection', exact: true });
  const selected = (count) => page.getByText(`${count} selected`, { exact: true });
  await expect(open).toBeEnabled();
  await expect(page.getByText('Open an archive to view its contents.', { exact: true })).toBeVisible();
  await expect(all).toBeDisabled();
  await expect(clear).toBeDisabled();
  await expect(selected(0)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Extract…', exact: true })).toBeDisabled();

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
      await expect(row.getByLabel(size === '—' ? 'Size in bytes: not applicable' : `Size in bytes: ${size}`, { exact: true })).toBeVisible();
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
  await expect(page.getByText(/^Could not open archive:/)).toBeVisible();
  const oversized = page.waitForEvent('filechooser');
  await open.click();
  await (await oversized).setFiles({
    name: 'oversized.zip', mimeType: 'application/zip', buffer: Buffer.alloc(32 * 1024 * 1024 + 1),
  });
  await expect(page.getByText('Could not open archive: Archive exceeds the 32 MiB limit.', { exact: true })).toBeVisible();
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

test('Create dialog cancels without a picker and applies Stored and Deflate to real worker output', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await creationPage(t);
  const body = Buffer.from(Array.from({ length: 2000 }, (_, i) => `line ${i}: repeated archive content ${i % 17}\n`).join(''));
  const file = { name: 'payload.txt', mimeType: 'text/plain', buffer: body };
  await choose('Create archive…', [file]);
  await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: file.name, exact: true }).click();
  const original = await downloadArchive(page, button('Download ZIP'));
  let pickers = 0;
  page.on('filechooser', () => pickers++);
  for (const cancel of ['Cancel', 'Escape']) {
    await button('Create archive…').click();
    await expect(page.getByText('Create archive', { exact: true })).toBeVisible();
    if (cancel === 'Escape') await page.keyboard.press('Escape');
    else await page.getByRole('group', { name: 'Create archive', exact: true }).getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.getByText('Create archive', { exact: true })).toHaveCount(0);
    await expect(page.getByRole('checkbox', { name: file.name, exact: true })).toBeChecked();
    assert.deepEqual(await downloadArchive(page, button('Download ZIP')), original);
  }
  assert.equal(pickers, 0);
  for (const [choice, method, level] of [['Store (no compression)', 0, null], ['Deflate', 8, '1'], ['Deflate', 8, '9']]) {
    await button('Create archive…').click();
    await page.getByRole('radio', { name: choice, exact: true }).click();
    if (level) await page.getByRole('textbox', { name: 'Compression level', exact: true }).fill(level);
    const pending = page.waitForEvent('filechooser');
    await button('Continue').click();
    await (await pending).setFiles([file]);
    await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
    await expect(button('Create archive…')).toBeEnabled();
    checkSingleZip(await downloadArchive(page, button('Download ZIP')), method, body);
  }
  await page.getByRole('checkbox', { name: file.name, exact: true }).click();
  const beforeFailure = await downloadArchive(page, button('Download ZIP'));
  await button('Create archive…').click();
  const pending = page.waitForEvent('filechooser');
  await button('Continue').click();
  await (await pending).setFiles([{ name: 'high-ratio.txt', mimeType: 'text/plain', buffer: Buffer.alloc(1024 * 1024, 65) }]);
  await expect(page.getByText(/Could not create ZIP: .*compression ratio/)).toBeVisible();
  await expect(page.getByRole('checkbox', { name: file.name, exact: true })).toBeChecked();
  assert.deepEqual(await downloadArchive(page, button('Download ZIP')), beforeFailure);
  await button('Create archive…').click();
  await page.getByRole('radio', { name: 'Store (no compression)', exact: true }).click();
  const retry = page.waitForEvent('filechooser');
  await button('Continue').click();
  await (await retry).setFiles([{ name: 'high-ratio.txt', mimeType: 'text/plain', buffer: Buffer.alloc(1024 * 1024, 65) }]);
  await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
  checkSingleZip(await downloadArchive(page, button('Download ZIP')), 0, Buffer.alloc(1024 * 1024, 65));
});

test('Chromium creates, downloads and reopens ZIP using a real Rust worker', { timeout: 180_000 }, async (t) => {
  const directory = await mkdtemp(resolve(tmpdir(), 'peazip-browser-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage({ acceptDownloads: true });
  page.setDefaultTimeout(15_000);
  const errors = [], workers = [], downloads = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
  page.on('download', (download) => downloads.push(download));
  page.on('worker', (worker) => {
    if (!['list', 'test'].includes(new URL(worker.url()).searchParams.get('operation'))) workers.push({ worker, closed: once(worker, 'close') });
  });
  t.after(() => assert.deepEqual(errors, []));
  await bootPage(page, await serve(t));
  const create = page.getByRole('button', { name: 'Create archive…', exact: true });
  const open = page.getByRole('button', { name: 'Open archive…', exact: true });
  const download = page.getByRole('button', { name: 'Download ZIP', exact: true });
  await expect(create).toBeEnabled();
  await expect(download).toBeDisabled();
  const files = [
    { name: '你好.txt', mimeType: 'text/plain', buffer: Buffer.from('你好，ZIP！\n') },
    { name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('second file\n') },
    { name: '空文件.txt', mimeType: 'text/plain', buffer: Buffer.alloc(0) },
  ];
  async function choose(button, files) {
    const pending = page.waitForEvent('filechooser');
    await button.click();
    if (button === create) await page.getByRole('button', { name: 'Continue', exact: true }).click();
    const chooser = await pending;
    if (button === create) assert.equal(chooser.isMultiple(), true);
    await chooser.setFiles(files);
  }
  async function rows() {
    await expect(page.getByText('Archive.zip', { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox')).toHaveCount(files.length);
    for (const file of files) {
      const row = page.getByRole('checkbox', { name: file.name, exact: true });
      await expect(row).toBeVisible();
      await expect(row.getByLabel(`Size in bytes: ${file.buffer.length}`, { exact: true })).toBeVisible();
    }
  }
  async function released(index) {
    await expect.poll(() => workers.length).toBe(index + 1);
    assert.match(workers[index].worker.url(), /\/archive-worker\.js$/);
    await Promise.race([workers[index].closed, new Promise((_, reject) => {
      const timer = setTimeout(() => reject(new Error('worker was not released')), 15_000);
      timer.unref();
    })]);
    assert.equal(page.workers().length, 0);
  }
  let releaseWorker;
  const gate = new Promise((resolve) => { releaseWorker = resolve; });
  await page.route('**/archive-worker.js', async (route) => { await gate; await route.continue(); });
  const wasm = page.waitForResponse((response) => new URL(response.url()).pathname.endsWith('/worker-core_bg.wasm'));
  try {
    await choose(create, files);
    await expect(create).toBeDisabled();
    await expect(open).toBeDisabled();
    await expect(download).toBeDisabled();
  } finally { releaseWorker(); }
  assert.equal((await wasm).ok(), true);
  await expect(page.getByText('Created ZIP: 3 entries', { exact: true })).toBeVisible();
  await released(0);
  await page.unroute('**/archive-worker.js');
  await rows();
  await expect(download).toBeEnabled();
  assert.equal(downloads.length, 0, 'creation must not automatically download');
  const pending = page.waitForEvent('download');
  await download.click();
  const artifact = await pending;
  assert.equal(artifact.suggestedFilename(), 'Archive.zip');
  const zipPath = resolve(directory, artifact.suggestedFilename());
  await artifact.saveAs(zipPath);
  assert.equal(await artifact.failure(), null);
  const { stdout } = await promisify(execFile)('uv', ['run', 'python', '-c', `
import json, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as archive:
    assert archive.testzip() is None
    assert len(archive.infolist()) == 3
    actual = {item.filename: archive.read(item) for item in archive.infolist()}
    assert actual == {'你好.txt': '你好，ZIP！\\n'.encode(), 'notes.txt': b'second file\\n', '空文件.txt': b''}, actual
    print(json.dumps({name: len(data) for name, data in actual.items()}, ensure_ascii=False))
`, zipPath]);
  t.diagnostic(`Python zipfile verified: ${stdout.trim()}`);
  await choose(open, zipPath);
  await rows();
  const first = page.getByRole('checkbox', { name: files[0].name, exact: true });
  await first.click();
  await choose(create, []);
  await expect(create).toBeEnabled();
  await rows();
  await expect(first).toBeChecked();
  assert.equal(workers.length, 1, 'cancel must not start a worker');
  const oversized = resolve(directory, 'oversized.bin');
  await writeFile(oversized, '');
  await truncate(oversized, 16 * 1024 * 1024 + 1);
  await choose(create, oversized);
  await expect(page.getByText('Could not create ZIP: A file exceeds the 16 MiB limit.', { exact: true })).toBeVisible();
  await expect(create).toBeEnabled();
  await rows();
  await expect(first).toBeChecked();
  assert.equal(workers.length, 1, 'metadata rejection must not start a worker');
  const totalFiles = [];
  for (let index = 0; index < 5; index++) {
    const path = resolve(directory, `total-${index}.bin`);
    await writeFile(path, '');
    await truncate(path, index < 4 ? 16 * 1024 * 1024 : 1);
    totalFiles.push(path);
  }
  await choose(create, totalFiles);
  await expect(page.getByText('Could not create ZIP: Selected files exceed the 64 MiB total limit.', { exact: true })).toBeVisible();
  await expect(create).toBeEnabled();
  await rows();
  await expect(first).toBeChecked();
  assert.equal(workers.length, 1, 'total size rejection must not start a worker');
  await choose(create, Array.from({ length: 4097 }, (_, index) => ({
    name: `empty-${index}.txt`, mimeType: 'text/plain', buffer: Buffer.alloc(0),
  })));
  await expect(page.getByText('Could not create ZIP: Invalid number of files.', { exact: true })).toBeVisible();
  await expect(create).toBeEnabled();
  await rows();
  await expect(first).toBeChecked();
  assert.equal(workers.length, 1, 'file count rejection must not start a worker');
  await choose(create, [files[0], { ...files[0], buffer: Buffer.from('duplicate') }]);
  await expect(page.getByText(/^Could not create ZIP:.*duplicate or conflicting archive path:/)).toBeVisible();
  await released(1);
  await rows();
  await expect(first).toBeChecked();
  await expect(download).toBeEnabled();
  await choose(create, files);
  await expect(page.getByText('Created ZIP: 3 entries', { exact: true })).toBeVisible();
  await released(2);
  await rows();
  await expect(first).not.toBeChecked();
  await expect(create).toBeEnabled();
  await expect(download).toBeEnabled();
  for (let attempt = 0; attempt < 2; attempt++) {
    const index = workers.length;
    let releaseWasm;
    let intercepted = false;
    const pendingWasm = new Promise((resolve) => { releaseWasm = resolve; });
    await page.route('**/worker-core_bg.wasm', async (route) => {
      intercepted = true;
      await pendingWasm;
      await route.abort();
    });
    try {
      await first.click();
      await choose(create, files);
      await expect.poll(() => intercepted).toBe(true);
      await expect(create).toBeDisabled();
      await expect(download).toBeDisabled();
      await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })));
      await expect(create).toBeEnabled();
      await expect(open).toBeEnabled();
      await expect(download).toBeEnabled();
      await released(index);
      await rows();
      await expect(first).toBeChecked();
    } finally {
      releaseWasm();
      await page.unrouteAll({ behavior: 'wait' });
    }
    await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true })));
    await choose(create, files);
    await expect(page.getByText('Created ZIP: 3 entries', { exact: true })).toBeVisible();
    await released(index + 1);
    await rows();
    await expect(first).not.toBeChecked();
    t.diagnostic(`persisted pagehide cancellation ${attempt + 1}: busy cleared, worker released, subsequent creation succeeded`);
  }
  assert.equal(downloads.length, 1);
  t.diagnostic(`${workers.length} real archive workers observed and released; cancel/oversize started none`);
});

for (const [width, height, colorScheme] of [[1024, 600, 'light'], [1440, 900, 'dark'], [360, 800, 'light'], [430, 800, 'dark']]) {
  test(`Create dialog remains usable at ${width}x${height} ${colorScheme}`, { timeout: 120_000 }, async (t) => {
    const { page, button } = await creationPage(t, { viewport: { width, height }, colorScheme, isMobile: width < 600, hasTouch: width < 600 });
    await button('Create archive…').click();
    const dialog = page.getByRole('group', { name: 'Create archive', exact: true });
    await expect(dialog).toBeVisible();
    await dialog.getByRole('radio', { name: 'Deflate', exact: true }).click();
    await dialog.getByRole('textbox', { name: 'Compression level', exact: true }).fill('0');
    await expect(dialog.getByRole('button', { name: 'Continue', exact: true })).toBeDisabled();
    await dialog.getByRole('textbox', { name: 'Compression level', exact: true }).fill('6');
    await expect(dialog.getByRole('button', { name: 'Continue', exact: true })).toBeInViewport();
    await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeInViewport();
    const box = await dialog.boundingBox();
    assert.ok(box.x >= 0 && box.x + box.width <= width + 1);
    const screenshot = fileURLToPath(new URL(`../../target/create-dialog-${width}-${colorScheme}.png`, import.meta.url));
    await page.screenshot({ path: screenshot });
    t.diagnostic(`Screenshot: ${screenshot}`);
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(button('Create archive…')).toBeEnabled();
  });
}

function checkSingleZip({ bytes }, method, expected) {
  const end = bytes.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  assert.ok(end >= 0);
  assert.equal(bytes.readUInt16LE(end + 10), 1);
  const central = bytes.readUInt32LE(end + 16);
  assert.equal(bytes.readUInt32LE(central), 0x02014b50);
  assert.equal(bytes.readUInt16LE(central + 10), method);
  const compressed = bytes.readUInt32LE(central + 20);
  const local = bytes.readUInt32LE(central + 42);
  assert.equal(bytes.readUInt32LE(local), 0x04034b50);
  assert.equal(bytes.readUInt16LE(local + 8), method);
  const start = local + 30 + bytes.readUInt16LE(local + 26) + bytes.readUInt16LE(local + 28);
  const payload = bytes.subarray(start, start + compressed);
  assert.deepEqual(method === 0 ? payload : inflateRawSync(payload), expected);
  return compressed;
}

test('TAR and TAR.GZ creation uses real workers, matching filenames and MIME, and independent readers', { timeout: 180_000 }, async (t) => {
  const { page, button, downloads } = await creationPage(t);
  const directory = await mkdtemp(resolve(tmpdir(), 'peazip-created-formats-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const files = [
    { name: '你好.txt', mimeType: 'text/plain', buffer: Buffer.from('created 中文\n') },
    { name: 'binary.bin', mimeType: 'application/octet-stream', buffer: Buffer.from([0, 127, 255, 1]) },
    { name: 'empty', mimeType: 'application/octet-stream', buffer: Buffer.alloc(0) },
  ];
  const workerKinds = [];
  page.on('worker', worker => workerKinds.push(new URL(worker.url()).searchParams.get('operation')));
  await page.evaluate(() => {
    window.createdMimes = [];
    const create = URL.createObjectURL.bind(URL);
    URL.createObjectURL = value => { window.createdMimes.push(value.type); return create(value); };
  });
  for (const [format, extension, mime] of [['TAR', 'tar', 'application/x-tar'], ['TAR.GZ', 'tar.gz', 'application/gzip']]) {
    await button('Create archive…').click();
    const modal = page.getByRole('group', { name: 'Create archive', exact: true });
    await modal.getByRole('radio', { name: format, exact: true }).click();
    if (format === 'TAR.GZ') {
      const level = modal.getByRole('textbox', { name: 'GZIP compression level', exact: true });
      await level.fill('0');
      await expect(modal.getByRole('button', { name: 'Continue', exact: true })).toBeDisabled();
      await level.fill('9');
    } else {
      await expect(modal.getByRole('textbox', { name: 'Compression level', exact: true })).toHaveCount(0);
    }
    const pending = page.waitForEvent('filechooser');
    await modal.getByRole('button', { name: 'Continue', exact: true }).click();
    await (await pending).setFiles(files);
    await expect(page.getByText(`Created ${format}: 3 entries`, { exact: true })).toBeVisible();
    await expect(button(`Download ${format}`)).toBeEnabled();
    const artifact = await downloadArchive(page, button(`Download ${format}`));
    assert.equal(artifact.name, `Archive.${extension}`);
    assert.equal((await page.evaluate(() => window.createdMimes)).at(-1), mime);
    const path = resolve(directory, artifact.name);
    await writeFile(path, artifact.bytes);
    const { stdout } = await promisify(execFile)('uv', ['run', 'python', '-c', `
import gzip, json, sys, tarfile
path = sys.argv[1]
with tarfile.open(path, 'r:*') as archive:
    actual = {item.name: archive.extractfile(item).read() for item in archive if item.isfile()}
    assert actual == {'你好.txt': 'created 中文\\n'.encode(), 'binary.bin': bytes([0,127,255,1]), 'empty': b''}, actual
    assert all(item.mtime == 0 and item.uid == 0 and item.gid == 0 for item in archive)
if path.endswith('.gz'):
    raw = gzip.decompress(open(path, 'rb').read())
    assert raw[-1024:] == bytes(1024)
print(json.dumps({name: len(data) for name, data in actual.items()}, ensure_ascii=False))
`, path]);
    t.diagnostic(`Independent ${format}: ${stdout.trim()}`);
    const reopen = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await reopen).setFiles(path);
    await expect(page.getByText('3 entries', { exact: true })).toBeVisible();
    await expect(button(`Download ${format}`)).toBeEnabled();
    assert.deepEqual(await downloadArchive(page, button(`Download ${format}`)), artifact);
  }
  assert.equal(workerKinds.filter(kind => kind === 'list').length, 4, 'each created archive and reopened file must use a real metadata worker');
  await expect.poll(() => page.workers().length).toBe(0);
  assert.equal(downloads.length, 4);
});

test('Strict gzip creation failure preserves the previous archive and can retry as TAR', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await creationPage(t);
  await choose('Create archive…', [{ name: 'keep.txt', mimeType: 'text/plain', buffer: Buffer.from('keep original') }]);
  await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: 'keep.txt', exact: true }).click();
  const original = await downloadArchive(page, button('Download ZIP'));
  await button('Create archive…').click();
  await page.getByRole('radio', { name: 'TAR.GZ', exact: true }).click();
  await page.getByRole('textbox', { name: 'GZIP compression level', exact: true }).fill('9');
  const pending = page.waitForEvent('filechooser');
  await button('Continue').click();
  await (await pending).setFiles([{ name: 'zeros.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(1024 * 1024) }]);
  await expect(page.getByText(/^Could not create TAR.GZ:.*compression ratio/)).toBeVisible();
  await expect(page.getByRole('checkbox', { name: 'keep.txt', exact: true })).toBeChecked();
  assert.deepEqual(await downloadArchive(page, button('Download ZIP')), original);
  await button('Create archive…').click();
  await expect(page.getByRole('radio', { name: 'TAR.GZ', exact: true })).toBeChecked();
  await page.getByRole('radio', { name: 'TAR', exact: true }).click();
  const retry = page.waitForEvent('filechooser');
  await button('Continue').click();
  await (await retry).setFiles([{ name: 'zeros.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(1024 * 1024) }]);
  await expect(page.getByText('Created TAR: 1 entries', { exact: true })).toBeVisible();
  const created = await downloadArchive(page, button('Download TAR'));
  assert.equal(created.name, 'Archive.tar');
  assert.equal(created.bytes.subarray(512, 512 + 1024 * 1024).every(value => value === 0), true);
});

for (const cancellation of ['Cancel', 'pagehide']) {
  test(`TAR.GZ creation ${cancellation} during the follow-up metadata worker preserves the old snapshot`, { timeout: 120_000 }, async (t) => {
    const { page, button, choose } = await creationPage(t);
    await choose('Create archive…', [{ name: 'keep.txt', mimeType: 'text/plain', buffer: Buffer.from('old snapshot') }]);
    await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
    await page.getByRole('checkbox', { name: 'keep.txt', exact: true }).click();
    const original = await downloadArchive(page, button('Download ZIP'));
    let requests = 0, blocked = false, release;
    const gate = new Promise(resolve => { release = resolve; });
    await page.route('**/worker-core_bg.wasm', async route => {
      if (++requests === 2) { blocked = true; await gate; }
      await route.continue().catch(() => {});
    });
    try {
      await button('Create archive…').click();
      await page.getByRole('radio', { name: 'TAR.GZ', exact: true }).click();
      const picker = page.waitForEvent('filechooser');
      await button('Continue').click();
      await (await picker).setFiles([{ name: 'new.txt', mimeType: 'text/plain', buffer: Buffer.from('new contents') }]);
      await expect.poll(() => blocked).toBe(true);
      await expect(page.getByText('Listing created archive…', { exact: true })).toBeVisible();
      if (cancellation === 'Cancel') await button('Cancel').click();
      else await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })));
      await expect(button('Create archive…')).toBeEnabled();
      await expect(page.getByRole('checkbox', { name: 'keep.txt', exact: true })).toBeChecked();
      await expect.poll(() => page.workers().length).toBe(0);
    } finally {
      release();
      await page.unrouteAll({ behavior: 'wait' });
    }
    if (cancellation === 'pagehide') await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true })));
    assert.deepEqual(await downloadArchive(page, button('Download ZIP')), original);
    await choose('Create archive…', [{ name: 'retry.txt', mimeType: 'text/plain', buffer: Buffer.from('retry contents') }]);
    await expect(page.getByText('Created TAR.GZ: 1 entries', { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox', { name: 'retry.txt', exact: true })).toBeVisible();
    await expect(button('Download TAR.GZ')).toBeEnabled();
  });
}

async function creationPage(t, contextOptions = {}) {
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage({ acceptDownloads: true, ...contextOptions });
  const errors = [], downloads = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('download', (download) => downloads.push(download));
  t.after(() => assert.deepEqual(errors, []));
  await page.addInitScript(() => {
    window.archiveWorkers = [];
    const NativeWorker = window.Worker;
    window.Worker = class extends NativeWorker {
      constructor(...args) {
        super(...args);
        if (['list', 'test'].includes(new URL(args[0], location.href).searchParams.get('operation'))) return;
        const record = { messages: [], terminated: false };
        window.archiveWorkers.push(record);
        this.addEventListener('message', ({ data }) => {
          record.messages.push(data.progress ? { progress: data.progress } : { code: data.code });
        });
        const terminate = this.terminate.bind(this);
        this.terminate = () => { record.terminated = true; terminate(); };
        const descriptor = Object.getOwnPropertyDescriptor(NativeWorker.prototype, 'onmessage');
        Object.defineProperty(this, 'onmessage', {
          set(handler) { record.handler = handler; descriptor.set.call(this, handler); },
          get() { return descriptor.get.call(this); },
        });
      }
    };
    window.creationStages = [];
    new MutationObserver(() => {
      const text = document.body?.innerText ?? '';
      for (const stage of text.match(/Reading files: \d+\/\d+|Compressing ZIP…/g) ?? []) {
        if (window.creationStages.at(-1) !== stage) window.creationStages.push(stage);
      }
    }).observe(document, { subtree: true, childList: true, characterData: true });
  });
  await bootPage(page, await serve(t));
  const button = (name) => page.getByRole('button', { name, exact: true });
  async function choose(name, files) {
    const pending = page.waitForEvent('filechooser');
    await button(name).click();
    if (name === 'Create archive…') await button('Continue').click();
    await (await pending).setFiles(files);
  }
  await expect(button('Create archive…')).toBeEnabled();
  return { page, button, choose, downloads };
}

test('Long names remain accessible and render on one line without breaking navigation at 360px', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, downloads } = await creationPage(t);
  await page.setViewportSize({ width: 360, height: 800 });
  const name = '长'.repeat(70) + '.verylongextension'.repeat(3);
  const archiveName = '归档'.repeat(70) + '.misnamed.tar';
  assert.ok(Buffer.byteLength(name) < 1024);
  async function singleLine(locator, fullText) {
    await expect(locator).toHaveCount(1);
    await expect(locator).toBeVisible();
    const layout = await locator.evaluate((element) => {
      const range = document.createRange();
      range.selectNodeContents(element);
      const rects = Array.from(range.getClientRects()).filter((rect) => rect.width && rect.height);
      const bounds = element.getBoundingClientRect();
      return { text: element.textContent, lines: [...new Set(rects.map((rect) => Math.round(rect.top)))], left: bounds.left, right: bounds.right };
    });
    assert.ok(layout.text.length > 0 && layout.text.length < fullText.length, 'the visible title must be truncated');
    assert.equal(layout.lines.length, 1, 'the title must occupy one rendered line');
    assert.ok(layout.left >= 0 && layout.right <= 361, 'the title must fit inside the narrow viewport');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'long names must not cause horizontal page scrolling');
  }
  await choose('Create archive…', [{ name, mimeType: 'text/plain', buffer: Buffer.from('long filename payload') }]);
  await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.archiveWorkers.map(({ terminated, messages }) => ({ terminated, code: messages.at(-1)?.code })))).toEqual([{ terminated: true, code: 1 }]);
  const row = page.getByRole('checkbox', { name, exact: true });
  await expect(row).toHaveAttribute('aria-label', name);
  await singleLine(row.getByText(/^长/u), name);
  await row.click();
  await expect(row).toBeChecked();
  assert.equal(downloads.length, 0);
  const zip = await downloadArchive(page, button('Download ZIP'));
  await choose('Open archive…', { name: archiveName, mimeType: 'application/x-tar', buffer: zip.bytes });
  await expect(page.getByText('1 entries', { exact: true })).toBeVisible();
  await expect(row).toHaveAttribute('aria-label', name);
  await expect(row).not.toBeChecked();
  await singleLine(row.getByText(/^长/u), name);
  await singleLine(page.getByText(/^归档/u), archiveName);
  await expect(button('Download ZIP')).toBeEnabled();
  await button('Name').click();
  await button('Size (bytes)').click();
  await button('Folder view').click();
  const path = page.getByRole('textbox', { name: 'Archive path', exact: true });
  await path.fill('/');
  await button('Go').click();
  await expect(path).toHaveValue('/');
  await expect(path).not.toBeFocused();
  await singleLine(row.getByText(/^长/u), name);
  await choose('Open archive…', {
    name: archiveName, mimeType: 'application/zip',
    buffer: await readFile(new URL('../../tests/fixtures/sample.tar', import.meta.url)),
  });
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await button('Open folder docs/').click();
  await expect(path).toHaveValue('docs/');
  await button('Up').click();
  await expect(path).toHaveValue('/');
  await button('Back').click();
  await expect(path).toHaveValue('docs/');
  await button('Forward').click();
  await expect(path).toHaveValue('/');
  await singleLine(page.getByText(/^归档/u), archiveName);
  assert.equal(downloads.length, 1, 'only the explicit archive download may run');
});

test('Cancel during real worker WASM loading preserves snapshot; retry ignores stale callbacks', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, downloads } = await creationPage(t);
  const cancel = button('Cancel');
  await expect(cancel).toBeDisabled();
  await choose('Create archive…', [{ name: 'original.txt', mimeType: 'text/plain', buffer: Buffer.from('original snapshot') }]);
  await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
  const original = page.getByRole('checkbox', { name: 'original.txt', exact: true });
  await original.click();
  async function snapshot() {
    const pending = page.waitForEvent('download');
    await button('Download ZIP').click();
    const stream = await (await pending).createReadStream();
    const chunks = [];
    for await (const chunk of stream) chunks.push(chunk);
    return Buffer.concat(chunks);
  }
  const before = await snapshot();
  const downloadsBefore = downloads.length;
  let release, intercepted = false;
  const gate = new Promise((resolve) => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async (route) => {
    intercepted = true;
    await gate;
    await route.continue().catch(() => {});
  });
  try {
    await choose('Create archive…', [{ name: 'cancelled.txt', mimeType: 'text/plain', buffer: Buffer.from('cancel me') }]);
    await expect.poll(() => intercepted).toBe(true);
    await expect(cancel).toBeEnabled();
    for (const name of ['Create archive…', 'Open archive…', 'Download ZIP', 'Select all', 'Invert selection', 'Clear selection']) await expect(button(name)).toBeDisabled();
    await cancel.click();
    await expect(page.getByText('Creation cancelled.', { exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => window.archiveWorkers[1].terminated)).toBe(true);
    assert.deepEqual(await page.evaluate(() => window.archiveWorkers[1].messages), [], 'cancel reached WASM initialization before file reading');
    await expect(original).toBeChecked();
    await expect(cancel).toBeDisabled();
    await expect(button('Create archive…')).toBeEnabled();
    assert.equal(downloads.length, downloadsBefore, 'cancellation must not download');
    assert.deepEqual(await snapshot(), before, 'cancel must preserve exact original archive bytes');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
  }
  await choose('Create archive…', [{ name: 'retry.txt', mimeType: 'text/plain', buffer: Buffer.from('retry succeeds') }]);
  await expect(page.getByRole('checkbox', { name: 'retry.txt', exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: 'retry.txt', exact: true }).click();
  const retry = await snapshot();
  // Only this stale-callback check injects messages; progress tests observe real Worker messages.
  await page.evaluate(() => {
    const old = window.archiveWorkers[1].handler;
    old({ data: { progress: { phase: 'read', done: 1, total: 9 } } });
    old({ data: { code: 1, bytes: new Uint8Array([0, 1, 2]) } });
  });
  await expect(page.getByText('Archive download requested.', { exact: true })).toBeVisible();
  await expect(page.getByRole('checkbox', { name: 'retry.txt', exact: true })).toBeVisible();
  await expect(page.getByRole('checkbox', { name: 'retry.txt', exact: true })).toBeChecked();
  assert.deepEqual(await snapshot(), retry, 'stale success cannot overwrite retry snapshot');
  await expect(cancel).toBeDisabled();
  t.diagnostic('Real worker WASM request held before initialization; Cancel terminated it, preserved ZIP bytes/selection, and retry survived stale callbacks.');
});

test('Real worker reports bounded read counts then compression and UI shows actual phases', { timeout: 180_000 }, async (t) => {
  const { page, button, choose, downloads } = await creationPage(t);
  const directory = await mkdtemp(resolve(tmpdir(), 'peazip-progress-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const { randomBytes } = await import('node:crypto');
  const paths = [];
  for (let index = 0; index < 3; index++) {
    const path = resolve(directory, `random-${index}.bin`);
    await writeFile(path, randomBytes(8 * 1024 * 1024));
    paths.push(path);
  }
  await choose('Create archive…', paths);
  await expect(page.getByText('Created ZIP: 3 entries', { exact: true })).toBeVisible({ timeout: 120_000 });
  const messages = await page.evaluate(() => window.archiveWorkers[0].messages);
  assert.deepEqual(messages, [
    { progress: { phase: 'read', done: 1, total: 3 } },
    { progress: { phase: 'read', done: 2, total: 3 } },
    { progress: { phase: 'read', done: 3, total: 3 } },
    { progress: { phase: 'compress', done: 3, total: 3 } },
    { code: 1 },
  ]);
  const stages = await page.evaluate(() => window.creationStages);
  assert.ok(stages.some((stage) => /^Reading files: [1-3]\/3$/.test(stage)), JSON.stringify(stages));
  assert.ok(stages.includes('Compressing ZIP…'), JSON.stringify(stages));
  assert.ok(stages.findIndex((stage) => stage.startsWith('Reading files:')) < stages.indexOf('Compressing ZIP…'));
  await expect(button('Cancel')).toBeDisabled();
  assert.equal(downloads.length, 0);

  const selected = page.getByRole('checkbox', { name: 'random-0.bin', exact: true });
  await selected.click();
  async function snapshot() {
    const pending = page.waitForEvent('download');
    await button('Download ZIP').click();
    const stream = await (await pending).createReadStream();
    const chunks = [];
    for await (const chunk of stream) chunks.push(chunk);
    return Buffer.concat(chunks);
  }
  const before = await snapshot();
  const downloadsBefore = downloads.length;
  await choose('Create archive…', paths);
  await expect(page.getByText('Compressing ZIP…', { exact: true })).toBeVisible();
  await button('Cancel').click();
  await expect(page.getByText('Creation cancelled.', { exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.archiveWorkers[1].terminated)).toBe(true);
  await expect.poll(() => page.workers().length).toBe(0);
  const cancelled = await page.evaluate(() => window.archiveWorkers[1].messages);
  assert.deepEqual(cancelled, messages.slice(0, -1), 'real compression must be interrupted before any success result');
  assert.equal(downloads.length, downloadsBefore, 'CPU cancellation must not download');
  await expect(selected).toBeChecked();
  await expect(button('Cancel')).toBeDisabled();
  await expect(button('Create archive…')).toBeEnabled();
  assert.deepEqual(await snapshot(), before, 'CPU cancellation must preserve exact prior ZIP bytes');
  await expect(selected).toBeChecked();
  await choose('Create archive…', paths);
  await expect(page.getByText('Created ZIP: 3 entries', { exact: true })).toBeVisible({ timeout: 120_000 });
  await expect(selected).not.toBeChecked();
  assert.deepEqual(await page.evaluate(() => window.archiveWorkers[2].messages), messages, 'retry must finish actual compression');
  assert.equal(downloads.length, downloadsBefore + 1, 'retry must not automatically download');
  t.diagnostic(`Real CPU cancellation: ${JSON.stringify(cancelled)}; worker terminated without success; prior ZIP bytes/selection preserved; same 24 MiB retry succeeded.`);

  const count = 251;
  await choose('Create archive…', Array.from({ length: count }, (_, index) => ({
    name: `empty-${index}.txt`, mimeType: 'text/plain', buffer: Buffer.alloc(0),
  })));
  await expect(page.getByText(`Created ZIP: ${count} entries`, { exact: true })).toBeVisible();
  const throttled = await page.evaluate(() => window.archiveWorkers[3].messages);
  const reads = throttled.filter((message) => message.progress?.phase === 'read').map(({ progress }) => progress);
  assert.ok(reads.length > 0 && reads.length <= 100);
  assert.equal(reads.at(-1).done, count);
  for (let index = 0; index < reads.length; index++) {
    assert.equal(reads[index].total, count);
    assert.ok(reads[index].done > (reads[index - 1]?.done ?? 0));
    assert.ok(reads[index].done <= count);
  }
  assert.deepEqual(throttled.slice(-2), [{ progress: { phase: 'compress', done: count, total: count } }, { code: 1 }]);
  assert.ok(throttled.slice(0, -2).every((message) => message.progress?.phase === 'read'));
  assert.equal(downloads.length, downloadsBefore + 1);
  t.diagnostic(`Unmodified worker: 24 MiB real compression; UI phases ${JSON.stringify(stages)}; ${count} files emitted ${reads.length} read messages, then compress, then success.`);
});

async function extractionPage(t, available = true, options = {}) {
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage(options);
  const errors = [], downloads = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('download', (download) => downloads.push(download));
  t.after(() => assert.deepEqual(errors, []));
  await page.addInitScript(({ available }) => {
    window.extractionWorkers = [];
    window.listWorkers = [];
    window.listWorkerHandlers = [];
    window.testWorkers = [];
    window.testWorkerHandlers = [];
    const NativeWorker = window.Worker;
    window.Worker = class extends NativeWorker {
      constructor(...args) {
        super(...args);
        const record = { terminated: false, codes: [] };
        const operation = new URL(args[0], location.href).searchParams.get('operation');
        const records = operation === 'list' ? window.listWorkers : operation === 'test' ? window.testWorkers : window.extractionWorkers;
        const handlers = operation === 'list' ? window.listWorkerHandlers : window.testWorkerHandlers;
        const index = records.push(record) - 1;
        if (operation === 'list' || operation === 'test') {
          const descriptor = Object.getOwnPropertyDescriptor(NativeWorker.prototype, 'onmessage');
          Object.defineProperty(this, 'onmessage', {
            set(handler) { handlers[index] = handler; descriptor.set.call(this, handler); },
            get() { return descriptor.get.call(this); },
          });
        }
        this.addEventListener('message', ({ data }) => {
          if (!data.progress) record.codes.push(data.code);
        });
        const terminate = this.terminate.bind(this);
        this.terminate = () => { record.terminated = true; terminate(); };
      }
    };
    window.pickerCalls = [];
    window.pickerFailure = null;
    window.testDirectoryName = `peazip-browser-${crypto.randomUUID()}`;
    // Only the picker is substituted: all handles and writes use real OPFS APIs.
    window.showDirectoryPicker = available ? async (options) => {
      window.pickerCalls.push({ mode: options.mode, active: navigator.userActivation.isActive });
      if (window.pickerFailure) throw new DOMException('Test picker rejection', window.pickerFailure);
      return (await navigator.storage.getDirectory()).getDirectoryHandle(window.testDirectoryName, { create: true });
    } : undefined;
  }, { available });
  await bootPage(page, await serve(t));
  const button = (name) => page.getByRole('button', { name, exact: true });
  await expect(button('Open archive…')).toBeEnabled();
  await expect(button('Extract…')).toBeDisabled();
  await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const directory = await root.getDirectoryHandle(window.testDirectoryName, { create: true });
    const file = await directory.getFileHandle('sentinel.txt', { create: true });
    const writer = await file.createWritable();
    await writer.write('leave me unchanged');
    await writer.close();
  });
  t.after(async () => {
    if (!page.isClosed()) await page.evaluate(async () => {
      await (await navigator.storage.getDirectory()).removeEntry(window.testDirectoryName, { recursive: true });
    });
  });
  async function choose(files = fixture('sample.zip')) {
    const pending = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await pending).setFiles(files);
    await expect(page.getByText('3 entries', { exact: true })).toBeVisible();
    await expect(button('Open archive…')).toBeEnabled();
  }
  async function tree() {
    return page.evaluate(async () => {
      async function read(directory) {
        const result = {};
        for await (const [name, handle] of directory.entries()) {
          result[name] = handle.kind === 'directory' ? await read(handle) :
            Array.from(new Uint8Array(await (await handle.getFile()).arrayBuffer()));
        }
        return result;
      }
      return read(await (await navigator.storage.getDirectory()).getDirectoryHandle(window.testDirectoryName));
    });
  }
  return { page, button, choose, tree, downloads };
}

async function menuPage(t, options = {}) {
  const context = await extractionPage(t, true, options);
  const { page, button } = context;
  const popup = page.getByRole('menu');
  async function openMenu(group) {
    await button(group).click();
    await expect(popup).toHaveCount(1);
    await expect(popup).toBeVisible();
    return popup;
  }
  async function menuAction(group, name) {
    await openMenu(group);
    await popup.getByRole('menuitem', { name, exact: true }).click();
    await expect(popup).toHaveCount(0);
  }
  return { ...context, popup, openMenu, menuAction };
}

test('Application menus share archive, hierarchy, layout and keyboard commands', { timeout: 120_000 }, async (t) => {
  const { page, button, popup, openMenu, menuAction, downloads } = await menuPage(t, { viewport: { width: 1440, height: 900 } });
  for (const group of ['File', 'Edit', 'Browser', 'Layout', 'Tools', 'Help']) await expect(button(group)).toBeVisible();
  for (const group of ['Organize', 'Options']) await expect(button(group)).toHaveCount(0);
  await openMenu('File');
  await expect(popup.getByRole('menuitem', { name: 'Open archive…', exact: true })).toBeEnabled();
  for (const name of ['Download archive', 'Close archive']) await expect(popup.getByRole('menuitem', { name, exact: true })).toBeDisabled();
  const picked = page.waitForEvent('filechooser');
  await popup.getByRole('menuitem', { name: 'Open archive…', exact: true }).click();
  await (await picked).setFiles(fileURLToPath(new URL('../../tests/fixtures/sample.tar', import.meta.url)));
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await expect(popup).toHaveCount(0);
  await button('Folder view').click();
  await expect(page.getByRole('checkbox')).toHaveCount(2);
  await openMenu('Layout');
  const flat = popup.getByRole('menuitemcheckbox', { name: 'All entries', exact: true });
  await expect(flat).not.toBeChecked();
  await flat.click();
  await expect(button('Folder view')).toBeEnabled();
  await expect(page.getByRole('checkbox')).toHaveCount(5);
  await openMenu('Layout');
  await expect(flat).toBeChecked();
  await flat.click();
  await expect(page.getByRole('checkbox')).toHaveCount(2);
  await openMenu('Layout');
  await expect(popup.getByRole('menuitemcheckbox', { name: 'Sort by name', exact: true })).toBeChecked();
  await popup.getByRole('menuitemcheckbox', { name: 'Sort by size', exact: true }).click();
  await openMenu('Layout');
  await expect(popup.getByRole('menuitemcheckbox', { name: 'Sort by size', exact: true })).toBeChecked();
  await expect(popup.getByRole('menuitemcheckbox', { name: 'Sort by name', exact: true })).not.toBeChecked();
  await page.keyboard.press('Escape');
  await expect(popup).toHaveCount(0);
  await button('Open folder docs/').click();
  await openMenu('Edit');
  const apple = await page.evaluate(() => /mac|iphone|ipad/i.test(navigator.userAgentData?.platform ?? navigator.platform));
  await expect(popup.getByText(`${apple ? 'Cmd' : 'Ctrl'}+A`, { exact: true })).toBeVisible();
  await expect(popup.getByText(/\bmod\+/)).toHaveCount(0);
  await popup.getByRole('menuitem', { name: 'Select all', exact: true }).click();
  await expect(popup).toHaveCount(0);
  await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
  for (const name of ['docs/empty/', 'docs/nested/', 'docs/你好.txt']) await expect(page.getByRole('checkbox', { name, exact: true })).toBeChecked();
  const path = page.getByRole('textbox', { name: 'Archive path', exact: true });
  await path.fill('docs/nested/');
  await path.press('ControlOrMeta+a');
  assert.deepEqual(await path.evaluate((input) => [input.selectionStart, input.selectionEnd]), [0, 'docs/nested/'.length]);
  await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
  await path.press('Alt+ArrowLeft');
  await expect(path).toHaveValue('docs/nested/');
  await expect(page.getByRole('checkbox')).toHaveCount(3);
  await expect(button('Forward')).toBeDisabled();
  await button('Name').focus();
  await expect(path).not.toBeFocused();
  await page.keyboard.press('Alt+ArrowLeft');
  await expect(path).toHaveValue('/');
  await expect(page.getByRole('checkbox')).toHaveCount(2);
  await expect(page.getByRole('checkbox', { name: 'other.txt', exact: true })).not.toBeChecked();
  await button('Browser').focus();
  await button('Browser').press('Enter');
  await expect(popup).toHaveCount(1);
  await page.keyboard.press('End');
  await expect(popup.getByRole('menuitem', { name: 'Go to path', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(popup).toHaveCount(0);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await expect(path).toBeFocused();
  await page.keyboard.type('docs/nested/');
  await expect(path).toHaveValue('docs/nested/');
  await path.press('Enter');
  await expect(path).toHaveValue('docs/nested/');
  await expect(path).not.toBeFocused();
  await menuAction('Edit', 'Clear selection');
  await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: 'docs/nested/data.bin', exact: true }).click();
  await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
  await openMenu('File');
  const pendingDownload = page.waitForEvent('download');
  await popup.getByRole('menuitem', { name: 'Download archive', exact: true }).click();
  assert.equal((await pendingDownload).suggestedFilename(), 'Archive.tar');
  await menuAction('File', 'Close archive');
  await expect(page.getByRole('checkbox')).toHaveCount(0);
  await expect(page.getByText('Open an archive to view its contents.', { exact: true })).toBeVisible();
  await expect(path).toHaveValue('/');
  await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
  for (const name of ['Back', 'Forward', 'Up', 'Download ZIP', 'Test archive', 'Extract…']) await expect(button(name)).toBeDisabled();
  await openMenu('File');
  for (const name of ['Download archive', 'Close archive']) await expect(popup.getByRole('menuitem', { name, exact: true })).toBeDisabled();
  assert.equal(downloads.length, 1);
});

test('Busy application menu items match toolbar eligibility and menu Cancel stops the real test worker', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, popup, openMenu, menuAction, tree, downloads } = await menuPage(t);
  await choose();
  const selected = page.getByRole('checkbox', { name: '你好.txt', exact: true });
  await selected.click();
  let release, intercepted = false;
  const gate = new Promise((resolve) => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async (route) => {
    intercepted = true;
    await gate;
    await route.continue().catch(() => {});
  });
  try {
    await menuAction('Tools', 'Test archive');
    await expect.poll(() => intercepted).toBe(true);
    await expect(page.getByText('Testing archive…', { exact: true })).toBeVisible();
    await openMenu('File');
    for (const [item, action] of [['Open archive…', 'Open archive…'], ['Create archive…', 'Create archive…'], ['Download archive', 'Download ZIP']]) {
      await expect(popup.getByRole('menuitem', { name: item, exact: true })).toBeDisabled();
      await expect(button(action)).toBeDisabled();
    }
    await expect(popup.getByRole('menuitem', { name: 'Close archive', exact: true })).toBeDisabled();
    await button('Tools').hover();
    await expect(popup).toHaveCount(1);
    for (const name of ['Test archive', 'Extract…']) {
      await expect(popup.getByRole('menuitem', { name, exact: true })).toBeDisabled();
      await expect(button(name)).toBeDisabled();
    }
    await expect(popup.getByRole('menuitem', { name: 'Cancel', exact: true })).toBeEnabled();
    await expect(button('Cancel')).toBeEnabled();
    await popup.getByRole('menuitem', { name: 'Cancel', exact: true }).click();
    await expect(page.getByText('Archive test cancelled.', { exact: true })).toBeVisible();
    await expect(button('Test archive')).toBeEnabled();
    await expect(selected).toBeChecked();
    await expect.poll(() => page.evaluate(() => window.testWorkers)).toEqual([{ terminated: true, codes: [] }]);
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
  }
  await menuAction('Tools', 'Test archive');
  await expect(page.getByText('Tested 3 entries (13 bytes). Content checksums: 2/2 files.', { exact: true })).toBeVisible();
  await expect(selected).toBeChecked();
  assert.deepEqual(await tree(), sentinelTree);
  assert.equal(downloads.length, 0);
});

test('The Help menu opens About and restores focus after dismissal', { timeout: 120_000 }, async (t) => {
  const { page, button, menuAction } = await menuPage(t);
  await menuAction('Help', 'About PeaZip');
  const dialog = page.getByRole('alertdialog', { name: 'About PeaZip', exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/Read: ZIP, TAR, TAR.GZ, 7z/)).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Close', exact: true })).toBeFocused();
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(button('Help')).toBeFocused();
  await button('Help').press('Enter');
  await page.getByRole('menuitem', { name: 'About PeaZip', exact: true }).press('Enter');
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Close', exact: true }).press('Enter');
  await expect(dialog).toHaveCount(0);
  await expect(button('Help')).toBeFocused();
});

for (const touch of [false, true]) {
  for (const colorScheme of ['light', 'dark']) {
    const viewport = touch ? { width: 360, height: 800 } : { width: 1440, height: 900 };
    test(`Application menu popup uses distinct item roles in ${touch ? 'touch' : 'desktop'} ${colorScheme} mode`, { timeout: 120_000 }, async (t) => {
      const { page, button, choose, popup, openMenu, menuAction } = await menuPage(t, { viewport, colorScheme, hasTouch: touch, isMobile: touch });
      await choose();
      await openMenu('File');
      await expect(popup.getByRole('menuitem', { name: 'Open archive…', exact: true })).toBeVisible();
      await expect(button('Open archive…')).toBeVisible();
      await expect(popup.getByRole('button', { name: 'Open archive…', exact: true })).toHaveCount(0);
      await expect(popup.getByRole('menuitem', { name: 'Create folder archive…', exact: true })).toHaveCount(0);
      const bounds = await popup.boundingBox();
      assert.ok(bounds.x >= 0 && bounds.x + bounds.width <= viewport.width + 1);
      if (touch) {
        assert.ok(Math.abs(bounds.x) <= 1 && Math.abs(bounds.width - viewport.width) <= 1, 'touch menus must use the built-in full-width sheet');
        await expect.poll(async () => {
          const sheet = await popup.boundingBox();
          return Math.abs(sheet.y + sheet.height - viewport.height);
        }).toBeLessThanOrEqual(1);
      }
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      const screenshot = resolve(root, '..', `web-menu-${touch ? 'touch' : 'desktop'}-${colorScheme}.png`);
      await page.screenshot({ path: screenshot, fullPage: true, animations: 'disabled' });
      t.diagnostic(`Screenshot: ${screenshot}`);
      await page.keyboard.press('Escape');
      await expect(popup).toHaveCount(0);
      if (touch) {
        await menuAction('Browser', 'Go to path');
        await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
        const path = page.getByRole('textbox', { name: 'Archive path', exact: true });
        await expect(path).toBeFocused();
        await page.keyboard.type('空目录/');
        await expect(path).toHaveValue('空目录/');
        await page.keyboard.press('Enter');
        await expect(path).toHaveValue('空目录/');
        await expect(path).not.toBeFocused();
        await expect(page.getByRole('checkbox')).toHaveCount(3);
        await button('Folder view').click();
        await expect(page.getByRole('checkbox')).toHaveCount(0);
        await button('All entries').click();
        await expect(page.getByRole('checkbox')).toHaveCount(3);
      }
      await openMenu('Layout');
      await expect(popup.getByRole('menuitemcheckbox', { name: 'All entries', exact: true })).toBeChecked();
      await popup.getByRole('menuitemcheckbox', { name: 'All entries', exact: true }).click();
      await expect(button('All entries')).toBeEnabled();
    });
  }
}

const sentinelTree = { 'sentinel.txt': Array.from(Buffer.from('leave me unchanged')) };

async function downloadArchive(page, button) {
  const pending = page.waitForEvent('download');
  await button.click();
  const artifact = await pending;
  const chunks = [];
  for await (const chunk of await artifact.createReadStream()) chunks.push(chunk);
  return { name: artifact.suggestedFilename(), bytes: Buffer.concat(chunks) };
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 360, height: 800 }]) {
  test(`Deselecting a child of a selected parent excludes it from real extraction at ${viewport.width}px`, { timeout: 120_000 }, async (t) => {
    const { page, button, tree, downloads } = await extractionPage(t);
    await page.setViewportSize(viewport);
    await button('Folder view').click();
    const pending = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await pending).setFiles(fileURLToPath(new URL('../../tests/fixtures/sample.tar', import.meta.url)));
    await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox')).toHaveCount(2);
    await page.getByRole('checkbox', { name: 'docs/', exact: true }).click();
    await button('Open folder docs/').click();
    for (const name of ['docs/empty/', 'docs/nested/', 'docs/你好.txt']) await expect(page.getByRole('checkbox', { name, exact: true })).toBeChecked();
    const chinese = page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true });
    await chinese.click();
    await expect(chinese).not.toBeChecked();
    await expect(page.getByText('2 selected', { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox', { name: 'docs/empty/', exact: true })).toBeChecked();
    await expect(page.getByRole('checkbox', { name: 'docs/nested/', exact: true })).toBeChecked();
    await button('Archive root').click();
    await expect(page.getByRole('checkbox', { name: 'docs/', exact: true })).toHaveAttribute('aria-checked', 'mixed');
    await button('Open folder docs/').click();
    await expect(chinese).not.toBeChecked();
    await button('Extract…').click();
    await button('Continue').click();
    const report = page.getByText(/^Extracted 2 entries to PeaZip-/);
    await expect(report).toBeVisible();
    const output = await tree();
    const folder = Object.keys(output).find((name) => name.startsWith('PeaZip-'));
    assert.match(folder, /^PeaZip-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
    assert.deepEqual(output, { ...sentinelTree, [folder]: {
      docs: { empty: {}, nested: { 'data.bin': [0, 127, 255] } },
    } }, 'parent selection must not reinclude the unchecked child during extraction');
    await expect(chinese).not.toBeChecked();
    await expect.poll(() => page.evaluate(() => window.extractionWorkers)).toEqual([{ terminated: true, codes: [1] }]);
    assert.equal(downloads.length, 0);
  });
}

test('Test archive validates ZIP, TAR, TGZ and 7Z in real workers without exports or downloads', { timeout: 120_000 }, async (t) => {
  const { page, button, tree, downloads } = await extractionPage(t);
  await expect(button('Test archive')).toBeDisabled();
  const reports = [];
  for (const [name, path, entries, files, total, checksummed, selectedName] of [
    ['zip.bin', fixture('sample.zip'), 3, 2, 13, 2, '你好.txt'],
    ['tar.bin', fileURLToPath(new URL('../../tests/fixtures/sample.tar', import.meta.url)), 5, 3, 27, 0, 'docs/你好.txt'],
    ['tgz.bin', fileURLToPath(new URL('../../tests/fixtures/sample.tar.gz', import.meta.url)), 5, 3, 27, 3, 'docs/你好.txt'],
    ['7z.bin', fileURLToPath(new URL('../../tests/fixtures/sample.7z', import.meta.url)), 6, 3, 26, 3, 'docs/你好.txt'],
  ]) {
    const pending = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await pending).setFiles({ name, mimeType: 'application/octet-stream', buffer: await readFile(path) });
    await expect(page.getByText(`${entries} entries`, { exact: true })).toBeVisible();
    await expect(button('Test archive')).toBeEnabled();
    const selected = page.getByRole('checkbox', { name: selectedName, exact: true });
    await selected.click();
    await button('Test archive').click();
    await expect(page.getByText(`Tested ${entries} entries (${total} bytes). Content checksums: ${checksummed}/${files} files.`, { exact: true })).toBeVisible();
    await expect(selected).toBeChecked();
    await expect(page.getByText(name, { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox')).toHaveCount(entries);
    await expect(button('Test archive')).toBeEnabled();
    reports.push({ terminated: true, codes: [1] });
    await expect.poll(() => page.evaluate(() => window.testWorkers)).toEqual(reports);
    await expect.poll(() => page.workers().length).toBe(0);
    assert.deepEqual(await tree(), sentinelTree);
    assert.equal(downloads.length, 0);
  }
  assert.deepEqual(await page.evaluate(() => window.pickerCalls), []);
  assert.deepEqual(await page.evaluate(() => window.extractionWorkers), []);
});

test('Test archive catches an unselected ZIP file with bad CRC and preserves bytes and selection', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree, downloads } = await extractionPage(t);
  const corrupt = Buffer.from(await readFile(fixture('sample.zip')));
  const central = corrupt.indexOf(Buffer.from('PK\x01\x02'), corrupt.indexOf(Buffer.from('PK\x01\x02')) + 4);
  assert.ok(central > 0);
  const local = corrupt.readUInt32LE(central + 42);
  assert.equal(corrupt.toString('utf8', central + 46, central + 46 + corrupt.readUInt16LE(central + 28)), 'notes.txt');
  corrupt.writeUInt32LE(0, local + 14);
  corrupt.writeUInt32LE(0, central + 16);
  await choose({ name: 'hidden-bad-crc.zip', mimeType: 'application/zip', buffer: corrupt });
  const selected = page.getByRole('checkbox', { name: '你好.txt', exact: true });
  await selected.click();
  await expect(page.getByRole('checkbox', { name: 'notes.txt', exact: true })).not.toBeChecked();
  const snapshot = await downloadArchive(page, button('Download ZIP'));
  const downloadsBefore = downloads.length;
  await button('Test archive').click();
  await expect(page.getByText(/^Could not test archive:/)).toBeVisible();
  await expect(button('Test archive')).toBeEnabled();
  await expect(selected).toBeChecked();
  await expect(page.getByText('hidden-bad-crc.zip', { exact: true })).toBeVisible();
  await expect(page.getByRole('checkbox')).toHaveCount(3);
  await expect.poll(() => page.evaluate(() => window.testWorkers)).toEqual([{ terminated: true, codes: [0] }]);
  assert.equal(downloads.length, downloadsBefore);
  assert.deepEqual(await tree(), sentinelTree);
  assert.deepEqual(await downloadArchive(page, button('Download ZIP')), snapshot);
  assert.deepEqual(snapshot.bytes, corrupt);
  await choose();
  await button('Test archive').click();
  await expect(page.getByText('Tested 3 entries (13 bytes). Content checksums: 2/2 files.', { exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.testWorkers)).toEqual([
    { terminated: true, codes: [0] }, { terminated: true, codes: [1] },
  ]);
  assert.deepEqual(await page.evaluate(() => window.pickerCalls), []);
  assert.deepEqual(await page.evaluate(() => window.extractionWorkers), []);
});

for (const cancellation of ['Cancel', 'pagehide']) {
  test(`${cancellation} stops a real test worker; retry ignores stale reports and preserves the archive`, { timeout: 120_000 }, async (t) => {
    const { page, button, choose, tree, downloads } = await extractionPage(t);
    await choose();
    const selected = page.getByRole('checkbox', { name: '你好.txt', exact: true });
    await selected.click();
    const snapshot = await downloadArchive(page, button('Download ZIP'));
    const downloadsBefore = downloads.length;
    let release, intercepted = false;
    const gate = new Promise((resolve) => { release = resolve; });
    await page.route('**/worker-core_bg.wasm', async (route) => {
      intercepted = true;
      await gate;
      await route.continue().catch(() => {});
    });
    try {
      await button('Test archive').click();
      await expect.poll(() => intercepted).toBe(true);
      await expect(page.getByText('Testing archive…', { exact: true })).toBeVisible();
      await expect(button('Cancel')).toBeEnabled();
      for (const name of ['Test archive', 'Open archive…', 'Create archive…', 'Download ZIP', 'Extract…', 'Select all', 'Invert selection', 'Clear selection']) await expect(button(name)).toBeDisabled();
      if (cancellation === 'Cancel') await button('Cancel').click();
      else await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })));
      await expect(page.getByText('Archive test cancelled.', { exact: true })).toBeVisible();
      await expect(button('Test archive')).toBeEnabled();
      await expect(button('Cancel')).toBeDisabled();
      await expect(selected).toBeChecked();
      await expect.poll(() => page.evaluate(() => window.testWorkers)).toEqual([{ terminated: true, codes: [] }]);
      await expect.poll(() => page.workers().length).toBe(0);
    } finally {
      release();
      await page.unrouteAll({ behavior: 'wait' });
    }
    if (cancellation === 'pagehide') await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true })));
    let releaseRetry, retryIntercepted = false;
    const retryGate = new Promise((resolve) => { releaseRetry = resolve; });
    await page.route('**/worker-core_bg.wasm', async (route) => {
      retryIntercepted = true;
      await retryGate;
      await route.continue().catch(() => {});
    });
    try {
      await button('Test archive').click();
      await expect.poll(() => retryIntercepted).toBe(true);
      await page.evaluate(() => {
        const old = window.testWorkerHandlers[0];
        old({ data: { code: 1, bytes: new Uint8Array(20) } });
        old({ data: { code: 0, bytes: new TextEncoder().encode('stale error') } });
      });
      await expect(page.getByText('Testing archive…', { exact: true })).toBeVisible();
      await expect(button('Test archive')).toBeDisabled();
    } finally {
      releaseRetry();
      await page.unrouteAll({ behavior: 'wait' });
    }
    await expect(page.getByText('Tested 3 entries (13 bytes). Content checksums: 2/2 files.', { exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => window.testWorkers)).toEqual([
      { terminated: true, codes: [] }, { terminated: true, codes: [1] },
    ]);
    await expect(selected).toBeChecked();
    assert.equal(downloads.length, downloadsBefore);
    assert.deepEqual(await downloadArchive(page, button('Download ZIP')), snapshot);
    assert.deepEqual(await tree(), sentinelTree);
    assert.deepEqual(await page.evaluate(() => window.pickerCalls), []);
    assert.deepEqual(await page.evaluate(() => window.extractionWorkers), []);
  });
}

test('7Z uses real list/extract workers and preserves its snapshot after header CRC and next-header failures', { timeout: 120_000 }, async (t) => {
  const { page, button, tree, downloads } = await extractionPage(t);
  const bytes = await readFile(new URL('../../tests/fixtures/sample.7z', import.meta.url));
  async function pick(buffer, name) {
    const pending = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await pending).setFiles({ name, mimeType: 'application/zip', buffer });
  }
  const wasm = page.waitForResponse((response) => new URL(response.url()).pathname.endsWith('/worker-core_bg.wasm'));
  await pick(bytes, 'misnamed.zip');
  assert.equal((await wasm).ok(), true);
  await expect(page.getByText('6 entries', { exact: true })).toBeVisible();
  await expect(page.getByRole('checkbox')).toHaveCount(6);
  const chinese = page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true });
  await expect(chinese.getByLabel('Size in bytes: 10', { exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.listWorkers)).toEqual([{ terminated: true, codes: [1] }]);
  assert.deepEqual(await page.evaluate(() => window.extractionWorkers), []);
  await expect(button('Download 7Z')).toBeEnabled();
  for (const name of ['Download ZIP', 'Download TAR', 'Download TAR.GZ']) await expect(button(name)).toHaveCount(0);
  const selected = page.getByRole('checkbox', { name: 'docs/', exact: true });
  await selected.click();
  await page.evaluate(() => {
    const create = URL.createObjectURL;
    window.downloadTypes = [];
    URL.createObjectURL = (blob) => { window.downloadTypes.push(blob.type); return create(blob); };
  });
  const snapshot = await downloadArchive(page, button('Download 7Z'));
  assert.equal(snapshot.name, 'Archive.7z');
  assert.deepEqual(snapshot.bytes, bytes);
  assert.deepEqual(await page.evaluate(() => window.downloadTypes), ['application/x-7z-compressed']);

  function crc32(bytes) {
    let crc = 0xffffffff;
    for (const byte of bytes) {
      crc ^= byte;
      for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
    return (~crc) >>> 0;
  }
  const next = 32 + Number(bytes.readBigUInt64LE(12));
  assert.equal(bytes[next], 0x17, 'fixture must exercise an encoded header');
  assert.equal(crc32(bytes.subarray(12, 32)), bytes.readUInt32LE(8));
  assert.equal(crc32(bytes.subarray(next)), bytes.readUInt32LE(28));
  const badStartCRC = Buffer.from(bytes);
  badStartCRC[8] ^= 1;
  const badNextCRC = Buffer.from(bytes);
  badNextCRC[next] ^= 1;
  const malformed = Buffer.from(bytes);
  malformed[next] = 0xff;
  malformed.writeUInt32LE(crc32(malformed.subarray(next)), 28);
  malformed.writeUInt32LE(crc32(malformed.subarray(12, 32)), 8);
  const badOffset = Buffer.from(bytes);
  badOffset.writeBigUInt64LE(0xffffffffffffffffn, 12);
  badOffset.writeUInt32LE(crc32(badOffset.subarray(12, 32)), 8);
  const codes = [1];
  for (const [name, corrupt] of [
    ['bad-start-crc.7z', badStartCRC], ['bad-next-crc.7z', badNextCRC],
    ['malformed-next-header.7z', malformed], ['bad-next-offset.7z', badOffset],
  ]) {
    await pick(corrupt, name);
    await expect(page.getByText(/^Could not open archive:/)).toBeVisible();
    await expect(button('Open archive…')).toBeEnabled();
    await expect(selected).toBeChecked();
    await expect(page.getByText('misnamed.zip', { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox')).toHaveCount(6);
    codes.push(0);
    await expect.poll(() => page.evaluate(() => window.listWorkers)).toEqual(codes.map((code) => ({ terminated: true, codes: [code] })));
    await expect.poll(() => page.workers().length).toBe(0);
    assert.deepEqual(await tree(), sentinelTree);
    assert.deepEqual(await downloadArchive(page, button('Download 7Z')), snapshot, `${name} must preserve archive bytes, format and selection`);
  }
  await pick(bytes, 'retry.7z');
  await expect(page.getByText('6 entries', { exact: true })).toBeVisible();
  await expect(page.getByText('retry.7z', { exact: true })).toBeVisible();
  await expect(selected).not.toBeChecked();
  await selected.click();
  await button('Extract…').click();
  await button('Continue').click();
  const report = page.getByText(/^Extracted 5 entries to PeaZip-/);
  await expect(report).toBeVisible();
  const folder = (await report.innerText()).split(' to ')[1];
  assert.deepEqual(await tree(), { ...sentinelTree, [folder]: {
    docs: { '你好.txt': Array.from(Buffer.from('7z 你好\n')), empty: {}, nested: { 'data.bin': [0, 127, 255] } },
  } });
  await expect.poll(() => page.evaluate(() => window.extractionWorkers)).toEqual([{ terminated: true, codes: [1] }]);
  codes.push(1);
  await expect.poll(() => page.evaluate(() => window.listWorkers)).toEqual(codes.map((code) => ({ terminated: true, codes: [code] })));
  await expect.poll(() => page.workers().length).toBe(0);
  assert.equal(downloads.length, 5, 'opening and extraction must not download automatically');
});

test('TAR gzip uses a real listing worker, preserves snapshots on bad CRC, downloads unchanged and extracts', { timeout: 120_000 }, async (t) => {
  const { page, button, tree, downloads } = await extractionPage(t);
  const bytes = await readFile(new URL('../../tests/fixtures/sample.tar.gz', import.meta.url));
  async function pick(buffer, name) {
    const pending = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await pending).setFiles({ name, mimeType: 'application/zip', buffer });
  }
  let release, intercepted = false;
  const gate = new Promise((resolve) => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async (route) => {
    intercepted = true;
    await gate;
    await route.continue().catch(() => {});
  });
  const wasm = page.waitForResponse((response) => new URL(response.url()).pathname.endsWith('/worker-core_bg.wasm'));
  try {
    await pick(bytes, 'misnamed.zip');
    await expect.poll(() => intercepted).toBe(true);
    await expect(button('Open archive…')).toBeDisabled();
    await expect(page.getByRole('checkbox')).toHaveCount(0);
    assert.equal(await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => resolve(true)))), true);
    await expect.poll(() => page.evaluate(() => window.listWorkers)).toEqual([{ terminated: false, codes: [] }]);
    assert.deepEqual(await page.evaluate(() => window.extractionWorkers), []);
  } finally { release(); }
  assert.equal((await wasm).ok(), true, 'listing must initialize the real Rust WASM in a Worker');
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await page.unrouteAll({ behavior: 'wait' });
  await expect.poll(() => page.evaluate(() => window.listWorkers)).toEqual([{ terminated: true, codes: [1] }]);
  await expect.poll(() => page.workers().length).toBe(0);
  await expect(button('Download TAR.GZ')).toBeEnabled();
  await expect(button('Download ZIP')).toHaveCount(0);
  await expect(button('Download TAR')).toHaveCount(0);
  const selected = page.getByRole('checkbox', { name: 'docs/', exact: true });
  await selected.click();
  await page.evaluate(() => {
    const create = URL.createObjectURL;
    window.downloadTypes = [];
    URL.createObjectURL = (blob) => { window.downloadTypes.push(blob.type); return create(blob); };
  });
  const snapshot = await downloadArchive(page, button('Download TAR.GZ'));
  assert.equal(snapshot.name, 'Archive.tar.gz');
  assert.deepEqual(snapshot.bytes, bytes);
  assert.deepEqual(await page.evaluate(() => window.downloadTypes), ['application/gzip']);

  const corrupt = Buffer.from(bytes);
  corrupt[corrupt.length - 8] ^= 1;
  await pick(corrupt, 'bad-crc.tar.gz');
  await expect(page.getByText(/^Could not open archive:.*gzip.*checksum/)).toBeVisible();
  await expect(button('Open archive…')).toBeEnabled();
  await expect(selected).toBeChecked();
  await expect(page.getByText('misnamed.zip', { exact: true })).toBeVisible();
  await expect(page.getByRole('checkbox')).toHaveCount(5);
  await expect.poll(() => page.evaluate(() => window.listWorkers)).toEqual([
    { terminated: true, codes: [1] }, { terminated: true, codes: [0] },
  ]);
  assert.deepEqual(await tree(), sentinelTree, 'bad gzip CRC must fail before any output is created');
  assert.deepEqual(await downloadArchive(page, button('Download TAR.GZ')), snapshot, 'bad CRC must preserve exact prior archive bytes and format');

  await pick(bytes, 'retry.tar.gz');
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await expect(page.getByText('retry.tar.gz', { exact: true })).toBeVisible();
  await expect(selected).not.toBeChecked();
  await selected.click();
  await button('Extract…').click();
  await button('Continue').click();
  const report = page.getByText(/^Extracted 4 entries to PeaZip-/);
  await expect(report).toBeVisible();
  const folder = (await report.innerText()).split(' to ')[1];
  assert.deepEqual(await tree(), { ...sentinelTree, [folder]: {
    docs: { '你好.txt': Array.from(Buffer.from('tar 你好\n')), empty: {}, nested: { 'data.bin': [0, 127, 255] } },
  } });
  await expect.poll(() => page.evaluate(() => window.extractionWorkers)).toEqual([{ terminated: true, codes: [1] }]);
  await expect.poll(() => page.evaluate(() => window.listWorkers)).toEqual([
    { terminated: true, codes: [1] }, { terminated: true, codes: [0] }, { terminated: true, codes: [1] },
  ]);
  await expect.poll(() => page.workers().length).toBe(0);
  assert.equal(downloads.length, 2, 'opening and extraction must not download automatically');
});

for (const cancellation of ['Cancel', 'pagehide']) {
  test(`${cancellation} stops a real listing worker and stale callbacks cannot replace a later gzip snapshot`, { timeout: 120_000 }, async (t) => {
    const { page, button, choose, tree } = await extractionPage(t);
    await choose();
    const selected = page.getByRole('checkbox', { name: '你好.txt', exact: true });
    await selected.click();
    const original = await downloadArchive(page, button('Download ZIP'));
    const bytes = await readFile(new URL('../../tests/fixtures/sample.tar.gz', import.meta.url));
    let release, intercepted = false;
    const gate = new Promise((resolve) => { release = resolve; });
    await page.route('**/worker-core_bg.wasm', async (route) => {
      intercepted = true;
      await gate;
      await route.continue().catch(() => {});
    });
    async function pick() {
      const pending = page.waitForEvent('filechooser');
      await button('Open archive…').click();
      await (await pending).setFiles({ name: 'retry.tar.gz', mimeType: 'application/gzip', buffer: bytes });
    }
    try {
      await pick();
      await expect.poll(() => intercepted).toBe(true);
      await expect(button('Open archive…')).toBeDisabled();
      await expect(button('Cancel')).toBeEnabled();
      if (cancellation === 'Cancel') {
        await button('Cancel').click();
        await expect(page.getByText('Opening cancelled.', { exact: true })).toBeVisible();
      } else {
        await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })));
      }
      await expect(button('Open archive…')).toBeEnabled();
      await expect.poll(() => page.evaluate(() => window.listWorkers[1])).toEqual({ terminated: true, codes: [] });
      await expect.poll(() => page.workers().length).toBe(0);
      await expect(selected).toBeChecked();
      if (cancellation === 'pagehide') await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true })));
      assert.deepEqual(await downloadArchive(page, button('Download ZIP')), original);
      assert.deepEqual(await tree(), sentinelTree);
    } finally {
      release();
      await page.unrouteAll({ behavior: 'wait' });
    }
    await pick();
    await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
    const retry = await downloadArchive(page, button('Download TAR.GZ'));
    await page.evaluate(() => {
      const old = window.listWorkerHandlers[1];
      old({ data: { progress: { phase: 'extract', done: 0, total: 1 } } });
      old({ data: { code: 1, bytes: new Uint8Array([0, 1, 2]) } });
      old({ data: { code: 0, bytes: new TextEncoder().encode('stale error') } });
    });
    await expect(page.getByText('retry.tar.gz', { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox')).toHaveCount(5);
    assert.deepEqual(await downloadArchive(page, button('Download TAR.GZ')), retry);
    await expect.poll(() => page.evaluate(() => window.listWorkers[2])).toEqual({ terminated: true, codes: [1] });
  });
}

test('TAR is detected by content, downloaded unchanged and extracted by the Rust worker', { timeout: 120_000 }, async (t) => {
  const { page, button, tree, downloads } = await extractionPage(t);
  const bytes = await readFile(new URL('../../tests/fixtures/sample.tar', import.meta.url));
  const picked = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await picked).setFiles({ name: 'misnamed.zip', mimeType: 'application/zip', buffer: bytes });
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await expect(page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true })).toBeVisible();
  await expect(button('Download TAR')).toBeEnabled();
  await expect(button('Download ZIP')).toHaveCount(0);
  const pending = page.waitForEvent('download');
  await button('Download TAR').click();
  const download = await pending;
  assert.equal(download.suggestedFilename(), 'Archive.tar');
  const chunks = [];
  for await (const chunk of await download.createReadStream()) chunks.push(chunk);
  assert.deepEqual(Buffer.concat(chunks), bytes, 'TAR bytes must not be mislabeled or recompressed');
  await page.getByRole('checkbox', { name: 'docs/', exact: true }).click();
  await button('Extract…').click();
  await button('Continue').click();
  const report = page.getByText(/^Extracted 4 entries to PeaZip-/);
  await expect(report).toBeVisible();
  const folder = (await report.innerText()).split(' to ')[1];
  assert.deepEqual(await tree(), { ...sentinelTree, [folder]: {
    docs: { '你好.txt': Array.from(Buffer.from('tar 你好\n')), empty: {}, nested: { 'data.bin': [0, 127, 255] } },
  } });
  assert.equal(downloads.length, 1);
});

for (const [width, height, colorScheme] of [[1024, 600, 'light'], [1440, 900, 'dark'], [360, 800, 'light'], [430, 800, 'dark']]) {
  test(`Extraction dialog is usable at ${width}x${height} ${colorScheme}`, { timeout: 120_000 }, async (t) => {
    const { page, button, choose, tree } = await extractionPage(t, true, { viewport: { width, height }, colorScheme, isMobile: width < 600, hasTouch: width < 600 });
    await choose();
    if (width >= 1024) await page.getByRole('checkbox', { name: '你好.txt', exact: true }).click();
    await button('Extract…').click();
    const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
    await modal.getByRole('radio', { name: 'Files only (no folders)', exact: true }).click();
    await expect(modal.getByRole('button', { name: 'Continue', exact: true })).toBeInViewport();
    await expect(modal.getByRole('button', { name: 'Cancel', exact: true })).toBeInViewport();
    const box = await modal.boundingBox();
    assert.ok(box.x >= 0 && box.x + box.width <= width + 1);
    const screenshot = fileURLToPath(new URL(`../../target/extract-dialog-${width}-${colorScheme}.png`, import.meta.url));
    await page.screenshot({ path: screenshot });
    t.diagnostic(`Screenshot: ${screenshot}`);
    await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(button('Extract…')).toBeEnabled();
    assert.deepEqual(await tree(), sentinelTree);
    assert.deepEqual(await page.evaluate(() => window.pickerCalls), []);
  });
}

for (const width of [1024, 360]) {
  test(`Type sorting uses the displayed type, retains selection and exports the same bytes at ${width}px`, { timeout: 120_000 }, async (t) => {
    const { page, button, tree } = await extractionPage(t, true, { viewport: { width, height: 800 }, isMobile: width < 600, hasTouch: width < 600 });
    const picker = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await picker).setFiles(await extractionFixture([['folder.bin/', ''], ['a.txt', 'first'], ['z.TXT', 'second'], ['dir.ext/plain', 'plain'], ['data.bin', 'binary']]));
    await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
    await page.getByRole('checkbox', { name: 'a.txt', exact: true }).click();
    await button('Layout').click();
    await page.getByRole('menuitemcheckbox', { name: 'Sort by type', exact: true }).click();
    const order = () => page.getByRole('checkbox').evaluateAll(nodes => nodes.map(node => node.getAttribute('aria-label')));
    await expect.poll(order).toEqual(['folder.bin/', 'data.bin', 'dir.ext/plain', 'a.txt', 'z.TXT']);
    if (width >= 760) await button('Type').click();
    else {
      await button('Layout').click();
      await page.getByRole('menuitemcheckbox', { name: 'Sort by type', exact: true }).click();
    }
    await expect.poll(order).toEqual(['folder.bin/', 'a.txt', 'z.TXT', 'dir.ext/plain', 'data.bin']);
    await expect(page.getByRole('menu')).toHaveCount(0);
    await expect(page.getByRole('checkbox', { name: 'a.txt', exact: true })).toBeChecked();
    const screenshot = fileURLToPath(new URL(`../../target/type-sort-${width}.png`, import.meta.url));
    await page.screenshot({ path: screenshot });
    await button('Extract…').click();
    const output = await finishExtraction(page, button);
    assert.deepEqual(await tree(), { ...sentinelTree, [output]: { 'a.txt': Array.from(Buffer.from('first')) } });
    await expect(page.getByRole('checkbox', { name: 'a.txt', exact: true })).toBeChecked();
  });
}

for (const width of [1024, 360]) {
  test(`Name filter preserves hidden selections and bounds displayed extraction at ${width}px`, { timeout: 120_000 }, async (t) => {
    const { page, button, tree } = await extractionPage(t, true, { viewport: { width, height: 800 }, isMobile: width < 600, hasTouch: width < 600 });
    const picker = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await picker).setFiles(await extractionFixture([['docs/你好.txt', '中文'], ['docs/plain.txt', 'plain'], ['outside.txt', 'outside'], ['literal[1].txt', 'literal']]));
    await expect(page.getByText('4 entries', { exact: true })).toBeVisible();
    const filter = page.getByRole('textbox', { name: 'Filter names', exact: true });
    await page.getByRole('checkbox', { name: 'outside.txt', exact: true }).click();
    await filter.fill('你好');
    await expect(page.getByRole('checkbox')).toHaveCount(1);
    await expect(page.getByText('1 of 4 rows shown', { exact: true })).toBeVisible();
    await filter.press('Enter');
    await button('Select all').click();
    await button('Extract…').click();
    const selected = await finishExtraction(page, button);
    let expected = { ...sentinelTree, [selected]: { docs: { '你好.txt': Array.from(Buffer.from('中文')) }, 'outside.txt': Array.from(Buffer.from('outside')) } };
    assert.deepEqual(await tree(), expected);
    await button('Clear selection').click();
    await filter.fill('PLAIN.TXT');
    await filter.press('Enter');
    await button('Extract…').click();
    const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
    await expect(modal.getByRole('radio', { name: 'Displayed entries', exact: true })).toBeChecked();
    await expect(modal.getByText(/Names contain: PLAIN.TXT/)).toBeVisible();
    const displayed = await finishExtraction(page, button);
    expected = { ...expected, [displayed]: { docs: { 'plain.txt': Array.from(Buffer.from('plain')) } } };
    assert.deepEqual(await tree(), expected);
    await filter.fill('[1]');
    await expect(page.getByRole('checkbox')).toHaveCount(1);
    await expect(page.getByRole('checkbox', { name: 'literal[1].txt', exact: true })).toBeVisible();
    await filter.fill('no matches');
    await expect(page.getByRole('checkbox')).toHaveCount(0);
    await expect(button('Select all')).toBeDisabled();
    await expect(button('Invert selection')).toBeDisabled();
    await filter.press('Enter');
    await button('Extract…').click();
    await expect(modal.getByRole('button', { name: 'Continue', exact: true })).toBeDisabled();
    await expect(modal.getByRole('radio', { name: 'All entries', exact: true })).toBeEnabled();
    await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
    await button('Clear filter').click();
    await expect(page.getByRole('checkbox')).toHaveCount(4);
    const screenshot = fileURLToPath(new URL(`../../target/name-filter-${width}.png`, import.meta.url));
    await filter.fill('docs');
    await page.screenshot({ path: screenshot });
    await button('Folder view').click();
    await expect(filter).toHaveValue('docs');
    await button('Open folder docs/').click();
    await expect(filter).toHaveValue('');
    await expect(page.getByRole('checkbox')).toHaveCount(2);
    assert.deepEqual(await tree(), expected);
  });
}

for (const width of [1024, 360]) {
  test(`Wildcard name mode handles sets, errors and frozen extraction at ${width}px`, { timeout: 120_000 }, async (t) => {
    const { page, button, tree } = await extractionPage(t, true, { viewport: { width, height: 800 }, isMobile: width < 600, hasTouch: width < 600 });
    const picker = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await picker).setFiles(await extractionFixture([['docs/a1.txt', 'first'], ['docs/a2.TXT', 'second'], ['docs/a3.bin', 'third'], ['outside.txt', 'outside'], ['literal[1].txt', 'literal']]));
    await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
    const filter = page.getByRole('textbox', { name: 'Filter names', exact: true });
    const mode = page.getByRole('switch', { name: 'Use wildcards', exact: true });
    await expect(mode).not.toBeChecked();
    await page.getByRole('checkbox', { name: 'outside.txt', exact: true }).click();
    await filter.fill('a[1-2].?xt');
    await expect(page.getByRole('checkbox')).toHaveCount(0);
    await mode.click();
    await expect(mode).toBeChecked();
    await expect(page.getByRole('checkbox')).toHaveCount(2);
    await filter.press('Enter');
    await button('Select all').click();
    await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
    await button('Extract…').click();
    const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
    await expect(mode).toBeDisabled();
    await expect(modal.getByText(/Name mask: a\[1-2\]\.\?xt/)).toBeVisible();
    await modal.getByRole('radio', { name: 'Displayed entries', exact: true }).click();
    const output = await finishExtraction(page, button);
    const expected = { ...sentinelTree, [output]: { docs: { 'a1.txt': Array.from(Buffer.from('first')), 'a2.TXT': Array.from(Buffer.from('second')) } } };
    assert.deepEqual(await tree(), expected);
    await filter.fill('[a-]');
    await expect(page.getByText('Invalid name mask: unclosed character set.', { exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox')).toHaveCount(0);
    await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
    await button('Clear selection').click();
    await filter.press('Enter');
    await button('Extract…').click();
    await expect(modal.getByRole('button', { name: 'Continue', exact: true })).toBeDisabled();
    await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(modal).toHaveCount(0);
    await filter.fill('a[!1].*');
    await expect(page.getByRole('checkbox')).toHaveCount(2);
    const screenshot = fileURLToPath(new URL(`../../target/wildcard-${width}.png`, import.meta.url));
    await page.screenshot({ path: screenshot });
    await mode.click();
    await expect(page.getByRole('checkbox')).toHaveCount(0);
    await filter.fill('literal[1]');
    await expect(page.getByRole('checkbox', { name: 'literal[1].txt', exact: true })).toBeVisible();
    await button('Clear filter').click();
    await expect(page.getByRole('checkbox')).toHaveCount(5);
    assert.deepEqual(await tree(), expected);
  });
}

async function packedFixture() {
  const { stdout } = await promisify(execFile)('uv', ['run', 'python', '-c', `
import base64, io, json, zipfile
out = io.BytesIO()
with zipfile.ZipFile(out, 'w') as archive:
    for name, data, method in [('folder/', b'', 0), ('compressed.txt', b'A' * 2000, 8), ('stored.bin', bytes(range(128)), 0), ('empty.txt', b'', 0), ('dir.ext/plain', b'data', 0)]:
        archive.writestr(name, data, compress_type=method)
with zipfile.ZipFile(out) as archive:
    packed = {item.filename: None if item.is_dir() else item.compress_size for item in archive.infolist()}
print(json.dumps({'archive': base64.b64encode(out.getvalue()).decode(), 'packed': packed}))
`]);
  const value = JSON.parse(stdout);
  return { file: { name: 'packed.zip', mimeType: 'application/zip', buffer: Buffer.from(value.archive, 'base64') }, packed: value.packed };
}

for (const [width, colorScheme] of [[1024, 'light'], [1440, 'dark'], [360, 'light'], [430, 'dark']]) {
  test(`Packed size shows real ZIP values and sorts without changing selection at ${width}px ${colorScheme}`, { timeout: 120_000 }, async (t) => {
    const { page, button, tree } = await extractionPage(t, true, { viewport: { width, height: width < 600 ? 800 : 900 }, colorScheme, isMobile: width < 600, hasTouch: width < 600 });
    const { file, packed } = await packedFixture();
    const picker = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await picker).setFiles(file);
    await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
    await page.getByRole('checkbox', { name: 'stored.bin', exact: true }).click();
    await button('Layout').click();
    await page.getByRole('menuitemcheckbox', { name: 'Sort by packed size', exact: true }).click();
    await expect(button('Packed (bytes)')).toBeVisible();
    if (width < 600) await expect(button('Size (bytes)')).toHaveCount(0);
    const rows = page.getByRole('checkbox');
    const paths = () => rows.evaluateAll(nodes => nodes.map(node => node.getAttribute('aria-label')));
    const ordered = Object.keys(packed).filter(name => packed[name] !== null).sort((a, b) => packed[a] - packed[b] || a.localeCompare(b));
    await expect.poll(paths).toEqual(['folder/', ...ordered]);
    for (const [name, value] of Object.entries(packed)) {
      const row = page.getByRole('checkbox', { name, exact: true });
      await expect(row.getByLabel(value === null ? 'Packed bytes: not applicable' : `Packed bytes: ${value}`, { exact: true })).toHaveText(value === null ? '—' : String(value));
    }
    await expect(page.getByRole('checkbox', { name: 'dir.ext/plain', exact: true }).getByText('File', { exact: true })).toHaveCount(width < 760 ? 0 : 1);
    await button('Packed (bytes)').click();
    await expect.poll(paths).toEqual(['folder/', ...ordered.reverse()]);
    await expect(page.getByRole('checkbox', { name: 'stored.bin', exact: true })).toBeChecked();
    const screenshot = fileURLToPath(new URL(`../../target/packed-${width}-${colorScheme}.png`, import.meta.url));
    await page.screenshot({ path: screenshot });
    t.diagnostic(`Screenshot: ${screenshot}`);
    await button('Extract…').click();
    const output = await finishExtraction(page, button);
    assert.deepEqual(await tree(), { ...sentinelTree, [output]: { 'stored.bin': Array.from({ length: 128 }, (_, i) => i) } });
    await button('Layout').click();
    await page.getByRole('menuitemcheckbox', { name: 'Sort by name', exact: true }).click();
    await expect(button('Size (bytes)')).toBeVisible();
    if (width < 600) await expect(button('Packed (bytes)')).toHaveCount(0);
  });
}

test('Packed metadata distinguishes TAR alignment, gzip/solid unknown values and non-solid 7z sizes', { timeout: 120_000 }, async (t) => {
  const { page, button } = await extractionPage(t);
  async function load(relative, count) {
    const picker = page.waitForEvent('filechooser');
    await button('Open archive…').click();
    await (await picker).setFiles(fileURLToPath(new URL(relative, import.meta.url)));
    await expect(page.getByText(`${count} entries`, { exact: true })).toBeVisible();
    await expect(button('Open archive…')).toBeEnabled();
  }
  await load('../../tests/fixtures/sample.tar', 5);
  await expect(page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true }).getByLabel('Packed bytes: 512', { exact: true })).toHaveText('512');
  await load('../../tests/fixtures/sample.tar.gz', 5);
  await expect(page.getByLabel('Packed bytes: unavailable', { exact: true })).toHaveCount(3);
  await expect(page.getByText("— means the file's packed size is unavailable.", { exact: true })).toBeVisible();
  await load('../../tests/fixtures/sample.7z', 6);
  await expect(page.getByLabel('Packed bytes: unavailable', { exact: true })).toHaveCount(3);
  await load('../../crates/peazip-core/tests/fixtures/extract-tree.7z', 7);
  await expect(page.getByRole('checkbox', { name: 'empty-file', exact: true }).getByLabel('Packed bytes: 0', { exact: true })).toHaveText('0');
  await expect(page.getByRole('checkbox', { name: 'docs/deep/data.bin', exact: true }).getByLabel('Packed bytes: 3', { exact: true })).toHaveText('3');
  await expect(page.getByLabel('Packed bytes: unavailable', { exact: true })).toHaveCount(0);
});

async function extractionFixture(entries) {
  const { stdout } = await promisify(execFile)('uv', ['run', 'python', '-c', `
import base64, io, json, sys, zipfile
out = io.BytesIO()
with zipfile.ZipFile(out, 'w', compression=zipfile.ZIP_STORED) as archive:
    for name, text in json.loads(sys.argv[1]):
        archive.writestr(name, text.encode())
print(base64.b64encode(out.getvalue()).decode())
`, JSON.stringify(entries)]);
  return { name: 'nested.zip', mimeType: 'application/zip', buffer: Buffer.from(stdout.trim(), 'base64') };
}

async function finishExtraction(page, button) {
  await button('Continue').click();
  const report = page.getByLabel(/^Extracted [0-9]+ entries to PeaZip-[0-9a-f-]{36}$/);
  await expect(report).toBeVisible();
  await expect(button('Extract…')).toBeEnabled();
  return (await report.getAttribute('aria-label')).split(' to ')[1];
}

test('Extraction dialog supports All without selecting rows and Files only with independent output checks', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree } = await extractionPage(t);
  await choose(await extractionFixture([['docs/你好.txt', 'hello 中文'], ['other/data.bin', 'binary text'], ['empty/', '']]));
  await expect(button('Extract…')).toBeEnabled();
  await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
  for (const cancel of ['Cancel', 'Escape']) {
    await button('Extract…').click();
    const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
    await expect(modal.getByRole('radio', { name: 'Displayed entries', exact: true })).toBeChecked();
    await expect(modal.getByRole('radio', { name: 'Selected entries', exact: true })).toBeDisabled();
    if (cancel === 'Escape') await page.keyboard.press('Escape');
    else await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(modal).toHaveCount(0);
  }
  assert.deepEqual(await page.evaluate(() => window.pickerCalls), []);
  assert.deepEqual(await tree(), sentinelTree);
  await button('Extract…').click();
  await page.getByRole('radio', { name: 'All entries', exact: true }).click();
  const full = await finishExtraction(page, button);
  const fullTree = { docs: { '你好.txt': Array.from(Buffer.from('hello 中文')) }, other: { 'data.bin': Array.from(Buffer.from('binary text')) }, empty: {} };
  assert.deepEqual(await tree(), { ...sentinelTree, [full]: fullTree });
  await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true }).click();
  await button('Extract…').click();
  const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
  await expect(modal.getByRole('radio', { name: 'Selected entries', exact: true })).toBeChecked();
  await modal.getByRole('radio', { name: 'Files only (no folders)', exact: true }).click();
  const selected = await finishExtraction(page, button);
  const before = { ...sentinelTree, [full]: fullTree, [selected]: { '你好.txt': Array.from(Buffer.from('hello 中文')) } };
  assert.deepEqual(await tree(), before);
  await expect(page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true })).toBeChecked();
  await button('Extract…').click();
  await modal.getByRole('radio', { name: 'All entries', exact: true }).click();
  const flat = await finishExtraction(page, button);
  const flatTree = { ...before, [flat]: { '你好.txt': Array.from(Buffer.from('hello 中文')), 'data.bin': Array.from(Buffer.from('binary text')) } };
  assert.deepEqual(await tree(), flatTree);
  await button('Clear selection').click();
  await page.getByRole('checkbox', { name: 'empty/', exact: true }).click();
  await button('Extract…').click();
  await button('Continue').click();
  await expect(page.getByText(/^Could not extract archive:.*no files selected/)).toBeVisible();
  assert.deepEqual(await tree(), flatTree);
  await expect(page.getByRole('checkbox', { name: 'empty/', exact: true })).toBeChecked();
  await button('Extract…').click();
  await modal.getByRole('radio', { name: 'Full paths', exact: true }).click();
  const empty = await finishExtraction(page, button);
  assert.deepEqual(await tree(), { ...flatTree, [empty]: { empty: {} } });
  assert.ok((await page.evaluate(() => window.pickerCalls)).every(call => call.active));
});

test('Displayed extraction follows the current folder, handles implicit directories and never falls back from an empty view', { timeout: 120_000 }, async (t) => {
  const { page, button, tree } = await extractionPage(t);
  const picker = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await picker).setFiles(fileURLToPath(new URL('../../tests/fixtures/sample.tar', import.meta.url)));
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await button('Folder view').click();
  const path = page.getByRole('textbox', { name: 'Archive path', exact: true });
  const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
  async function go(directory, row) {
    await path.fill(directory);
    await path.press('Enter');
    await expect(path).toHaveValue(directory);
    if (row) await expect(page.getByRole('checkbox', { name: row, exact: true })).toBeVisible();
    else await expect(page.getByRole('checkbox')).toHaveCount(0);
  }
  await go('docs/', 'docs/你好.txt');
  await button('Extract…').click();
  await expect(modal.getByText('Current view: /docs/', { exact: true })).toBeVisible();
  await expect(modal.getByRole('radio', { name: 'Displayed entries', exact: true })).toBeChecked();
  const docs = await finishExtraction(page, button);
  const docsTree = { docs: { '你好.txt': Array.from(Buffer.from('tar 你好\n')), empty: {}, nested: { 'data.bin': [0, 127, 255] } } };
  let expected = { ...sentinelTree, [docs]: docsTree };
  assert.deepEqual(await tree(), expected);
  await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
  await go('docs/nested/', 'docs/nested/data.bin');
  await button('Extract…').click();
  const nested = await finishExtraction(page, button);
  expected = { ...expected, [nested]: { docs: { nested: { 'data.bin': [0, 127, 255] } } } };
  assert.deepEqual(await tree(), expected);
  await go('docs/empty/');
  await button('Extract…').click();
  await expect(modal.getByRole('radio', { name: 'Displayed entries', exact: true })).toBeChecked();
  await expect(modal.getByRole('radio', { name: 'Displayed entries', exact: true })).toBeDisabled();
  await expect(modal.getByRole('button', { name: 'Continue', exact: true })).toBeDisabled();
  assert.equal((await page.evaluate(() => window.pickerCalls)).length, 2);
  await modal.getByRole('radio', { name: 'All entries', exact: true }).click();
  const all = await finishExtraction(page, button);
  const allTree = { ...docsTree, 'other.txt': Array.from(Buffer.from('not selected\n')) };
  expected = { ...expected, [all]: allTree };
  assert.deepEqual(await tree(), expected);
  await button('All entries').click();
  await expect(page.getByRole('checkbox')).toHaveCount(5);
  await button('Extract…').click();
  await expect(modal.getByText('All entries view', { exact: true })).toBeVisible();
  await expect(modal.getByRole('radio', { name: 'Displayed entries', exact: true })).toBeChecked();
  const flatView = await finishExtraction(page, button);
  assert.deepEqual(await tree(), { ...expected, [flatView]: allTree });
  await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
});

test('Invert selection affects current folder contents while keeping selections outside the view', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree } = await extractionPage(t);
  await expect(button('Invert selection')).toBeDisabled();
  await choose(await extractionFixture([['docs/a.txt', 'first'], ['docs/b.txt', 'second'], ['outside.txt', 'outside']]));
  await page.getByRole('checkbox', { name: 'docs/a.txt', exact: true }).click();
  await page.getByRole('checkbox', { name: 'outside.txt', exact: true }).click();
  await button('Folder view').click();
  await button('Open folder docs/').click();
  await expect(page.getByRole('checkbox', { name: 'docs/b.txt', exact: true })).toBeVisible();
  await button('Invert selection').click();
  await expect(page.getByRole('checkbox', { name: 'docs/a.txt', exact: true })).not.toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'docs/b.txt', exact: true })).toBeChecked();
  await button('Extract…').click();
  await expect(button('Invert selection')).toBeDisabled();
  const output = await finishExtraction(page, button);
  assert.deepEqual(await tree(), { ...sentinelTree, [output]: { docs: { 'b.txt': Array.from(Buffer.from('second')) }, 'outside.txt': Array.from(Buffer.from('outside')) } });
  await button('Invert selection').click();
  await expect(page.getByRole('checkbox', { name: 'docs/a.txt', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'docs/b.txt', exact: true })).not.toBeChecked();
  await button('Archive root').click();
  await expect(page.getByRole('checkbox', { name: 'outside.txt', exact: true })).toBeChecked();
  await button('Edit').click();
  await page.getByRole('menuitem', { name: 'Invert selection', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'outside.txt', exact: true })).not.toBeChecked();
  await button('All entries').click();
  await expect(page.getByRole('checkbox', { name: 'docs/b.txt', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'docs/a.txt', exact: true })).not.toBeChecked();
});

test('Invert selection handles overlapping parent rows without reintroducing deselected descendants', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree } = await extractionPage(t);
  await choose(await extractionFixture([['docs/', ''], ['docs/a.txt', 'first'], ['docs/b.txt', 'second']]));
  await page.getByRole('checkbox', { name: 'docs/', exact: true }).click();
  await page.getByRole('checkbox', { name: 'docs/a.txt', exact: true }).click();
  await button('Invert selection').click();
  await expect(page.getByRole('checkbox', { name: 'docs/a.txt', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'docs/b.txt', exact: true })).not.toBeChecked();
  await button('Extract…').click();
  const output = await finishExtraction(page, button);
  assert.deepEqual(await tree(), { ...sentinelTree, [output]: { docs: { 'a.txt': Array.from(Buffer.from('first')) } } });
  await button('Invert selection').click();
  await expect(page.getByRole('checkbox', { name: 'docs/a.txt', exact: true })).not.toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'docs/b.txt', exact: true })).toBeChecked();
});

test('Files-only duplicate names fail before output and Full paths retry preserves archive selection', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree } = await extractionPage(t);
  await choose(await extractionFixture([['a/same.txt', 'first'], ['b/same.txt', 'second'], ['empty/', '']]));
  await button('Select all').click();
  await button('Extract…').click();
  const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
  await modal.getByRole('radio', { name: 'Files only (no folders)', exact: true }).click();
  await button('Continue').click();
  await expect(page.getByText(/^Could not extract archive:.*duplicate or conflicting archive path:/)).toBeVisible();
  assert.deepEqual(await tree(), sentinelTree);
  await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
  await button('Extract…').click();
  await modal.getByRole('radio', { name: 'Full paths', exact: true }).click();
  const output = await finishExtraction(page, button);
  assert.deepEqual(await tree(), { ...sentinelTree, [output]: { a: { 'same.txt': Array.from(Buffer.from('first')) }, b: { 'same.txt': Array.from(Buffer.from('second')) }, empty: {} } });
  await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
});

test('Web extraction writes selected bytes and empty directories through real OPFS and Rust worker', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree, downloads } = await extractionPage(t);
  await choose();
  await page.getByRole('checkbox', { name: '你好.txt', exact: true }).click();
  await page.getByRole('checkbox', { name: '空目录/', exact: true }).click();
  await button('Extract…').click();
  await button('Continue').click();
  const report = page.getByText(/^Extracted 2 entries to PeaZip-[0-9a-f-]{36}$/);
  await expect(report).toBeVisible();
  const output = (await report.innerText()).split(' to ')[1];
  assert.match(output, /^PeaZip-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  assert.deepEqual(await tree(), {
    ...sentinelTree,
    [output]: { '你好.txt': Array.from(Buffer.from('你好\n')), '空目录': {} },
  }, 'must write exact decompressed bytes and an empty directory, omit notes.txt, and preserve sentinel');
  assert.deepEqual(await page.evaluate(() => window.pickerCalls), [{ mode: 'readwrite', active: true }]);
  await expect.poll(() => page.evaluate(() => window.extractionWorkers)).toEqual([{ terminated: true, codes: [1] }]);
  assert.equal(downloads.length, 0, 'directory extraction must not download a ZIP');
  t.diagnostic('Real Chromium OPFS file APIs and Rust extraction worker verified; native external picker/permissions are not exercised.');
});

test('Web extraction handles picker cancellation and denial, CRC failure, then recovers', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree } = await extractionPage(t);
  await choose();
  const selected = page.getByRole('checkbox', { name: '你好.txt', exact: true });
  await selected.click();
  for (const failure of ['AbortError', 'NotAllowedError']) {
    await page.evaluate((name) => { window.pickerFailure = name; }, failure);
    await button('Extract…').click();
    await button('Continue').click();
    await expect(page.getByText(failure === 'AbortError' ? 'Extraction cancelled.' : /^Could not extract archive:.*Test picker rejection/)).toBeVisible();
    await expect(button('Extract…')).toBeEnabled();
    await expect(selected).toBeChecked();
    assert.deepEqual(await tree(), sentinelTree);
    assert.deepEqual(await page.evaluate(() => window.extractionWorkers), [], 'picker rejection must not start a worker');
  }
  await page.evaluate(() => { window.pickerFailure = null; });
  const corrupt = Buffer.from(await readFile(fixture('sample.zip')));
  assert.equal(corrupt.readUInt32LE(0), 0x04034b50);
  const payload = 30 + corrupt.readUInt16LE(26) + corrupt.readUInt16LE(28);
  corrupt[payload] ^= 1; // Keep headers and central directory intact so listing still succeeds.
  await choose({ name: 'bad-crc.zip', mimeType: 'application/zip', buffer: corrupt });
  await selected.click();
  await button('Extract…').click();
  await button('Continue').click();
  await expect(page.getByText(/^Could not extract archive:/)).toBeVisible();
  await expect(button('Extract…')).toBeEnabled();
  await expect(selected).toBeChecked();
  assert.deepEqual(await tree(), sentinelTree, 'CRC validation must finish before any output directory is created');
  await expect.poll(() => page.evaluate(() => window.extractionWorkers)).toEqual([{ terminated: true, codes: [0] }]);
  await choose();
  await selected.click();
  await button('Extract…').click();
  await button('Continue').click();
  await expect(page.getByText(/^Extracted 1 entries to PeaZip-/)).toBeVisible();
  const result = await tree();
  assert.deepEqual(result['sentinel.txt'], sentinelTree['sentinel.txt']);
  assert.equal(Object.keys(result).length, 2);
});

test('Web extraction is disabled with an explanation when directory picker capability is unavailable', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree } = await extractionPage(t, false);
  await choose();
  await button('Select all').click();
  await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
  await expect(button('Extract…')).toBeDisabled();
  await expect(page.getByText('Directory export is unavailable in this browser.', { exact: true })).toBeVisible();
  assert.deepEqual(await page.evaluate(() => window.pickerCalls), []);
  assert.deepEqual(await page.evaluate(() => window.extractionWorkers), []);
  assert.deepEqual(await tree(), sentinelTree);
});

test('Cancel extraction during real worker WASM loading preserves archive snapshot and selection', { timeout: 120_000 }, async (t) => {
  const { page, button, tree, downloads } = await extractionPage(t);
  // A created archive also gives us a downloadable snapshot to compare byte for byte.
  const pending = page.waitForEvent('filechooser');
  await button('Create archive…').click();
  await button('Continue').click();
  await (await pending).setFiles(fixture('sample.zip'));
  await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
  const selected = page.getByRole('checkbox', { name: 'sample.zip', exact: true });
  await selected.click();
  async function snapshot() {
    const pending = page.waitForEvent('download');
    await button('Download ZIP').click();
    const stream = await (await pending).createReadStream();
    const chunks = [];
    for await (const chunk of stream) chunks.push(chunk);
    return Buffer.concat(chunks);
  }
  const before = await snapshot();
  const downloadsBefore = downloads.length;
  let release, intercepted = false;
  const gate = new Promise((resolve) => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async (route) => {
    intercepted = true;
    await gate;
    await route.continue().catch(() => {});
  });
  try {
    await button('Extract…').click();
    await button('Continue').click();
    await expect.poll(() => intercepted).toBe(true);
    await expect(button('Cancel')).toBeEnabled();
    for (const name of ['Create archive…', 'Open archive…', 'Download ZIP', 'Select all', 'Clear selection', 'Extract…']) await expect(button(name)).toBeDisabled();
    await button('Cancel').click();
    await expect(page.getByText('Extraction cancelled.', { exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => window.extractionWorkers[1])).toEqual({ terminated: true, codes: [] });
    await expect.poll(() => page.workers().length).toBe(0);
    await expect(selected).toBeChecked();
    await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
    await expect(button('Cancel')).toBeDisabled();
    await expect(button('Extract…')).toBeEnabled();
    assert.deepEqual(await tree(), sentinelTree);
    assert.equal(downloads.length, downloadsBefore);
    assert.deepEqual(await snapshot(), before, 'cancel must preserve exact archive bytes');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
  }
  await expect(selected).toBeChecked();
  await button('Extract…').click();
  await button('Continue').click();
  await expect(page.getByText(/^Extracted 1 entries to PeaZip-/)).toBeVisible();
  assert.deepEqual(await tree(), {
    ...sentinelTree,
    [(await page.getByText(/^Extracted 1 entries to PeaZip-/).innerText()).split(' to ')[1]]: {
      'sample.zip': Array.from(await readFile(fixture('sample.zip'))),
    },
  });
});

test('Cancel during an OPFS write waits for abort and reports retained partial output', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree, downloads } = await extractionPage(t);
  await choose();
  const selected = page.getByRole('checkbox', { name: '你好.txt', exact: true });
  await selected.click();
  await page.evaluate(() => {
    const picker = window.showDirectoryPicker;
    function directory(handle) {
      return {
        getDirectoryHandle: async (...args) => directory(await handle.getDirectoryHandle(...args)),
        getFileHandle: async (...args) => {
          const file = await handle.getFileHandle(...args);
          return { createWritable: async () => {
            const writer = await file.createWritable();
            let rejectWrite;
            return {
              // Hold the write for deterministic cancellation, retaining real OPFS handles/abort.
              write: () => {
                window.exportWriteStarted = true;
                return new Promise((_, reject) => { rejectWrite = reject; });
              },
              close: () => writer.close(),
              abort: async () => {
                await writer.abort();
                window.exportWriteAborted = true;
                rejectWrite(new DOMException('Test write aborted', 'AbortError'));
              },
            };
          } };
        },
      };
    }
    window.showDirectoryPicker = async (options) => directory(await picker(options));
  });
  await button('Extract…').click();
  await button('Continue').click();
  await expect.poll(() => page.evaluate(() => window.exportWriteStarted === true)).toBe(true);
  await button('Cancel').click();
  await expect(page.getByText(/^Could not extract archive: Cancelled:.*completed: 0.*possibly unfinished: 你好.txt.*output may remain/)).toBeVisible();
  assert.equal(await page.evaluate(() => window.exportWriteAborted), true);
  await expect(selected).toBeChecked();
  await expect(button('Extract…')).toBeEnabled();
  await expect(button('Cancel')).toBeDisabled();
  const result = await tree();
  const output = Object.keys(result).find((name) => name.startsWith('PeaZip-'));
  assert.ok(output);
  assert.deepEqual(result, { ...sentinelTree, [output]: { '你好.txt': [] } });
  assert.equal(downloads.length, 0);
  t.diagnostic('Write timing injected around real OPFS writable; abort and retained empty file verified, external directory picker not tested.');
});

const detailColumnNames = ['Name', 'Type', 'Size (bytes)', 'Packed (bytes)'];
const resizeColumn = (page, name) => page.getByLabel(`Resize ${name} column`, { exact: true });

function detailSortButton(page, name) {
  return resizeColumn(page, name).locator('..').getByRole('button');
}

async function detailHeader(page, name) {
  const handle = resizeColumn(page, name);
  await expect(handle).toHaveCount(1);
  return handle.evaluate((node, name) => {
    // The labeled nsh handle and sort button share a column cell, not the whole header.
    for (let cell = node.parentElement; cell; cell = cell.parentElement) {
      const buttons = [...cell.querySelectorAll('[role="button"],button')];
      if (buttons.length === 1) {
        const { x, y, width, height } = cell.getBoundingClientRect();
        return { x, y, width, height };
      }
    }
    throw new Error(`No header cell for ${name}`);
  }, name);
}

async function detailScroller(page, action, value = 0) {
  return page.getByRole('button', { name: 'Name', exact: true }).evaluate((node, { action, value }) => {
    for (let parent = node.parentElement; parent; parent = parent.parentElement) {
      const overflow = getComputedStyle(parent).overflowX;
      if ((overflow === 'auto' || overflow === 'scroll') && (action === 'inspect' || parent.scrollWidth > parent.clientWidth + 1)) {
        if (action === 'set') parent.scrollLeft = value;
        const { x, width } = parent.getBoundingClientRect();
        return { left: parent.scrollLeft, maximum: parent.scrollWidth - parent.clientWidth, x, width };
      }
    }
    throw new Error('Detailed list must have an overflowing horizontal scroll container');
  }, { action, value });
}

async function dragDetailColumn(page, name, delta) {
  await expect(resizeColumn(page, name)).toBeEnabled();
  await resizeColumn(page, name).scrollIntoViewIfNeeded();
  const handle = await resizeColumn(page, name).boundingBox();
  assert.ok(handle && handle.width > 0 && handle.height > 0);
  const before = await detailHeader(page, name);
  const x = handle.x + handle.width / 2, y = handle.y + handle.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  try { await page.mouse.move(x + delta, y, { steps: 16 }); }
  finally { await page.mouse.up(); }
  await expect.poll(async () => Math.abs((await detailHeader(page, name)).width - before.width - delta)).toBeLessThan(3);
  return (await detailHeader(page, name)).width;
}

async function detailWidths(page) {
  return Promise.all(detailColumnNames.map(async name => (await detailHeader(page, name)).width));
}

async function expectDetailWidths(page, expected) {
  await expect.poll(async () => Math.max(...(await detailWidths(page)).map((width, index) => Math.abs(width - expected[index])))).toBeLessThan(2);
}

async function expectDetailAlignment(page) {
  const headers = await Promise.all(detailColumnNames.map(name => detailHeader(page, name)));
  const rows = page.getByRole('checkbox');
  assert.ok(await rows.count(), 'alignment must check actual rows');
  for (const row of await rows.all()) {
    const path = await row.getAttribute('aria-label');
    const directory = path.endsWith('/');
    const basename = path.replace(/\/$/, '').split('/').at(-1);
    const type = directory ? 'Folder' : basename.includes('.') ? basename.split('.').at(-1).toUpperCase() : 'File';
    const cells = [await row.boundingBox(), await row.getByLabel(type, { exact: true }).boundingBox(),
      await row.getByLabel(/^Size in bytes:/).boundingBox(), await row.getByLabel(/^Packed bytes:/).boundingBox()];
    assert.ok(cells.every(Boolean), `all ${path} cells must be laid out`);
    for (let index = 0; index < headers.length; index++) {
      assert.ok(Math.abs(cells[index].x - headers[index].x) < 3,
        `${path}: ${detailColumnNames[index]} starts at ${cells[index].x}, header at ${headers[index].x}`);
      if (index > 0) assert.ok(Math.abs(cells[index].width - headers[index].width) < 3,
        `${path}: ${detailColumnNames[index]} body/header widths differ`);
      if (index > 0) {
        const textInset = locator => locator.evaluate(node => {
          const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
          let text;
          while ((text = walker.nextNode())) {
            if (!text.textContent.trim()) continue;
            const range = document.createRange();
            range.selectNodeContents(text);
            const rectangle = range.getBoundingClientRect();
            if (rectangle.width) return rectangle.x - node.getBoundingClientRect().x;
          }
          throw new Error('Column has no visible text');
        });
        const bodyCell = index === 1 ? row.getByLabel(type, { exact: true })
          : row.getByLabel(index === 2 ? /^Size in bytes:/ : /^Packed bytes:/);
        const headerInset = await textInset(detailSortButton(page, detailColumnNames[index]));
        const bodyInset = await textInset(bodyCell);
        assert.ok(Math.abs(headerInset - bodyInset) < 2,
          `${path}: ${detailColumnNames[index]} text inset differs: header=${headerInset}, body=${bodyInset}`);
      }
    }
    assert.ok(Math.abs(cells[1].x - cells[0].x - headers[0].width) < 3,
      `${path}: Name cell width must include its checkbox and match the header`);
  }
}

async function saveResizedColumns(page, name, requireScroll = true) {
  const { createHash } = await import('node:crypto');
  const directory = fileURLToPath(new URL('../../target/', import.meta.url));
  const sourceHashes = {};
  for (const source of ['Cargo.lock', 'crates/peazip-ui/src/columns.rs', 'crates/peazip-ui/src/explorer.rs',
    'crates/peazip-ui/src/lib.rs', 'crates/peazip-ui/src/navigation.rs', 'crates/peazip-ui/src/filesystem.rs',
    'web/tests/browser.test.mjs', 'target/web-dist/peazip.wasm']) {
    sourceHashes[source] = createHash('sha256').update(await readFile(new URL(`../../${source}`, import.meta.url))).digest('hex');
  }
  const scroll = await detailScroller(page, requireScroll ? 'read' : 'inspect');
  if (requireScroll) assert.ok(scroll.maximum > 0 && scroll.left > 0, 'screenshot must prove resized and horizontally scrolled state');
  const metadata = { viewport: page.viewportSize(),
    theme: await page.evaluate(() => matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'),
    widths: await detailWidths(page), scroll, sourceHashes,
    rows: await page.getByRole('checkbox').evaluateAll(nodes => nodes.map(node => ({
      path: node.getAttribute('aria-label'), checked: node.getAttribute('aria-checked'),
    }))),
  };
  await page.screenshot({ path: resolve(directory, `${name}.png`), animations: 'disabled' });
  await writeFile(resolve(directory, `${name}.json`), JSON.stringify(metadata, null, 2) + '\n');
}

test('Detailed columns drag and scroll together without changing partial/hidden selection or extracted bytes', { timeout: 120_000 }, async (t) => {
  const { page, button, tree, downloads } = await extractionPage(t, true, { viewport: { width: 1024, height: 600 }, colorScheme: 'light' });
  const pending = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await pending).setFiles(fileURLToPath(new URL('../../tests/fixtures/sample.tar', import.meta.url)));
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await button('Folder view').click();
  const docs = page.getByRole('checkbox', { name: 'docs/', exact: true });
  await docs.click();
  await button('Open folder docs/').click();
  const chinese = page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true });
  await chinese.click();
  await button('Archive root').click();
  await expect(docs).toHaveAttribute('aria-checked', 'mixed');
  const other = page.getByRole('checkbox', { name: 'other.txt', exact: true });
  await other.click();
  await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
  for (const name of detailColumnNames) await expect(resizeColumn(page, name)).toBeEnabled();
  await expectDetailAlignment(page);
  await dragDetailColumn(page, 'Name', 240);
  const widths = await detailWidths(page);
  const scroll = await detailScroller(page, 'read');
  assert.ok(scroll.maximum > 100, `drag must make the detailed table scrollable: ${JSON.stringify(scroll)}`);
  await detailScroller(page, 'set', 0);
  const headerBefore = await detailHeader(page, 'Name');
  const rowBefore = await other.boundingBox();
  const packedBefore = await other.getByLabel(/^Packed bytes:/).boundingBox();
  const amount = Math.min(160, scroll.maximum);
  await detailScroller(page, 'set', amount);
  await expect.poll(async () => (await detailScroller(page, 'read')).left).toBeGreaterThan(amount - 2);
  const headerAfter = await detailHeader(page, 'Name');
  const rowAfter = await other.boundingBox();
  const packedAfter = await other.getByLabel(/^Packed bytes:/).boundingBox();
  assert.ok(Math.abs(headerBefore.x - headerAfter.x - amount) < 3, 'header must move by the horizontal scroll amount');
  assert.ok(Math.abs(rowBefore.x - rowAfter.x - amount) < 3, 'row must move with its header');
  assert.ok(Math.abs(packedBefore.x - packedAfter.x - amount) < 3, 'packed cell must share the same scroll offset');
  await expectDetailAlignment(page);
  await saveResizedColumns(page, 'columns-resized-scrolled-1024-light');
  await expect(docs).toHaveAttribute('aria-checked', 'mixed');
  await expect(other).toBeChecked();
  await detailScroller(page, 'set', 0);
  await button('All entries').click();
  await expect(page.getByRole('checkbox')).toHaveCount(5);
  await expectDetailWidths(page, widths);
  await expectDetailAlignment(page);
  await expect(chinese).not.toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'docs/nested/data.bin', exact: true })).toBeChecked();
  await button('Size (bytes)').click();
  await expectDetailWidths(page, widths);
  await expectDetailAlignment(page);
  const filter = page.getByRole('textbox', { name: 'Filter names', exact: true });
  await filter.fill('你好');
  await filter.press('Enter');
  await expect(page.getByRole('checkbox')).toHaveCount(1);
  await expect(chinese).not.toBeChecked();
  await expect(page.getByText('3 selected', { exact: true })).toBeVisible();
  await expectDetailWidths(page, widths);
  await expectDetailAlignment(page);
  await button('Extract…').click();
  const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
  await expect(modal.getByRole('radio', { name: 'Selected entries', exact: true })).toBeChecked();
  const output = await finishExtraction(page, button);
  assert.deepEqual(await tree(), { ...sentinelTree, [output]: {
    docs: { empty: {}, nested: { 'data.bin': [0, 127, 255] } },
    'other.txt': Array.from(Buffer.from('not selected\n')),
  } }, 'resizing, filtering and horizontal scrolling must not alter selected paths or binary bytes');
  assert.equal(downloads.length, 0);
  await button('Clear filter').click();
  await button('Folder view').click();
  await expect(docs).toHaveAttribute('aria-checked', 'mixed');
  await expect(other).toBeChecked();
  await expectDetailWidths(page, widths);
  await expectDetailAlignment(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ colorScheme: 'dark' });
  await dragDetailColumn(page, 'Name', 300);
  const desktopScroll = await detailScroller(page, 'read');
  assert.ok(desktopScroll.maximum > 0);
  await detailScroller(page, 'set', desktopScroll.maximum);
  await expectDetailAlignment(page);
  await saveResizedColumns(page, 'columns-resized-scrolled-1440-dark');
  t.diagnostic('New resized/scrolled desktop screenshots and viewport/theme/source/Wasm hashes: target/columns-resized-scrolled-{1024-light,1440-dark}.{png,json}');
});

test('Detailed column keyboard resizing uses 10/50 pixel steps and Home restores automatic widths', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 1024, height: 900 });
  await choose();
  const automatic = await detailWidths(page);
  for (const [index, name] of detailColumnNames.entries()) {
    const handle = resizeColumn(page, name);
    await handle.scrollIntoViewIfNeeded();
    await handle.focus();
    await expect(handle).toBeFocused();
    const before = (await detailHeader(page, name)).width;
    for (const [key, delta] of [['ArrowRight', 10], ['Shift+ArrowRight', 60], ['ArrowLeft', 50], ['Shift+ArrowLeft', 0]]) {
      await page.keyboard.press(key);
      await expect.poll(async () => Math.abs((await detailHeader(page, name)).width - before - delta)).toBeLessThan(2);
      await expect(page.getByRole('checkbox')).toHaveCount(2);
      await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
    }
    await page.keyboard.press('Shift+ArrowRight');
    await page.keyboard.press('Home');
    await expect.poll(async () => Math.abs((await detailHeader(page, name)).width - automatic[index])).toBeLessThan(2);
  }
  await dragDetailColumn(page, 'Name', 200);
  const grown = (await detailHeader(page, 'Name')).width;
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect.poll(async () => Math.abs((await detailHeader(page, 'Name')).width - grown)).toBeLessThan(2);
  await resizeColumn(page, 'Name').focus();
  await page.keyboard.press('Home');
  await expect.poll(async () => (await detailHeader(page, 'Name')).width).toBeGreaterThan(automatic[0] + 100);
  const reset = await detailWidths(page);
  await page.setViewportSize({ width: 1024, height: 900 });
  await expectDetailWidths(page, automatic);
  assert.ok(reset[0] > automatic[0], 'Home must restore viewport-responsive automatic Name width');
  await expectDetailAlignment(page);
  await expect(button('Open archive…')).toBeEnabled();
});

test('Narrow detailed lists ignore saved widths, hide resize handles and keep Size/Packed inside the viewport', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 1024, height: 900 });
  await choose();
  await dragDetailColumn(page, 'Name', 200);
  const custom = await detailWidths(page);
  for (const width of [360, 430, 759]) {
    await page.setViewportSize({ width, height: 800 });
    for (const name of detailColumnNames) await expect(resizeColumn(page, name)).toHaveCount(0);
    await expect(button('Type')).toHaveCount(0);
    await expect(button('Size (bytes)')).toBeVisible();
    await expect(button('Packed (bytes)')).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    for (const row of await page.getByRole('checkbox').all()) {
      const cell = await row.getByLabel(/^Size in bytes:/).boundingBox();
      assert.ok(cell.x >= -1 && cell.x + cell.width <= width + 1, 'narrow Size must remain on screen despite saved widths');
    }
    await button('Layout').click();
    await page.getByRole('menuitemcheckbox', { name: 'Sort by packed size', exact: true }).click();
    await expect(button('Packed (bytes)')).toBeVisible();
    await expect(button('Size (bytes)')).toHaveCount(0);
    for (const row of await page.getByRole('checkbox').all()) {
      const cell = await row.getByLabel(/^Packed bytes:/).boundingBox();
      assert.ok(cell.x >= -1 && cell.x + cell.width <= width + 1, 'narrow Packed must remain on screen');
    }
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await button('Layout').click();
    await page.getByRole('menuitemcheckbox', { name: 'Sort by name', exact: true }).click();
  }
  await page.setViewportSize({ width: 1024, height: 900 });
  await expectDetailWidths(page, custom);
  await expectDetailAlignment(page);
});

test('Detailed column handles cannot resize through a modal or while a real archive worker is busy', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 1024, height: 900 });
  await choose();
  const before = await detailWidths(page);
  await button('Create archive…').click();
  const modal = page.getByRole('group', { name: 'Create archive', exact: true });
  await expect(modal).toBeVisible();
  for (const name of detailColumnNames) await expect(resizeColumn(page, name)).toBeDisabled();
  await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expectDetailWidths(page, before);
  let release, intercepted = false;
  const gate = new Promise(resolve => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async route => {
    intercepted = true;
    await gate;
    await route.continue().catch(() => {});
  });
  try {
    await button('Test archive').click();
    await expect.poll(() => intercepted).toBe(true);
    await expect(page.getByText('Testing archive…', { exact: true })).toBeVisible();
    for (const name of detailColumnNames) await expect(resizeColumn(page, name)).toBeDisabled();
    const handle = resizeColumn(page, 'Name');
    const box = await handle.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    try { await page.mouse.move(box.x + 100, box.y + box.height / 2, { steps: 8 }); }
    finally { await page.mouse.up(); }
    await expectDetailWidths(page, before);
    await button('Cancel').click();
    await expect(page.getByText('Archive test cancelled.', { exact: true })).toBeVisible();
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
  }
  for (const name of detailColumnNames) await expect(resizeColumn(page, name)).toBeEnabled();
  await expectDetailWidths(page, before);
  await expectDetailAlignment(page);
});

test('Detailed overflowing virtual list reaches its final directory and opens it after scrolling both axes', { timeout: 120_000 }, async (t) => {
  const { page, button } = await extractionPage(t, true, { viewport: { width: 1024, height: 700 } });
  const directories = Array.from({ length: 180 }, (_, index) => [`dir-${String(index).padStart(3, '0')}/`, '']);
  const file = await extractionFixture([...directories, ['z-last/', ''], ['z-last/payload.txt', 'last directory payload']]);
  await button('Folder view').click();
  const picker = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await picker).setFiles(file);
  await expect(page.getByText('182 entries', { exact: true })).toBeVisible();
  const first = page.getByRole('checkbox', { name: 'dir-000/', exact: true });
  await expect(first).toBeVisible();
  const initialRows = await page.getByRole('checkbox').count();
  assert.ok(initialRows < 181, 'large directory fixture must actually exercise virtualized rows');
  await dragDetailColumn(page, 'Name', 240);
  const horizontal = await detailScroller(page, 'read');
  assert.ok(horizontal.maximum > 100);
  await detailScroller(page, 'set', horizontal.maximum);
  const vertical = await first.evaluateHandle(node => {
    for (let ancestor = node.parentElement; ancestor; ancestor = ancestor.parentElement) {
      if (['auto', 'scroll'].includes(getComputedStyle(ancestor).overflowY)
          && ancestor.scrollHeight > ancestor.clientHeight + 1) return ancestor;
    }
    throw new Error('Virtualized body has no vertical scroller');
  });
  try {
    const box = await vertical.asElement().boundingBox();
    const viewport = await detailScroller(page, 'read');
    const x = Math.min(viewport.x + viewport.width - 24, 1000);
    const y = box.y + Math.min(box.height / 2, 120);
    await page.mouse.move(x, y);
    await page.mouse.wheel(0, 100000);
    await expect.poll(() => vertical.evaluate(node => node.scrollTop)).toBeGreaterThan(100);
    await expect.poll(() => vertical.evaluate(node => node.scrollHeight - node.clientHeight - node.scrollTop)).toBeLessThan(3);
    const last = page.getByRole('checkbox', { name: 'z-last/', exact: true });
    const openLast = button('Open folder z-last/');
    await expect(last).toBeVisible();
    await expect(openLast).toBeInViewport({ ratio: 1 });
    const openBox = await openLast.boundingBox();
    const scrollBox = await detailScroller(page, 'read');
    assert.ok(openBox.x >= scrollBox.x - 1 && openBox.x + openBox.width <= scrollBox.x + scrollBox.width + 1,
      'the final directory open button must be fully inside the horizontal table viewport');
    assert.ok(openBox.y >= box.y - 1 && openBox.y + openBox.height <= box.y + box.height + 1,
      'the final directory row must not be clipped below the virtualized body');
    await expectDetailAlignment(page);
    await openLast.click();
    await expect(page.getByRole('textbox', { name: 'Archive path', exact: true })).toHaveValue('z-last/');
    await expect(page.getByRole('checkbox')).toHaveCount(1);
    await expect(page.getByRole('checkbox', { name: 'z-last/payload.txt', exact: true })).toBeVisible();
    await expect(page.getByRole('checkbox', { name: 'z-last/payload.txt', exact: true })
      .getByLabel(`Size in bytes: ${Buffer.byteLength('last directory payload')}`, { exact: true })).toBeVisible();
    await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
  } finally {
    await vertical.dispose();
  }
});

const adjustedDetailWidths = [240, 72, 72, 72];

async function changeEveryDetailColumn(page) {
  for (const name of detailColumnNames) {
    const handle = resizeColumn(page, name);
    await handle.scrollIntoViewIfNeeded();
    await handle.focus();
    await page.keyboard.press('Shift+ArrowRight');
  }
  return detailWidths(page);
}

async function adjustColumnsMenu(page) {
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  const item = page.getByRole('menuitem', { name: 'Auto adjust columns', exact: true });
  await expect(item).toBeEnabled();
  await item.click();
  await expect(page.getByRole('menu')).toHaveCount(0);
}

async function detailSelectionState(page) {
  return {
    rows: await page.getByRole('checkbox').evaluateAll(nodes => nodes.map(node => ({
      path: node.getAttribute('aria-label'), checked: node.getAttribute('aria-checked'),
    }))),
    selected: await page.getByText(/^\d+ selected$/, { exact: true }).innerText(),
    filter: await page.getByRole('textbox', { name: 'Filter names', exact: true }).inputValue(),
    path: await page.getByRole('textbox', { name: 'Archive path', exact: true }).inputValue(),
  };
}

test('Auto adjust columns restores fixed legacy widths idempotently without changing selection, sort, filter or snapshot', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree } = await extractionPage(t, true, { viewport: { width: 1024, height: 600 } });
  await choose(await extractionFixture([['dir/long-name.txt', 'first'], ['outside.txt', 'outside'], ['dir/data.bin', 'binary']]));
  await page.getByRole('checkbox', { name: 'outside.txt', exact: true }).click();
  await page.getByRole('checkbox', { name: 'dir/data.bin', exact: true }).click();
  await button('Size (bytes)').click();
  const filter = page.getByRole('textbox', { name: 'Filter names', exact: true });
  await filter.fill('dir/');
  await filter.press('Enter');
  await expect(page.getByRole('checkbox')).toHaveCount(2);
  const before = await detailSelectionState(page);
  const snapshot = await downloadArchive(page, button('Download ZIP'));
  const changed = await changeEveryDetailColumn(page);
  assert.ok(changed.every((width, index) => Math.abs(width - adjustedDetailWidths[index]) > 2), 'all four columns must differ from reset widths');
  for (let iteration = 0; iteration < 2; iteration++) {
    await adjustColumnsMenu(page);
    await expectDetailWidths(page, adjustedDetailWidths);
    await expectDetailAlignment(page);
    assert.deepEqual(await detailSelectionState(page), before);
    await button('Layout').click();
    await expect(page.getByRole('menuitemcheckbox', { name: 'Sort by size', exact: true })).toBeChecked();
    await page.keyboard.press('Escape');
    assert.deepEqual(await downloadArchive(page, button('Download ZIP')), snapshot);
    assert.deepEqual(await tree(), sentinelTree);
  }
  await saveResizedColumns(page, 'columns-adjusted-1024-light', false);
  // Fixed legacy widths must not be confused with Home's viewport-responsive auto width.
  await resizeColumn(page, 'Name').focus();
  await page.keyboard.press('Home');
  await expect.poll(async () => (await detailHeader(page, 'Name')).width).toBeGreaterThan(240);
  await expect.poll(async () => (await detailHeader(page, 'Type')).width).toBe(72);
  await adjustColumnsMenu(page);
  await expectDetailWidths(page, adjustedDetailWidths);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ colorScheme: 'dark' });
  await expectDetailWidths(page, adjustedDetailWidths);
  await expectDetailAlignment(page);
  assert.deepEqual(await detailSelectionState(page), before);
  await saveResizedColumns(page, 'columns-adjusted-1440-dark', false);
});

test('Auto adjust columns shortcut respects text editing and narrow execution persists into the wide layout', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 1024, height: 600 });
  await choose();
  let changed = await changeEveryDetailColumn(page);
  await button('Name').focus();
  await page.keyboard.press('ControlOrMeta+Alt+a');
  await expectDetailWidths(page, adjustedDetailWidths);
  changed = await changeEveryDetailColumn(page);
  for (const [label, value] of [['Filter names', 'docs'], ['Archive path', '/']]) {
    const input = page.getByRole('textbox', { name: label, exact: true });
    if (label === 'Filter names') await input.fill(value);
    await input.focus();
    await expect(input).toBeFocused();
    const before = await input.inputValue();
    await page.keyboard.press('ControlOrMeta+Alt+a');
    await expect(input).toBeFocused();
    await expect(input).toHaveValue(before);
    await expectDetailWidths(page, changed);
  }
  await button('Clear filter').click();
  await page.setViewportSize({ width: 360, height: 800 });
  for (const name of detailColumnNames) await expect(resizeColumn(page, name)).toHaveCount(0);
  await adjustColumnsMenu(page);
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await expect(button('Size (bytes)')).toBeVisible();
  await page.setViewportSize({ width: 1024, height: 600 });
  await expectDetailWidths(page, adjustedDetailWidths);
  await expectDetailAlignment(page);
  await expect(page.getByText('0 selected', { exact: true })).toBeVisible();
});

test('Auto adjust columns cannot bypass modal or worker busy eligibility through shortcuts or forced menu events', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 1024, height: 600 });
  await choose();
  const changed = await changeEveryDetailColumn(page);
  await button('Create archive…').click();
  const modal = page.getByRole('group', { name: 'Create archive', exact: true });
  await expect(modal).toBeVisible();
  await page.keyboard.press('ControlOrMeta+Alt+a');
  await expectDetailWidths(page, changed);
  // Dispatch directly to the underlying trigger/item so pointer-modal interception cannot
  // be mistaken for command-level eligibility enforcement.
  await button('Layout').dispatchEvent('click');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await expectDetailWidths(page, changed);
  await expect(modal).toBeVisible();
  await button('Layout').dispatchEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true });
  await page.evaluate(() => document.dispatchEvent(new KeyboardEvent('keydown', {
    key: 'a', code: 'KeyA', ctrlKey: true, altKey: true, bubbles: true, cancelable: true,
  })));
  await expect(page.getByRole('menu')).toHaveCount(0);
  await expectDetailWidths(page, changed);
  await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expectDetailWidths(page, changed);
  let release, intercepted = false;
  const gate = new Promise(resolve => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async route => {
    intercepted = true;
    await gate;
    await route.continue().catch(() => {});
  });
  try {
    await button('Test archive').click();
    await expect.poll(() => intercepted).toBe(true);
    await expect(page.getByText('Testing archive…', { exact: true })).toBeVisible();
    await button('Layout').click();
    const busyAdjust = page.getByRole('menuitem', { name: 'Auto adjust columns', exact: true });
    await expect(busyAdjust).toBeDisabled();
    await busyAdjust.dispatchEvent('click');
    await expectDetailWidths(page, changed);
    await page.keyboard.press('Escape');
    await button('Name').focus();
    await page.keyboard.press('ControlOrMeta+Alt+a');
    await expectDetailWidths(page, changed);
    await button('Cancel').click();
    await expect(page.getByText('Archive test cancelled.', { exact: true })).toBeVisible();
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
  }
  await adjustColumnsMenu(page);
  await expectDetailWidths(page, adjustedDetailWidths);
});

test('Auto adjust columns keeps large raw Size/Packed values accessible on a single virtualized row', { timeout: 120_000 }, async (t) => {
  const directory = await mkdtemp(resolve(tmpdir(), 'peazip-adjust-columns-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const archivePath = resolve(directory, 'large-stored.zip');
  const size = 12_345_678;
  await promisify(execFile)('uv', ['run', 'python', '-c', `
import random, sys, zipfile
with zipfile.ZipFile(sys.argv[1], 'w', compression=zipfile.ZIP_STORED) as archive:
    archive.writestr('big-entropy.bin', random.Random(17).randbytes(${size}))
    archive.writestr('small.txt', b'small')
    archive.writestr('empty/', b'')
`, archivePath]);
  const { page, button, choose } = await extractionPage(t, true, { viewport: { width: 1024, height: 600 } });
  await choose(archivePath);
  const large = page.getByRole('checkbox', { name: 'big-entropy.bin', exact: true });
  const small = page.getByRole('checkbox', { name: 'small.txt', exact: true });
  const beforeHeight = (await large.boundingBox()).height;
  await adjustColumnsMenu(page);
  await expectDetailWidths(page, adjustedDetailWidths);
  await expect(button('Size (bytes)')).toHaveText('Size');
  await expect(button('Packed (bytes)')).toHaveText('Packed');
  for (const [label, name] of [[`Size in bytes: ${size}`, 'Size (bytes)'], [`Packed bytes: ${size}`, 'Packed (bytes)']]) {
    const cell = large.getByLabel(label, { exact: true });
    await expect(cell).toBeVisible();
    await expect(cell).toHaveAttribute('aria-label', label);
    const geometry = await cell.evaluate(node => {
      const range = document.createRange();
      range.selectNodeContents(node);
      const rectangles = [...range.getClientRects()].filter(rect => rect.width > 0 && rect.height > 0);
      return { lines: [...new Set(rectangles.map(rect => Math.round(rect.y)))].length,
        height: node.getBoundingClientRect().height, width: node.getBoundingClientRect().width };
    });
    assert.equal(geometry.lines, 1, `${name} must not wrap its eight-digit value`);
    assert.ok(geometry.width <= 73 && geometry.width >= 71);
    assert.ok(geometry.height < 24, `${name} must retain single-line cell height`);
  }
  const after = await large.boundingBox();
  const smallBox = await small.boundingBox();
  assert.ok(Math.abs(after.height - beforeHeight) < 2, 'fixed widths must not increase the virtualized row height');
  assert.ok(Math.abs(after.height - smallBox.height) < 2, 'large and small values must use the same row height');
  await large.click();
  await expect(large).toBeChecked();
  await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
  await expect(button('Extract…')).toBeEnabled();
  await expectDetailAlignment(page);
});

async function toggleDetailColumn(page, label, enabled) {
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  const item = page.getByRole('menuitemcheckbox', { name: label, exact: true });
  await expect(item).toBeEnabled();
  if ((await item.getAttribute('aria-checked') === 'true') !== enabled) {
    await item.click();
  } else await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
}

async function expectColumnPreferences(page, expected) {
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  for (const [label, enabled] of Object.entries(expected)) {
    const item = page.getByRole('menuitemcheckbox', { name: label, exact: true });
    if (enabled) await expect(item).toBeChecked();
    else await expect(item).not.toBeChecked();
  }
  await page.keyboard.press('Escape');
}

const showColumnLabels = ['Show type column', 'Show size column', 'Show packed column'];

test('Column visibility Size toggle removes header/body/handle and restores width and selection', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 1024, height: 600 });
  await choose();
  const selected = page.getByRole('checkbox', { name: 'other.txt', exact: true });
  await selected.click();
  const widths = await changeEveryDetailColumn(page);
  const state = await detailSelectionState(page);
  await toggleDetailColumn(page, 'Show size column', false);
  await expect(button('Size (bytes)')).toHaveCount(0);
  await expect(resizeColumn(page, 'Size (bytes)')).toHaveCount(0);
  await expect(page.getByRole('checkbox').getByLabel(/^Size in bytes:/)).toHaveCount(0);
  await expect(selected).toBeChecked();
  assert.deepEqual(await detailSelectionState(page), state);
  await expectColumnPreferences(page, { 'Show type column': true, 'Show size column': false, 'Show packed column': true });
  await toggleDetailColumn(page, 'Show size column', true);
  await expectDetailWidths(page, widths);
  await expectDetailAlignment(page);
  assert.deepEqual(await detailSelectionState(page), state);
});

async function visibleDetailColumns(page, shown, wide = true) {
  const headers = [];
  for (const name of detailColumnNames) {
    const present = name === 'Name' || shown.includes(name);
    await expect(page.getByRole('button', { name, exact: true })).toHaveCount(present ? 1 : 0);
    await expect(resizeColumn(page, name)).toHaveCount(present && wide ? 1 : 0);
    if (present && wide) headers.push([name, await detailHeader(page, name)]);
  }
  for (const row of await page.getByRole('checkbox').all()) {
    const path = await row.getAttribute('aria-label');
    const basename = path.replace(/\/$/, '').split('/').at(-1);
    const type = path.endsWith('/') ? 'Folder' : basename.includes('.') ? basename.split('.').at(-1).toUpperCase() : 'File';
    const locators = { Type: row.getByLabel(type, { exact: true }),
      'Size (bytes)': row.getByLabel(/^Size in bytes:/), 'Packed (bytes)': row.getByLabel(/^Packed bytes:/) };
    for (const [name, cell] of Object.entries(locators)) await expect(cell).toHaveCount(shown.includes(name) ? 1 : 0);
    if (!wide) continue;
    const box = await row.boundingBox();
    let end = box.x;
    for (const [name, header] of headers) {
      assert.ok(Math.abs(header.x - end) < 3, `${path}: hidden columns must leave no gap before ${name}`);
      if (name !== 'Name') {
        const cell = await locators[name].boundingBox();
        assert.ok(Math.abs(cell.x - header.x) < 3 && Math.abs(cell.width - header.width) < 3,
          `${path}: ${name} header and body boundaries differ`);
      }
      end += header.width;
    }
    assert.ok(Math.abs(box.x + box.width - end) < 3, `${path}: checkbox row must end with the last shown column`);
  }
}

async function saveVisibilityScreenshot(page, name, preferences) {
  const { createHash } = await import('node:crypto');
  const hashes = {};
  for (const file of ['columns.rs', 'explorer.rs', 'lib.rs', 'commands.rs', 'menus.rs', 'filesystem.rs']) {
    hashes[file] = createHash('sha256').update(await readFile(new URL(`../../crates/peazip-ui/src/${file}`, import.meta.url))).digest('hex');
  }
  hashes.wasm = createHash('sha256').update(await readFile(resolve(root, 'peazip.wasm'))).digest('hex');
  const data = { viewport: page.viewportSize(),
    theme: await page.evaluate(() => matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'),
    preferences, sourceHashes: hashes,
    hashScope: 'Source file hashes are capture-time bytes; wasm is the served compiled artifact, not proof that later source formatting was rebuilt.',
    state: await detailSelectionState(page) };
  const output = fileURLToPath(new URL(`../../target/${name}`, import.meta.url));
  await page.screenshot({ path: `${output}.png`, animations: 'disabled' });
  await writeFile(`${output}.json`, JSON.stringify(data, null, 2) + '\n');
}

test('Column visibility hides every optional cell without gaps and preserves partial hidden selection, sort and real bytes', { timeout: 120_000 }, async (t) => {
  const { page, button, tree } = await extractionPage(t, true, { viewport: { width: 1024, height: 600 }, colorScheme: 'light' });
  const picker = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await picker).setFiles(fileURLToPath(new URL('../../tests/fixtures/sample.tar', import.meta.url)));
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await button('Folder view').click();
  await page.getByRole('checkbox', { name: 'docs/', exact: true }).click();
  await button('Open folder docs/').click();
  await page.getByRole('checkbox', { name: 'docs/你好.txt', exact: true }).click();
  await button('Archive root').click();
  await page.getByRole('checkbox', { name: 'other.txt', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'docs/', exact: true })).toHaveAttribute('aria-checked', 'mixed');
  await button('Layout').click();
  await page.getByRole('menuitemcheckbox', { name: 'Sort by packed size', exact: true }).click();
  const filter = page.getByRole('textbox', { name: 'Filter names', exact: true });
  await filter.fill('docs'); await filter.press('Enter');
  const state = await detailSelectionState(page);
  const snapshot = await downloadArchive(page, button('Download TAR'));
  const widths = await changeEveryDetailColumn(page);
  for (const label of showColumnLabels) await toggleDetailColumn(page, label, false);
  await visibleDetailColumns(page, []);
  const hidden = Object.fromEntries(showColumnLabels.map(label => [label, false]));
  await expectColumnPreferences(page, hidden);
  assert.deepEqual(await detailSelectionState(page), state);
  await saveVisibilityScreenshot(page, 'columns-visibility-1024-hidden', hidden);
  for (const label of showColumnLabels) await toggleDetailColumn(page, label, true);
  await expectDetailWidths(page, widths); await expectDetailAlignment(page);
  await saveVisibilityScreenshot(page, 'columns-visibility-1024-restored', Object.fromEntries(showColumnLabels.map(label => [label, true])));
  await page.setViewportSize({ width: 1440, height: 900 }); await page.emulateMedia({ colorScheme: 'dark' });
  for (const label of showColumnLabels) await toggleDetailColumn(page, label, false);
  await visibleDetailColumns(page, []);
  await saveVisibilityScreenshot(page, 'columns-visibility-1440-hidden', hidden);
  await adjustColumnsMenu(page);
  await expectColumnPreferences(page, hidden); await visibleDetailColumns(page, []);
  assert.equal((await detailHeader(page, 'Name')).width, 240);
  for (const label of showColumnLabels) await toggleDetailColumn(page, label, true);
  await expectDetailWidths(page, adjustedDetailWidths); await expectDetailAlignment(page);
  await saveVisibilityScreenshot(page, 'columns-visibility-1440-restored', Object.fromEntries(showColumnLabels.map(label => [label, true])));
  assert.deepEqual(await detailSelectionState(page), state);
  await button('Layout').click();
  await expect(page.getByRole('menuitemcheckbox', { name: 'Sort by packed size', exact: true })).toBeChecked();
  await page.keyboard.press('Escape');
  assert.deepEqual(await downloadArchive(page, button('Download TAR')), snapshot);
  await button('Extract…').click();
  await expect(page.getByRole('radio', { name: 'Selected entries', exact: true })).toBeChecked();
  const folder = await finishExtraction(page, button);
  assert.deepEqual(await tree(), { ...sentinelTree, [folder]: {
    docs: { empty: {}, nested: { 'data.bin': [0, 127, 255] } }, 'other.txt': Array.from(Buffer.from('not selected\n')),
  } });
});

test('Column visibility narrow rules preserve preferences and remembered widths across viewport changes', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 1024, height: 600 });
  await choose(); const widths = await changeEveryDetailColumn(page);
  await page.setViewportSize({ width: 360, height: 600 });
  await visibleDetailColumns(page, ['Size (bytes)'], false);
  await button('Layout').click();
  await page.getByRole('menuitemcheckbox', { name: 'Sort by packed size', exact: true }).click();
  await visibleDetailColumns(page, ['Packed (bytes)'], false);
  await toggleDetailColumn(page, 'Show packed column', false);
  await visibleDetailColumns(page, ['Size (bytes)'], false);
  await toggleDetailColumn(page, 'Show size column', false);
  await visibleDetailColumns(page, [], false);
  await toggleDetailColumn(page, 'Show packed column', true);
  await button('Layout').click();
  await page.getByRole('menuitemcheckbox', { name: 'Sort by name', exact: true }).click();
  await visibleDetailColumns(page, ['Packed (bytes)'], false);
  await toggleDetailColumn(page, 'Show size column', true);
  await visibleDetailColumns(page, ['Size (bytes)'], false);
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.setViewportSize({ width: 1024, height: 600 });
  await expectDetailWidths(page, widths); await expectDetailAlignment(page);
  await expectColumnPreferences(page, Object.fromEntries(showColumnLabels.map(label => [label, true])));
});

test('Column visibility rejects stale and forced menu events while modal or worker busy', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 1024, height: 600 }); await choose();
  const widths = await changeEveryDetailColumn(page);
  await button('Layout').click();
  const stale = await page.getByRole('menuitemcheckbox', { name: 'Show size column', exact: true }).elementHandle();
  await page.keyboard.press('Escape'); await button('Create archive…').click();
  const modal = page.getByRole('group', { name: 'Create archive', exact: true }); await expect(modal).toBeVisible();
  await stale.evaluate(node => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  await button('Layout').dispatchEvent('click'); await expect(page.getByRole('menu')).toHaveCount(0);
  await expectDetailWidths(page, widths); await visibleDetailColumns(page, detailColumnNames.slice(1));
  await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
  let release, intercepted = false; const gate = new Promise(resolve => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async route => { intercepted = true; await gate; await route.continue().catch(() => {}); });
  try {
    await button('Test archive').click(); await expect.poll(() => intercepted).toBe(true); await button('Layout').click();
    for (const label of showColumnLabels) {
      const item = page.getByRole('menuitemcheckbox', { name: label, exact: true });
      await expect(item).toBeDisabled(); await item.dispatchEvent('click'); await expect(item).toBeChecked();
    }
    await stale.evaluate(node => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    await expectDetailWidths(page, widths); await page.keyboard.press('Escape'); await button('Cancel').click();
    await expect(page.getByText('Archive test cancelled.', { exact: true })).toBeVisible();
  } finally { release(); await page.unrouteAll({ behavior: 'wait' }); await stale.dispose(); }
  await expectColumnPreferences(page, Object.fromEntries(showColumnLabels.map(label => [label, true])));
  await expectDetailWidths(page, widths);
});

test('Column visibility Layout ten commands are reachable and dismiss correctly at 360x600', { timeout: 120_000 }, async (t) => {
  const { page, button, choose } = await navigationPage(t, { width: 360, height: 600 }); await choose();
  const labels = ['All entries', 'Auto adjust columns', 'Sort by name', 'Sort by type', 'Sort by size',
    'Sort by packed size', 'Smart file name sorting', ...showColumnLabels];
  for (const label of labels) {
    await button('Layout').click(); const menu = page.getByRole('menu'); await expect(menu).toHaveCount(1);
    await expect(menu.locator('[role="menuitem"],[role="menuitemcheckbox"]')).toHaveCount(10);
    const bounds = await menu.boundingBox();
    assert.ok(bounds.x >= -1 && bounds.y >= -1 && bounds.x + bounds.width <= 361 && bounds.y + bounds.height <= 601,
      `360x600 menu must fit: ${JSON.stringify(bounds)}`);
    const item = menu.getByRole(label === 'Auto adjust columns' ? 'menuitem' : 'menuitemcheckbox', { name: label, exact: true });
    await item.scrollIntoViewIfNeeded(); await expect(item).toBeInViewport({ ratio: 1 });
    if (label === 'Show packed column') {
      await saveVisibilityScreenshot(page, 'columns-visibility-menu-360x600', 'ten menu commands including Smart file name sorting; last item fully visible');
    }
    await item.click(); await expect(menu).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
});

test('Expanded filesystem budget rejects Full paths before output but permits Files only retry of the same 257-file archive', { timeout: 240_000 }, async (t) => {
  const count = 257;
  const entries = Array.from({ length: count }, (_, index) => {
    const id = String(index).padStart(3, '0');
    return [[`d${id}`, ...Array(63).fill('a'), `file-${id}.txt`].join('/'), ''];
  });
  const physical = new Set(entries.flatMap(([path]) => {
    const parts = path.split('/');
    return parts.map((_, index) => parts.slice(0, index + 1).join('/'));
  }));
  const basenames = entries.map(([path]) => path.split('/').at(-1));
  assert.equal(new Set(basenames).size, count);
  assert.equal(physical.size, 16_705);
  assert.ok(entries.every(([path]) => Buffer.byteLength(path) < 1024));
  assert.ok(count < 4096 && physical.size > 16_384);
  const archive = await extractionFixture(entries);
  const { page, button, tree, downloads } = await extractionPage(t, true, { viewport: { width: 1024, height: 600 } });
  const picker = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await picker).setFiles(archive);
  await expect(page.getByText(`${count} entries`, { exact: true })).toBeVisible();
  await expect(button('Open archive…')).toBeEnabled();
  await expect.poll(() => page.evaluate(() => window.listWorkers)).toEqual([{ terminated: true, codes: [1] }]);
  assert.ok(await page.getByRole('checkbox').count() > 0);
  assert.ok(await page.getByRole('checkbox').count() < count, 'assert metadata count, not all virtualized rows in the DOM');
  const selected = page.getByRole('checkbox', { name: entries[0][0], exact: true });
  await selected.click();
  await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
  const baseline = await tree();
  const snapshot = await downloadArchive(page, button('Download ZIP'));
  assert.deepEqual(snapshot.bytes, archive.buffer);
  const downloadsBefore = downloads.length;
  await page.evaluate(() => {
    window.budgetOutputCreates = [];
    const prototype = FileSystemDirectoryHandle.prototype;
    for (const [method, kind] of [['getDirectoryHandle', 'directory'], ['getFileHandle', 'file']]) {
      const original = prototype[method];
      prototype[method] = function (name, options) {
        // The picker reopens the already-owned parent; count all actual output creation requests.
        if (options?.create && name !== window.testDirectoryName) window.budgetOutputCreates.push({ kind, name });
        return original.call(this, name, options);
      };
    }
  });
  await button('Test archive').click();
  await expect(page.getByText(`Tested ${count} entries (0 bytes). Content checksums: ${count}/${count} files.`, { exact: true })).toBeVisible();
  await expect(button('Test archive')).toBeEnabled();
  await expect(selected).toBeChecked();
  assert.deepEqual(await tree(), baseline);
  assert.deepEqual(await page.evaluate(() => window.budgetOutputCreates), [], 'listing and testing must not create output');

  await button('Extract…').click();
  const modal = page.getByRole('group', { name: 'Extract archive', exact: true });
  await modal.getByRole('radio', { name: 'All entries', exact: true }).click();
  await modal.getByRole('radio', { name: 'Full paths', exact: true }).click();
  await modal.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(page.getByText(/^Could not extract archive:.*expanded filesystem items/)).toBeVisible();
  await expect(button('Extract…')).toBeEnabled();
  await expect(selected).toBeChecked();
  await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
  assert.deepEqual(await page.evaluate(() => window.budgetOutputCreates), [], 'Full paths must fail before any output directory or file is requested');
  assert.deepEqual(await tree(), baseline);
  await expect.poll(() => page.evaluate(() => window.extractionWorkers)).toEqual([{ terminated: true, codes: [0] }]);
  assert.equal(downloads.length, downloadsBefore, 'rejection must not download or publish output');
  assert.deepEqual(await downloadArchive(page, button('Download ZIP')), snapshot, 'rejection preserves the original archive bytes');

  await button('Extract…').click();
  await modal.getByRole('radio', { name: 'All entries', exact: true }).click();
  await modal.getByRole('radio', { name: 'Files only (no folders)', exact: true }).click();
  await modal.getByRole('button', { name: 'Continue', exact: true }).click();
  const report = page.getByLabel(new RegExp(`^Extracted ${count} entries to PeaZip-[0-9a-f-]{36}$`));
  await expect(report).toBeVisible({ timeout: 180_000 });
  await expect(button('Extract…')).toBeEnabled();
  const folder = (await report.getAttribute('aria-label')).split(' to ')[1];
  const actual = await tree();
  assert.deepEqual(Object.keys(actual).sort(), [...Object.keys(baseline), folder].sort());
  for (const [name, bytes] of Object.entries(baseline)) assert.deepEqual(actual[name], bytes);
  assert.equal(Object.keys(actual[folder]).length, count);
  assert.deepEqual(Object.keys(actual[folder]).sort(), [...basenames].sort());
  assert.ok(Object.values(actual[folder]).every(bytes => Array.isArray(bytes) && bytes.length === 0),
    'every flattened descendant must be an actual empty file, never a directory');
  const creates = await page.evaluate(() => window.budgetOutputCreates);
  assert.deepEqual(creates.filter(item => item.kind === 'directory'), [{ kind: 'directory', name: folder }]);
  assert.equal(creates.filter(item => item.kind === 'file').length, count);
  await expect(selected).toBeChecked();
  await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.extractionWorkers)).toEqual([
    { terminated: true, codes: [0] }, { terminated: true, codes: [1] },
  ]);
  assert.deepEqual(await downloadArchive(page, button('Download ZIP')), snapshot);
  t.diagnostic(`Actual ZIP: ${count} empty files, ${physical.size} expanded nodes; Full paths made zero output calls; Files only wrote ${count} unique files through real OPFS.`);
});

const sortHotkeys = [
  ['0', 'Sort by name'], ['1', 'Sort by type'], ['2', 'Sort by size'], ['3', 'Sort by packed size'],
];
async function pressSortHotkey(page, digit) {
  const mac = await page.evaluate(() => /Mac|iPhone|iPad/.test(navigator.platform));
  await page.keyboard.press(`${mac ? 'Meta' : 'Control'}+Alt+${digit}`);
}
async function openSortFixture(page, button) {
  const { stdout } = await promisify(execFile)('uv', ['run', 'python', '-c', `
import base64, io, json, zipfile
out = io.BytesIO()
with zipfile.ZipFile(out, 'w') as archive:
    for name, data, method in [('docs/', b'', 0), ('docs/a.txt', b'A'*2000, 8), ('docs/z.bin', bytes(range(128)), 0), ('docs/plain', b'data', 0), ('outside.txt', b'', 0)]:
        archive.writestr(name, data, compress_type=method)
with zipfile.ZipFile(out) as archive:
    metadata = {i.filename: {'size': i.file_size, 'packed': i.compress_size, 'directory': i.is_dir()} for i in archive.infolist()}
print(json.dumps({'bytes': base64.b64encode(out.getvalue()).decode(), 'metadata': metadata}))
`]);
  const value = JSON.parse(stdout);
  const picker = page.waitForEvent('filechooser');
  await button('Open archive…').click();
  await (await picker).setFiles({ name: 'sort-hotkeys.zip', mimeType: 'application/zip', buffer: Buffer.from(value.bytes, 'base64') });
  await expect(page.getByText('5 entries', { exact: true })).toBeVisible();
  await expect(button('Open archive…')).toBeEnabled();
  return value;
}

for (const width of [1024, 360]) {
  test(`Sort hotkeys 0–3 toggle real ordering while preserving hidden columns, partial selection and bytes at ${width}px`, { timeout: 120_000 }, async (t) => {
    const { page, button, tree } = await extractionPage(t, true, { viewport: { width, height: 600 } });
    const fixture = await openSortFixture(page, button);
    await page.getByRole('checkbox', { name: 'docs/', exact: true }).click();
    await page.getByRole('checkbox', { name: 'docs/z.bin', exact: true }).click();
    await page.getByRole('checkbox', { name: 'outside.txt', exact: true }).click();
    const filter = page.getByRole('textbox', { name: 'Filter names', exact: true });
    await filter.fill('docs'); await filter.press('Enter');
    await expect(page.getByRole('checkbox', { name: 'docs/', exact: true })).toHaveAttribute('aria-checked', 'mixed');
    const selected = await page.getByText(/^\d+ selected$/, { exact: true }).innerText();
    const snapshot = await downloadArchive(page, button('Download ZIP'));
    assert.deepEqual(snapshot.bytes, Buffer.from(fixture.bytes, 'base64'));
    for (const label of showColumnLabels) await toggleDetailColumn(page, label, false);
    const hidden = Object.fromEntries(showColumnLabels.map(label => [label, false]));
    await button('Layout').click();
    await page.getByRole('menuitemcheckbox', { name: 'Sort by type', exact: true }).click();
    const paths = () => page.getByRole('checkbox').evaluateAll(nodes => nodes.map(node => node.getAttribute('aria-label')));
    const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
    for (const [digit, label] of sortHotkeys) {
      for (const descending of [false, true]) {
        await button('Name').focus();
        await pressSortHotkey(page, digit);
        const expected = Object.keys(fixture.metadata).filter(name => name.includes('docs')).sort((a, b) => {
          const left = fixture.metadata[a], right = fixture.metadata[b];
          if (left.directory !== right.directory) return left.directory ? -1 : 1;
          const kind = path => path.endsWith('/') ? 'Folder' : path.split('/').at(-1).includes('.') ? path.split('.').at(-1).toUpperCase() : 'File';
          const primary = digit === '0' ? compare(a, b) : digit === '1' ? compare(kind(a), kind(b))
            : (digit === '2' ? left.size - right.size : left.packed - right.packed);
          return (descending ? -primary : primary) || compare(a, b);
        });
        await expect.poll(paths).toEqual(expected);
        await expect(filter).toHaveValue('docs');
        await expect(page.getByText(selected, { exact: true })).toBeVisible();
        await expect(page.getByRole('checkbox', { name: 'docs/', exact: true })).toHaveAttribute('aria-checked', 'mixed');
        await expect(page.getByRole('checkbox', { name: 'docs/z.bin', exact: true })).not.toBeChecked();
        for (const name of detailColumnNames.slice(1)) await expect(button(name)).toHaveCount(0);
        await button('Layout').click();
        const item = page.getByRole('menuitemcheckbox', { name: label, exact: true });
        await expect(item).toBeChecked();
        assert.match(await item.innerText(), new RegExp(digit), 'menu must show its numeric shortcut hint');
        for (const [preference, enabled] of Object.entries(hidden)) await expect(page.getByRole('menuitemcheckbox', { name: preference, exact: true })).not.toBeChecked();
        await page.keyboard.press('Escape');
      }
    }
    assert.deepEqual(await downloadArchive(page, button('Download ZIP')), snapshot);
    await button('Clear filter').click();
    await expect(page.getByRole('checkbox', { name: 'outside.txt', exact: true })).toBeChecked();
    if (width === 360) {
      await toggleDetailColumn(page, 'Show packed column', true);
      await toggleDetailColumn(page, 'Show size column', true);
      await button('Name').focus(); await pressSortHotkey(page, '2');
      await expect(button('Size (bytes)')).toBeVisible(); await expect(button('Packed (bytes)')).toHaveCount(0);
      await button('Name').focus(); await pressSortHotkey(page, '3');
      await expect(button('Packed (bytes)')).toBeVisible(); await expect(button('Size (bytes)')).toHaveCount(0);
      await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
    assert.deepEqual(await tree(), sentinelTree);
  });
}

test('Sort hotkeys do not steal editing, open-menu, modal or busy input', { timeout: 120_000 }, async (t) => {
  const { page, button, tree } = await extractionPage(t, true, { viewport: { width: 1024, height: 600 } });
  await openSortFixture(page, button);
  const paths = () => page.getByRole('checkbox').evaluateAll(nodes => nodes.map(node => node.getAttribute('aria-label')));
  for (const label of ['Filter names', 'Archive path']) {
    const field = page.getByRole('textbox', { name: label, exact: true });
    if (label === 'Filter names') await field.fill('docs');
    const before = await paths(), value = await field.inputValue();
    await field.focus();
    for (const [digit] of sortHotkeys) {
      await pressSortHotkey(page, digit);
      await expect(field).toBeFocused(); await expect(field).toHaveValue(value);
      assert.deepEqual(await paths(), before);
    }
  }
  await button('Clear filter').click();
  const before = await paths();
  await button('Layout').click();
  for (const [digit] of sortHotkeys) {
    await pressSortHotkey(page, digit);
    await expect(page.getByRole('menu')).toHaveCount(1);
    await expect(page.getByRole('menuitemcheckbox', { name: 'Sort by name', exact: true })).toBeChecked();
    assert.deepEqual(await paths(), before);
  }
  await page.keyboard.press('Escape');
  await button('Create archive…').click();
  const modal = page.getByRole('group', { name: 'Create archive', exact: true }); await expect(modal).toBeVisible();
  for (const [digit] of sortHotkeys) {
    await pressSortHotkey(page, digit);
    await expect(modal).toBeVisible(); assert.deepEqual(await paths(), before);
  }
  await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
  let release, intercepted = false; const gate = new Promise(resolve => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async route => { intercepted = true; await gate; await route.continue().catch(() => {}); });
  try {
    await button('Test archive').click(); await expect.poll(() => intercepted).toBe(true);
    for (const [digit] of sortHotkeys) { await button('Name').focus(); await pressSortHotkey(page, digit); assert.deepEqual(await paths(), before); }
    await button('Layout').click();
    for (const [, label] of sortHotkeys) await expect(page.getByRole('menuitemcheckbox', { name: label, exact: true })).toBeDisabled();
    await page.keyboard.press('Escape'); await button('Cancel').click();
    await expect(page.getByText('Archive test cancelled.', { exact: true })).toBeVisible();
  } finally { release(); await page.unrouteAll({ behavior: 'wait' }); }
  await button('Name').focus(); await pressSortHotkey(page, '2');
  await expect.poll(paths).toEqual(['docs/', 'outside.txt', 'docs/plain', 'docs/z.bin', 'docs/a.txt']);
  assert.deepEqual(await tree(), sentinelTree);
});

async function setSmartNameSorting(page, enabled) {
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  const item = page.getByRole('menuitemcheckbox', { name: 'Smart file name sorting', exact: true });
  await expect(item).toBeEnabled();
  if ((await item.getAttribute('aria-checked') === 'true') !== enabled) await item.click();
  else await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
}
async function expectSmartNameSorting(page, enabled) {
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  const item = page.getByRole('menuitemcheckbox', { name: 'Smart file name sorting', exact: true });
  if (enabled) await expect(item).toBeChecked(); else await expect(item).not.toBeChecked();
  await page.keyboard.press('Escape');
}
async function openSmartSortFixture(page, button) {
  const entries = [['folder2/', ''], ['folder10/', ''], ['file2.txt', 'two'], ['file10.txt', 'ten'],
    ['report.txt2', 'two'], ['report.txt10', 'ten'], ['foo2bar10.txt', 'one'], ['foo2bar3.txt', 'tri'],
    ['zero2.txt', 'two'], ['zero02.txt', 'zer'], ['zero10.txt', 'ten']];
  const file = await extractionFixture(entries);
  const picker = page.waitForEvent('filechooser'); await button('Open archive…').click();
  await (await picker).setFiles(file);
  await expect(page.getByText(`${entries.length} entries`, { exact: true })).toBeVisible();
  await expect(button('Open archive…')).toBeEnabled();
  return { file, entries };
}
async function assertSmartSortPairs(page, smart, descending = false) {
  await expect.poll(async () => {
    const names = await page.getByRole('checkbox').evaluateAll(nodes => nodes.sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top).map(node => node.getAttribute('aria-label')));
    const pairs = [smart ? ['file2.txt', 'file10.txt'] : ['file10.txt', 'file2.txt'],
      smart ? ['report.txt2', 'report.txt10'] : ['report.txt10', 'report.txt2'],
      smart ? ['folder2/', 'folder10/'] : ['folder10/', 'folder2/'],
      ['foo2bar10.txt', 'foo2bar3.txt'], ['zero02.txt', 'zero10.txt']];
    return pairs.every(([a, b]) => names.includes(a) && names.includes(b)
      && (descending ? names.indexOf(a) > names.indexOf(b) : names.indexOf(a) < names.indexOf(b)));
  }).toBe(true);
}

test('Smart file name sorting uses first numeric stem/extension runs, preserves direction and exports selected bytes', { timeout: 120_000 }, async (t) => {
  const { page, button, tree } = await extractionPage(t, true, { viewport: { width: 1024, height: 900 } });
  const fixture = await openSmartSortFixture(page, button);
  await expectSmartNameSorting(page, true);
  await assertSmartSortPairs(page, true);
  await saveVisibilityScreenshot(page, 'smart-sort-numeric-on-1024', { 'Smart file name sorting': true });
  const snapshot = await downloadArchive(page, button('Download ZIP'));
  await page.getByRole('checkbox', { name: 'file2.txt', exact: true }).click();
  await page.getByRole('checkbox', { name: 'zero02.txt', exact: true }).click();
  for (const label of showColumnLabels) await toggleDetailColumn(page, label, false);
  const preferences = Object.fromEntries(showColumnLabels.map(label => [label, false]));
  await setSmartNameSorting(page, false); await assertSmartSortPairs(page, false);
  await saveVisibilityScreenshot(page, 'smart-sort-numeric-off-1024', { 'Smart file name sorting': false });
  await button('Name').click(); await assertSmartSortPairs(page, false, true);
  await setSmartNameSorting(page, true); await assertSmartSortPairs(page, true, true);
  await button('Name').click(); await assertSmartSortPairs(page, true);
  const ascending = await page.getByRole('checkbox').evaluateAll(nodes => nodes.sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top).map(node => node.getAttribute('aria-label')));
  await setSmartNameSorting(page, false); await setSmartNameSorting(page, true);
  assert.deepEqual(await page.getByRole('checkbox').evaluateAll(nodes => nodes.sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top).map(node => node.getAttribute('aria-label'))), ascending,
    'leading-zero ties and first-run suffixes must remain deterministic after toggling back');
  const filter = page.getByRole('textbox', { name: 'Filter names', exact: true });
  await filter.fill('file'); await filter.press('Enter');
  const state = await detailSelectionState(page);
  await setSmartNameSorting(page, false); await setSmartNameSorting(page, true);
  assert.deepEqual(await detailSelectionState(page), state);
  await expect(page.getByText('2 selected', { exact: true })).toBeVisible();
  await expectColumnPreferences(page, preferences);
  assert.deepEqual(await downloadArchive(page, button('Download ZIP')), snapshot);
  await button('Extract…').click();
  await expect(page.getByRole('radio', { name: 'Selected entries', exact: true })).toBeChecked();
  const folder = await finishExtraction(page, button);
  assert.deepEqual(await tree(), { ...sentinelTree, [folder]: { 'file2.txt': Array.from(Buffer.from('two')), 'zero02.txt': Array.from(Buffer.from('zer')) } });
  await button('Clear filter').click();
  for (const sort of ['Sort by type', 'Sort by size', 'Sort by packed size']) {
    await button('Layout').click(); await page.getByRole('menuitemcheckbox', { name: sort, exact: true }).click();
    await setSmartNameSorting(page, false);
    await button('Layout').click();
    await expect(page.getByRole('menuitemcheckbox', { name: sort, exact: true })).toBeChecked();
    await page.keyboard.press('Escape');
    let names = await page.getByRole('checkbox').evaluateAll(nodes => nodes.sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top).map(node => node.getAttribute('aria-label')));
    assert.ok(names.indexOf('file10.txt') < names.indexOf('file2.txt'), `${sort} ascending lexical tie: ${JSON.stringify(names)}`);
    const rawTieOrder = names;
    await setSmartNameSorting(page, true);
    names = await page.getByRole('checkbox').evaluateAll(nodes => nodes.sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top).map(node => node.getAttribute('aria-label')));
    if (sort === 'Sort by size') {
      assert.deepEqual(names, rawTieOrder, 'Size equal values keep the original raw-name/path tie order under smart toggle');
    } else {
      assert.ok(names.indexOf('file2.txt') < names.indexOf('file10.txt'), `${sort} ascending smart tie`);
    }
    await button('Layout').click(); await page.getByRole('menuitemcheckbox', { name: sort, exact: true }).click();
    const before = await page.getByRole('checkbox').evaluateAll(nodes => nodes.sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top).map(node => node.getAttribute('aria-label')));
    await setSmartNameSorting(page, false); await setSmartNameSorting(page, true);
    assert.deepEqual(await page.getByRole('checkbox').evaluateAll(nodes => nodes.sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top).map(node => node.getAttribute('aria-label'))), before,
      `${sort} direction must survive smart toggle`);
  }
  await expect(page.getByRole('checkbox', { name: 'file2.txt', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'zero02.txt', exact: true })).toBeChecked();
  assert.deepEqual(await downloadArchive(page, button('Download ZIP')), snapshot);
});

test('Smart file name sorting session preference survives reopen, folder/flat view and narrow resizing', { timeout: 120_000 }, async (t) => {
  const { page, button } = await extractionPage(t, true, { viewport: { width: 1024, height: 900 } });
  const { file, entries } = await openSmartSortFixture(page, button);
  await setSmartNameSorting(page, false); await assertSmartSortPairs(page, false);
  await button('Folder view').click(); await expectSmartNameSorting(page, false);
  await page.setViewportSize({ width: 360, height: 900 }); await expectSmartNameSorting(page, false);
  await button('All entries').click(); await assertSmartSortPairs(page, false);
  const picker = page.waitForEvent('filechooser'); await button('Open archive…').click(); await (await picker).setFiles(file);
  await expect(page.getByText(`${entries.length} entries`, { exact: true })).toBeVisible();
  await expect(button('Open archive…')).toBeEnabled();
  await expectSmartNameSorting(page, false); await assertSmartSortPairs(page, false);
  await setSmartNameSorting(page, true); await assertSmartSortPairs(page, true);
  await page.setViewportSize({ width: 1440, height: 900 }); await expectSmartNameSorting(page, true);
  await assertSmartSortPairs(page, true);
});

test('Smart file name sorting rejects modal and busy stale menu events', { timeout: 120_000 }, async (t) => {
  const { page, button } = await extractionPage(t, true, { viewport: { width: 1024, height: 900 } });
  await openSmartSortFixture(page, button);
  await button('Layout').click();
  const stale = await page.getByRole('menuitemcheckbox', { name: 'Smart file name sorting', exact: true }).elementHandle();
  await page.keyboard.press('Escape'); await button('Create archive…').click();
  const modal = page.getByRole('group', { name: 'Create archive', exact: true }); await expect(modal).toBeVisible();
  await stale.evaluate(node => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  await button('Layout').dispatchEvent('click'); await expect(page.getByRole('menu')).toHaveCount(0);
  await assertSmartSortPairs(page, true); await modal.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expectSmartNameSorting(page, true);
  let release, intercepted = false; const gate = new Promise(resolve => { release = resolve; });
  await page.route('**/worker-core_bg.wasm', async route => { intercepted = true; await gate; await route.continue().catch(() => {}); });
  try {
    await button('Test archive').click(); await expect.poll(() => intercepted).toBe(true); await button('Layout').click();
    const item = page.getByRole('menuitemcheckbox', { name: 'Smart file name sorting', exact: true });
    await expect(item).toBeDisabled(); await item.dispatchEvent('click'); await expect(item).toBeChecked();
    await stale.evaluate(node => node.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    await assertSmartSortPairs(page, true); await page.keyboard.press('Escape'); await button('Cancel').click();
    await expect(page.getByText('Archive test cancelled.', { exact: true })).toBeVisible();
  } finally { release(); await page.unrouteAll({ behavior: 'wait' }); await stale.dispose(); }
  await expectSmartNameSorting(page, true); await setSmartNameSorting(page, false); await assertSmartSortPairs(page, false);
});
