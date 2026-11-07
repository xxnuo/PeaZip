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
  await expect(page.getByRole('button', { name: 'Extract selected…', exact: true })).toBeDisabled();

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
  await choose('Create ZIP…', [file]);
  await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: file.name, exact: true }).click();
  const original = await downloadArchive(page, button('Download ZIP'));
  let pickers = 0;
  page.on('filechooser', () => pickers++);
  for (const cancel of ['Cancel', 'Escape']) {
    await button('Create ZIP…').click();
    await expect(page.getByText('Create archive', { exact: true })).toBeVisible();
    if (cancel === 'Escape') await page.keyboard.press('Escape');
    else await page.getByRole('group', { name: 'Create archive', exact: true }).getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.getByText('Create archive', { exact: true })).toHaveCount(0);
    await expect(page.getByRole('checkbox', { name: file.name, exact: true })).toBeChecked();
    assert.deepEqual(await downloadArchive(page, button('Download ZIP')), original);
  }
  assert.equal(pickers, 0);
  for (const [choice, method, level] of [['Store (no compression)', 0, null], ['Deflate', 8, '1'], ['Deflate', 8, '9']]) {
    await button('Create ZIP…').click();
    await page.getByRole('radio', { name: choice, exact: true }).click();
    if (level) await page.getByRole('textbox', { name: 'Compression level', exact: true }).fill(level);
    const pending = page.waitForEvent('filechooser');
    await button('Continue').click();
    await (await pending).setFiles([file]);
    await expect(page.getByText('Created ZIP: 1 entries', { exact: true })).toBeVisible();
    await expect(button('Create ZIP…')).toBeEnabled();
    checkSingleZip(await downloadArchive(page, button('Download ZIP')), method, body);
  }
  await page.getByRole('checkbox', { name: file.name, exact: true }).click();
  const beforeFailure = await downloadArchive(page, button('Download ZIP'));
  await button('Create ZIP…').click();
  const pending = page.waitForEvent('filechooser');
  await button('Continue').click();
  await (await pending).setFiles([{ name: 'high-ratio.txt', mimeType: 'text/plain', buffer: Buffer.alloc(1024 * 1024, 65) }]);
  await expect(page.getByText(/Could not create ZIP: .*compression ratio/)).toBeVisible();
  await expect(page.getByRole('checkbox', { name: file.name, exact: true })).toBeChecked();
  assert.deepEqual(await downloadArchive(page, button('Download ZIP')), beforeFailure);
  await button('Create ZIP…').click();
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
  const create = page.getByRole('button', { name: 'Create ZIP…', exact: true });
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
      await expect(row.getByText(String(file.buffer.length), { exact: true })).toBeVisible();
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
    await button('Create ZIP…').click();
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
    await expect(button('Create ZIP…')).toBeEnabled();
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
    if (name === 'Create ZIP…') await button('Continue').click();
    await (await pending).setFiles(files);
  }
  await expect(button('Create ZIP…')).toBeEnabled();
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
  await choose('Create ZIP…', [{ name, mimeType: 'text/plain', buffer: Buffer.from('long filename payload') }]);
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
  await choose('Create ZIP…', [{ name: 'original.txt', mimeType: 'text/plain', buffer: Buffer.from('original snapshot') }]);
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
    await choose('Create ZIP…', [{ name: 'cancelled.txt', mimeType: 'text/plain', buffer: Buffer.from('cancel me') }]);
    await expect.poll(() => intercepted).toBe(true);
    await expect(cancel).toBeEnabled();
    for (const name of ['Create ZIP…', 'Open archive…', 'Download ZIP', 'Select all', 'Clear selection']) await expect(button(name)).toBeDisabled();
    await cancel.click();
    await expect(page.getByText('Creation cancelled.', { exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => window.archiveWorkers[1].terminated)).toBe(true);
    assert.deepEqual(await page.evaluate(() => window.archiveWorkers[1].messages), [], 'cancel reached WASM initialization before file reading');
    await expect(original).toBeChecked();
    await expect(cancel).toBeDisabled();
    await expect(button('Create ZIP…')).toBeEnabled();
    assert.equal(downloads.length, downloadsBefore, 'cancellation must not download');
    assert.deepEqual(await snapshot(), before, 'cancel must preserve exact original archive bytes');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
  }
  await choose('Create ZIP…', [{ name: 'retry.txt', mimeType: 'text/plain', buffer: Buffer.from('retry succeeds') }]);
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
  await choose('Create ZIP…', paths);
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
  await choose('Create ZIP…', paths);
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
  await expect(button('Create ZIP…')).toBeEnabled();
  assert.deepEqual(await snapshot(), before, 'CPU cancellation must preserve exact prior ZIP bytes');
  await expect(selected).toBeChecked();
  await choose('Create ZIP…', paths);
  await expect(page.getByText('Created ZIP: 3 entries', { exact: true })).toBeVisible({ timeout: 120_000 });
  await expect(selected).not.toBeChecked();
  assert.deepEqual(await page.evaluate(() => window.archiveWorkers[2].messages), messages, 'retry must finish actual compression');
  assert.equal(downloads.length, downloadsBefore + 1, 'retry must not automatically download');
  t.diagnostic(`Real CPU cancellation: ${JSON.stringify(cancelled)}; worker terminated without success; prior ZIP bytes/selection preserved; same 24 MiB retry succeeded.`);

  const count = 251;
  await choose('Create ZIP…', Array.from({ length: count }, (_, index) => ({
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
  await expect(button('Extract selected…')).toBeDisabled();
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
  for (const name of ['Back', 'Forward', 'Up', 'Download ZIP', 'Test archive', 'Extract selected…']) await expect(button(name)).toBeDisabled();
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
    for (const [item, action] of [['Open archive…', 'Open archive…'], ['Create ZIP…', 'Create ZIP…'], ['Download archive', 'Download ZIP']]) {
      await expect(popup.getByRole('menuitem', { name: item, exact: true })).toBeDisabled();
      await expect(button(action)).toBeDisabled();
    }
    await expect(popup.getByRole('menuitem', { name: 'Close archive', exact: true })).toBeDisabled();
    await button('Tools').hover();
    await expect(popup).toHaveCount(1);
    for (const name of ['Test archive', 'Extract selected…']) {
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
      await expect(popup.getByRole('menuitem', { name: 'Create folder ZIP…', exact: true })).toHaveCount(0);
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
    await button('Extract selected…').click();
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
      for (const name of ['Test archive', 'Open archive…', 'Create ZIP…', 'Download ZIP', 'Extract selected…', 'Select all', 'Clear selection']) await expect(button(name)).toBeDisabled();
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
  await expect(chinese.getByText('10', { exact: true })).toBeVisible();
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
  await button('Extract selected…').click();
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
  await button('Extract selected…').click();
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
  await button('Extract selected…').click();
  const report = page.getByText(/^Extracted 4 entries to PeaZip-/);
  await expect(report).toBeVisible();
  const folder = (await report.innerText()).split(' to ')[1];
  assert.deepEqual(await tree(), { ...sentinelTree, [folder]: {
    docs: { '你好.txt': Array.from(Buffer.from('tar 你好\n')), empty: {}, nested: { 'data.bin': [0, 127, 255] } },
  } });
  assert.equal(downloads.length, 1);
});

test('Web extraction writes selected bytes and empty directories through real OPFS and Rust worker', { timeout: 120_000 }, async (t) => {
  const { page, button, choose, tree, downloads } = await extractionPage(t);
  await choose();
  await page.getByRole('checkbox', { name: '你好.txt', exact: true }).click();
  await page.getByRole('checkbox', { name: '空目录/', exact: true }).click();
  await button('Extract selected…').click();
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
    await button('Extract selected…').click();
    await expect(page.getByText(failure === 'AbortError' ? 'Extraction cancelled.' : /^Could not extract archive:.*Test picker rejection/)).toBeVisible();
    await expect(button('Extract selected…')).toBeEnabled();
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
  await button('Extract selected…').click();
  await expect(page.getByText(/^Could not extract archive:/)).toBeVisible();
  await expect(button('Extract selected…')).toBeEnabled();
  await expect(selected).toBeChecked();
  assert.deepEqual(await tree(), sentinelTree, 'CRC validation must finish before any output directory is created');
  await expect.poll(() => page.evaluate(() => window.extractionWorkers)).toEqual([{ terminated: true, codes: [0] }]);
  await choose();
  await selected.click();
  await button('Extract selected…').click();
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
  await expect(button('Extract selected…')).toBeDisabled();
  await expect(page.getByText('Directory export is unavailable in this browser.', { exact: true })).toBeVisible();
  assert.deepEqual(await page.evaluate(() => window.pickerCalls), []);
  assert.deepEqual(await page.evaluate(() => window.extractionWorkers), []);
  assert.deepEqual(await tree(), sentinelTree);
});

test('Cancel extraction during real worker WASM loading preserves archive snapshot and selection', { timeout: 120_000 }, async (t) => {
  const { page, button, tree, downloads } = await extractionPage(t);
  // A created archive also gives us a downloadable snapshot to compare byte for byte.
  const pending = page.waitForEvent('filechooser');
  await button('Create ZIP…').click();
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
    await button('Extract selected…').click();
    await expect.poll(() => intercepted).toBe(true);
    await expect(button('Cancel')).toBeEnabled();
    for (const name of ['Create ZIP…', 'Open archive…', 'Download ZIP', 'Select all', 'Clear selection', 'Extract selected…']) await expect(button(name)).toBeDisabled();
    await button('Cancel').click();
    await expect(page.getByText('Extraction cancelled.', { exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => window.extractionWorkers[1])).toEqual({ terminated: true, codes: [] });
    await expect.poll(() => page.workers().length).toBe(0);
    await expect(selected).toBeChecked();
    await expect(page.getByText('1 selected', { exact: true })).toBeVisible();
    await expect(button('Cancel')).toBeDisabled();
    await expect(button('Extract selected…')).toBeEnabled();
    assert.deepEqual(await tree(), sentinelTree);
    assert.equal(downloads.length, downloadsBefore);
    assert.deepEqual(await snapshot(), before, 'cancel must preserve exact archive bytes');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
  }
  await expect(selected).toBeChecked();
  await button('Extract selected…').click();
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
  await button('Extract selected…').click();
  await expect.poll(() => page.evaluate(() => window.exportWriteStarted === true)).toBe(true);
  await button('Cancel').click();
  await expect(page.getByText(/^Could not extract archive: Cancelled:.*completed: 0.*possibly unfinished: 你好.txt.*output may remain/)).toBeVisible();
  assert.equal(await page.evaluate(() => window.exportWriteAborted), true);
  await expect(selected).toBeChecked();
  await expect(button('Extract selected…')).toBeEnabled();
  await expect(button('Cancel')).toBeDisabled();
  const result = await tree();
  const output = Object.keys(result).find((name) => name.startsWith('PeaZip-'));
  assert.ok(output);
  assert.deepEqual(result, { ...sentinelTree, [output]: { '你好.txt': [] } });
  assert.equal(downloads.length, 0);
  t.diagnostic('Write timing injected around real OPFS writable; abort and retained empty file verified, external directory picker not tested.');
});
