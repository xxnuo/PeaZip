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

async function creationPage(t) {
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage({ acceptDownloads: true });
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
  await page.goto(await serve(t));
  const button = (name) => page.getByRole('button', { name, exact: true });
  async function choose(name, files) {
    const pending = page.waitForEvent('filechooser');
    await button(name).click();
    await (await pending).setFiles(files);
  }
  await expect(button('Create ZIP…')).toBeEnabled();
  return { page, button, choose, downloads };
}

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
    for (const name of ['Create ZIP…', 'Open ZIP…', 'Download ZIP', 'Select all', 'Clear selection']) await expect(button(name)).toBeDisabled();
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
  await expect(page.getByText('ZIP download requested.', { exact: true })).toBeVisible();
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

async function extractionPage(t, available = true) {
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage();
  const errors = [], downloads = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('download', (download) => downloads.push(download));
  t.after(() => assert.deepEqual(errors, []));
  await page.addInitScript(({ available }) => {
    window.extractionWorkers = [];
    const NativeWorker = window.Worker;
    window.Worker = class extends NativeWorker {
      constructor(...args) {
        super(...args);
        const record = { terminated: false, codes: [] };
        window.extractionWorkers.push(record);
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
  await page.goto(await serve(t));
  const button = (name) => page.getByRole('button', { name, exact: true });
  await expect(button('Open ZIP…')).toBeEnabled();
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
    await button('Open ZIP…').click();
    await (await pending).setFiles(files);
    await expect(page.getByText('3 entries', { exact: true })).toBeVisible();
    await expect(button('Open ZIP…')).toBeEnabled();
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

const sentinelTree = { 'sentinel.txt': Array.from(Buffer.from('leave me unchanged')) };

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
    await expect(page.getByText(failure === 'AbortError' ? 'Extraction cancelled.' : /^Could not extract ZIP:.*Test picker rejection/)).toBeVisible();
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
  await expect(page.getByText(/^Could not extract ZIP:/)).toBeVisible();
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
    for (const name of ['Create ZIP…', 'Open ZIP…', 'Download ZIP', 'Select all', 'Clear selection', 'Extract selected…']) await expect(button(name)).toBeDisabled();
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
  await expect(page.getByText(/^Could not extract ZIP: Cancelled:.*completed: 0.*possibly unfinished: 你好.txt.*output may remain/)).toBeVisible();
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
