import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { access, readFile, mkdtemp, rm, writeFile, truncate } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
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
  page.on('worker', (worker) => workers.push({ worker, closed: once(worker, 'close') }));
  t.after(() => assert.deepEqual(errors, []));
  await page.goto(await serve(t));
  const create = page.getByRole('button', { name: 'Create ZIP…', exact: true });
  const open = page.getByRole('button', { name: 'Open ZIP…', exact: true });
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
  await expect(page.getByText(/^Could not create ZIP:.*duplicate or conflicting ZIP path:/)).toBeVisible();
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
