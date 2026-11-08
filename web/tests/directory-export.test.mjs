import test from 'node:test';
import assert from 'node:assert/strict';
import { exportEntries, parseEntries } from '../directory-export.js';

function packet(entries) {
  const chunks = entries.map(([kind, path, data = new Uint8Array()]) => {
    const name = typeof path === 'string' ? new TextEncoder().encode(path) : path;
    const result = new Uint8Array(12 + name.length + data.length);
    const view = new DataView(result.buffer);
    [kind, name.length, data.length].forEach((n, i) => view.setUint32(i * 4, n, true));
    result.set(name, 12);
    result.set(data, 12 + name.length);
    return result;
  });
  const result = new Uint8Array(4 + chunks.reduce((sum, item) => sum + item.length, 0));
  new DataView(result.buffer).setUint32(0, chunks.length, true);
  let offset = 4;
  for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.length; }
  return result;
}
const job = () => ({ controller: new AbortController(), outputName: null, phase: 'write' });
const namedError = (name) => Object.assign(new Error(name), { name });

test('cancellation requests abort while a write is still pending', { timeout: 5000 }, async () => {
  let began;
  let rejectWrite;
  const started = new Promise((resolve) => { began = resolve; });
  const pendingWrite = new Promise((_, reject) => { rejectWrite = reject; });
  const fs = fake(async (stage) => {
    if (stage === 'write') { began(); await pendingWrite; }
    if (stage === 'abort') rejectWrite(namedError('AbortError'));
  });
  const task = job();
  const result = exportEntries(fs.root, packet([[0, 'waiting', Uint8Array.of(1)]]), task, () => {});
  const failed = assert.rejects(result, /Cancelled:.*completed: 0.*possibly unfinished: waiting/);
  await started;
  task.controller.abort();
  await failed;
  assert.equal(fs.streams[0].aborted, true);
  assert.equal(fs.streams[0].closed, false);
});

// An injected model of calls and failures, not a real browser filesystem API.
function fake(hook = () => {}) {
  const calls = [];
  const streams = [];
  function directory(path = '') {
    const children = new Map();
    return {
      kind: 'directory', children,
      async getDirectoryHandle(name, options) {
        const full = `${path}/${name}`;
        calls.push(['directory', full, !!options?.create]);
        await hook('directory', full, options, children);
        if (!children.has(name)) {
          if (!options?.create) throw namedError('NotFoundError');
          children.set(name, directory(full));
        }
        const child = children.get(name);
        if (child.kind !== 'directory') throw namedError('TypeMismatchError');
        return child;
      },
      async getFileHandle(name, options) {
        const full = `${path}/${name}`;
        calls.push(['file', full, !!options?.create]);
        await hook('file', full, options, children);
        if (!children.has(name)) children.set(name, {
          kind: 'file', bytes: null,
          async createWritable() {
            await hook('writable', full);
            const file = this;
            let staged;
            const stream = {
              aborted: false, closed: false,
              async write(data) { await hook('write', full); staged = data.slice(); },
              async close() { await hook('close', full); file.bytes = staged; this.closed = true; },
              async abort() { await hook('abort', full); this.aborted = true; },
            };
            streams.push(stream);
            return stream;
          },
        });
        if (children.get(name).kind !== 'file') throw namedError('TypeMismatchError');
        return children.get(name);
      },
    };
  }
  return { root: directory(), calls, streams };
}

test('preserves hierarchy, empty directories and commits before progress', async () => {
  const fs = fake();
  const task = job();
  const progress = [];
  const result = await exportEntries(fs.root, packet([
    [0, '中文/a', Uint8Array.of(1, 2)], [1, '中文/'], [1, 'empty/'], [0, 'zero'],
  ]), task, (done, total) => {
    if (done === 1) assert.equal(fs.streams[0].closed, true);
    progress.push([done, total]);
  });
  assert.match(task.outputName, /^PeaZip-[0-9a-f-]{36}$/);
  assert.equal(result, `Extracted 4 entries to ${task.outputName}`);
  const output = fs.root.children.get(task.outputName);
  assert.deepEqual(output.children.get('中文').children.get('a').bytes, Uint8Array.of(1, 2));
  assert.equal(output.children.get('empty').children.size, 0);
  assert.deepEqual(progress, [[1, 4], [2, 4], [3, 4], [4, 4]]);
});

test('rejects entire invalid packet before any filesystem call', async () => {
  const badEntries = [
    [[0, '/abs']], [[0, '../x']], [[0, 'a/../x']], [[0, 'C:x']], [[0, 'a\\b']],
    [[0, 'a\0b']], [[0, 'a\u0085b']], [[0, 'a//b']], [[0, './x']], [[0, 'a/']],
    [[0, new Uint8Array([0xff])]], [[2, 'a']], [[1, 'dir']], [[1, 'dir/', Uint8Array.of(1)]],
    [[0, 'A'], [0, 'a']], [[1, 'A/'], [0, 'a/b']], [[0, 'a'], [0, 'a/b']],
    [[0, 'a/b'], [0, 'a']], [[1, 'a/'], [1, 'a/']], [[0, 'ß'], [0, 'ss']],
    [[0, 'x'.repeat(1025)]], [[0, 'a', new Uint8Array(16 * 1024 * 1024 + 1)]],
  ];
  for (const entries of badEntries) {
    const fs = fake();
    await assert.rejects(exportEntries(fs.root, packet([[0, 'valid'], ...entries]), job(), () => {}));
    assert.equal(fs.calls.length, 0);
  }
  const valid = packet([[0, 'a']]);
  for (const bytes of [valid.subarray(0, 2), valid.subarray(0, valid.length - 1), new Uint8Array([...valid, 0])]) {
    assert.throws(() => parseEntries(bytes));
  }
});

test('count, payload, and implicit directory budgets', () => {
  assert.throws(() => parseEntries(packet(Array.from({ length: 4097 }, (_, i) => [1, `${i}/`]))));
  assert.equal(parseEntries(packet(Array.from({ length: 2050 }, (_, i) => [0, `${i}/f`]))).length, 2050);
  assert.throws(() => parseEntries(packet(Array.from({ length: 257 }, (_, i) => [0, `${i}/${'d/'.repeat(62)}file-${i}`]))), /tree entry limit/);
  const data = new Uint8Array(16 * 1024 * 1024);
  assert.throws(() => parseEntries(packet([...Array.from({ length: 4 }, (_, i) => [0, `${i}`, data]), [0, 'extra', Uint8Array.of(1)]])), /Payload limit/);
  assert.equal(parseEntries(packet(Array.from({ length: 4096 }, (_, i) => [0, `${i}`]))).length, 4096);
});

for (const type of ['directory', 'file']) {
  test(`existing output ${type} is never merged`, async () => {
    const fs = fake((op, path, options, children) => {
      if (op === 'directory' && !options?.create) children.set(path.split('/').at(-1), { kind: type });
    });
    await assert.rejects(exportEntries(fs.root, packet([[0, 'a']]), job(), () => {}), /completed: 0/);
    assert.equal(fs.calls.some((call) => call[2]), false);
  });
}

for (const failure of ['NotAllowedError', 'SecurityError', 'TypeMismatchError']) {
  test(`lookup ${failure} is not absence`, async () => {
    const fs = fake(() => { throw namedError(failure); });
    await assert.rejects(exportEntries(fs.root, packet([[0, 'a']]), job(), () => {}), new RegExp(failure));
    assert.equal(fs.calls.length, 1);
  });
}

test('nested collision stops before create', async () => {
  const fs = fake((op, path, options, children) => {
    if (op === 'directory' && path.endsWith('/a') && !options?.create) children.set('a', { kind: 'file' });
  });
  await assert.rejects(exportEntries(fs.root, packet([[0, 'a']]), job(), () => {}), /TypeMismatchError.*completed: 0.*unfinished: a/);
  assert.equal(fs.streams.length, 0);
});

for (const stage of ['write', 'close', 'writable', 'file']) {
  test(`${stage} failure reports committed count and unfinished path`, async () => {
    const fs = fake((op, path) => { if (op === stage && path.endsWith('/bad')) throw new Error(`${stage} failed`); });
    const progress = [];
    const task = job();
    await assert.rejects(exportEntries(fs.root, packet([[0, 'ok'], [0, 'bad']]), task, (n) => progress.push(n)),
      /output: PeaZip-.*completed: 1; possibly unfinished: bad/);
    assert.deepEqual(progress, [1]);
    assert.equal(fs.root.children.has(task.outputName), true);
    if (stage === 'write') assert.equal(fs.streams.at(-1).aborted, true);
    if (stage === 'close') assert.equal(fs.streams.at(-1).aborted, false);
  });
}

for (const stage of ['directory', 'file', 'writable', 'write', 'close']) {
  test(`abort at ${stage} boundary accurately reports residue`, async () => {
    const task = job();
    const fs = fake((op) => { if (op === stage) task.controller.abort(); });
    const progress = [];
    await assert.rejects(exportEntries(fs.root, packet([[0, 'a'], [0, 'b']]), task, (n) => progress.push(n)), /Cancelled:.*output may remain/);
    assert.deepEqual(progress, stage === 'close' ? [1] : []);
    if (stage === 'write' || stage === 'writable') assert.equal(fs.streams[0].aborted, true);
    if (stage === 'close') assert.equal(fs.streams[0].closed, true);
  });
}

test('pre-abort performs no disk operations', async () => {
  const task = job();
  task.controller.abort();
  const fs = fake();
  await assert.rejects(exportEntries(fs.root, packet([]), task, () => {}), /Cancelled/);
  assert.equal(fs.calls.length, 0);
});

test('nested permission failure never proceeds to create', async () => {
  const fs = fake((op, path, options) => {
    if (path.endsWith('/a') && !options?.create) throw namedError('NotAllowedError');
  });
  await assert.rejects(exportEntries(fs.root, packet([[0, 'a/b']]), job(), () => {}), /NotAllowedError.*unfinished: a\/b/);
  assert.equal(fs.calls.some((call) => call[1].endsWith('/a') && call[2]), false);
});

test('abort cleanup failure preserves original failure and residue report', async () => {
  const fs = fake((op) => {
    if (op === 'write') throw new Error('write failure');
    if (op === 'abort') throw new Error('abort failure');
  });
  await assert.rejects(exportEntries(fs.root, packet([[0, 'a']]), job(), () => {}),
    /write failure.*completed: 0.*unfinished: a.*stream abort failed: abort failure/);
});

test('input data is snapshotted before filesystem awaits', async () => {
  const bytes = packet([[0, 'a', Uint8Array.of(42)]]);
  const fs = fake(() => bytes.fill(0));
  const task = job();
  await exportEntries(fs.root, bytes, task, () => {});
  assert.deepEqual(fs.root.children.get(task.outputName).children.get('a').bytes, Uint8Array.of(42));
});
