import assert from 'node:assert/strict';
import { test } from 'node:test';
import { moduleFor } from './web-worker-harness.mjs';

const flush = () => new Promise((resolve) => setImmediate(resolve));
function request(paths, version = 1) {
  const packet = new Uint8Array(28);
  const view = new DataView(packet.buffer);
  [version, paths, 1, 5].forEach((value, i) => view.setUint32(4 * i, value, true));
  packet.set(new TextEncoder().encode('tree/'), 16);
  view.setUint32(21, 3, true);
  packet.set([80, 75, 0], 25);
  return packet;
}
function output(path, content) {
  const name = new TextEncoder().encode(path);
  const bytes = new Uint8Array(16 + name.length + content.length);
  const view = new DataView(bytes.buffer);
  [1, 0, name.length, content.length].forEach((value, i) => view.setUint32(4 * i, value, true));
  bytes.set(name, 16);
  bytes.set(content, 16 + name.length);
  return bytes;
}

test('worker rejects unknown path mode/version and truncated headers before Wasm or archive reads', async () => {
  const messages = [];
  let inits = 0;
  const self = { postMessage: (message) => messages.push(message) };
  await moduleFor('archive-worker.js', { self }, async () => { inits++; throw new Error('Wasm must not start'); });
  for (const packet of [request(2), request(256), request(0, 2), request(1).slice(0, 7)]) {
    await self.onmessage({ data: { operation: 'extract-v1', packet: packet.buffer } });
    assert.equal(messages.at(-1).code, 0);
    assert.match(new TextDecoder().decode(messages.at(-1).bytes), /path mode|version|request/);
  }
  assert.equal(inits, 0);
});

test('worker preserves legacy dispatch and sends versioned packets unchanged with extraction progress', async () => {
  for (const paths of [null, 0, 1]) {
    const messages = [], calls = [];
    const data = request(paths ?? 0);
    const packet = paths === null ? data.slice(8) : data;
    const wasm = {
      memory: { buffer: new ArrayBuffer(4096) },
      pw_input_resize: (size) => { assert.equal(size, packet.length); return 16; },
      pw_extract: () => { calls.push('legacy'); return 1; },
      pw_extract_v1: () => { calls.push('v1'); return 1; },
      pw_output_len: () => 0, pw_output_ptr: () => 16,
    };
    const self = { postMessage: (message) => messages.push(message) };
    await moduleFor('archive-worker.js', { self }, async () => wasm);
    await self.onmessage({ data: { operation: paths === null ? 'extract' : 'extract-v1', packet: packet.buffer } });
    assert.deepEqual(calls, [paths === null ? 'legacy' : 'v1']);
    assert.deepEqual([...new Uint8Array(wasm.memory.buffer, 16, packet.length)], [...packet]);
    assert.deepEqual({ ...messages[0].progress }, { phase: 'extract', done: 0, total: 1 });
    assert.equal(messages.at(-1).code, 1);
  }
});

function filesystem(hook = () => {}) {
  const streams = [], calls = [];
  function directory() {
    const children = new Map();
    return { children,
      async getDirectoryHandle(name, options) {
        calls.push(['directory', name, !!options?.create]);
        if (!children.has(name)) {
          if (!options?.create) throw Object.assign(new Error('Not found'), { name: 'NotFoundError' });
          children.set(name, directory());
        }
        return children.get(name);
      },
      async getFileHandle(name) {
        calls.push(['file', name]);
        const file = { bytes: null, async createWritable() {
          const stream = {
            async write(bytes) { await hook('write'); file.bytes = bytes.slice(); },
            async close() { await hook('close'); this.closed = true; },
            async abort() { this.aborted = true; await hook('abort'); },
          };
          streams.push(stream);
          return stream;
        } };
        children.set(name, file);
        return file;
      },
    };
  }
  return { root: directory(), streams, calls };
}
async function bridge(fs, picker = async () => fs.root) {
  const workers = [], responses = [], progress = [];
  const memory = { buffer: new ArrayBuffer(8192) };
  let picks = 0, length = 0;
  const events = {};
  class Worker {
    constructor() { workers.push(this); }
    postMessage(message) { this.message = message; }
    terminate() { this.terminated = true; }
  }
  const namespace = await moduleFor('archive-bridge.js', {
    Worker, crypto: { randomUUID: () => '00000000-0000-4000-8000-000000000001' },
    window: { addEventListener: (name, callback) => { events[name] = callback; },
      showDirectoryPicker: () => { picks++; return picker(); } },
  });
  namespace.__nsh_finalize({ exports: {
    memory, pc_response_begin: (size) => { length = size; return 4096; },
    pc_response_end: (code) => responses.push({ code,
      text: new TextDecoder().decode(new Uint8Array(memory.buffer, 4096, length)) }),
    pc_progress: (...args) => progress.push(args),
  } });
  const legacy = request(0).slice(8);
  new Uint8Array(memory.buffer, 16, legacy.length).set(legacy);
  return { api: namespace.__nsh_imports().peazip, memory, legacy, workers, responses, progress, events,
    get picks() { return picks; } };
}

test('bridge validates mode before memory access/picker and versioned success keeps writer paths and bytes', async () => {
  const fs = filesystem();
  const h = await bridge(fs);
  for (const paths of [undefined, null, -1, 2, 256, 0.5, '1']) {
    // Invalid pointer/size must not be evaluated when the mode is invalid.
    assert.equal(h.api.pc_extract_with_options_start(-1, -1, paths), 0);
  }
  assert.equal(h.picks, 0);
  assert.equal(h.workers.length, 0);
  assert.equal(h.responses.length, 0);
  assert.equal(h.api.pc_extract_with_options_start(16, h.legacy.length, 1), 1);
  await flush();
  assert.equal(h.picks, 1);
  const worker = h.workers[0];
  assert.equal(worker.message.operation, 'extract-v1');
  assert.deepEqual([...new Uint8Array(worker.message.packet)], [...request(1)]);
  worker.onmessage({ data: { progress: { phase: 'extract', done: 0, total: 1 } } });
  const content = Uint8Array.of(0, 127, 255);
  await worker.onmessage({ data: { code: 1, bytes: output('你好.bin', content) } });
  await flush();
  assert.equal(worker.terminated, true);
  assert.equal(h.responses.at(-1).code, 1);
  assert.match(h.responses.at(-1).text, /^Extracted 1 entries to PeaZip-/);
  const directory = [...fs.root.children.values()][0];
  assert.deepEqual(directory.children.get('你好.bin').bytes, content);
  assert.equal(fs.streams[0].closed, true);
  assert.deepEqual(h.progress, [[2, 0, 1], [3, 1, 1]]);
});

test('legacy/full/flat picker and worker cancellation ignore late responses without creating output', async () => {
  for (const paths of [null, 0, 1]) {
    const fs = filesystem();
    const h = await bridge(fs);
    const start = () => paths === null ? h.api.pc_extract_start(16, h.legacy.length)
      : h.api.pc_extract_with_options_start(16, h.legacy.length, paths);
    assert.equal(start(), 1);
    await flush();
    const worker = h.workers[0];
    assert.equal(worker.message.operation, paths === null ? 'extract' : 'extract-v1');
    assert.deepEqual([...new Uint8Array(worker.message.packet)], paths === null ? [...h.legacy] : [...request(paths)]);
    assert.equal(h.api.pc_cancel(), 1);
    await worker.onmessage({ data: { code: 1, bytes: output('never', Uint8Array.of(1)) } });
    await flush();
    assert.equal(worker.terminated, true);
    assert.deepEqual(h.responses.map((r) => r.code), [2]);
    assert.deepEqual(fs.calls, []);
  }
  const fs = filesystem();
  let finishPicker;
  const h = await bridge(fs, () => new Promise((resolve) => { finishPicker = resolve; }));
  h.api.pc_extract_with_options_start(16, h.legacy.length, 1);
  h.api.pc_cancel();
  finishPicker(fs.root);
  await flush();
  assert.equal(h.workers.length, 0);
  assert.deepEqual(h.responses.map((r) => r.code), [2]);
  assert.deepEqual(fs.calls, []);
});

test('core failure creates no output; flat write failure still aborts and reports partial output', async () => {
  const fs = filesystem((stage) => { if (stage === 'write') throw new Error('disk full'); });
  const h = await bridge(fs);
  h.api.pc_extract_with_options_start(16, h.legacy.length, 1);
  await flush();
  await h.workers[0].onmessage({ data: { code: 0, bytes: new TextEncoder().encode('duplicate flat name') } });
  await flush();
  assert.deepEqual(fs.calls, []);
  assert.deepEqual(h.responses.map((r) => r.code), [0]);
  assert.equal(h.responses.at(-1).text, 'duplicate flat name');
  h.api.pc_extract_with_options_start(16, h.legacy.length, 1);
  await flush();
  await h.workers[1].onmessage({ data: { code: 1, bytes: output('failed', Uint8Array.of(1)) } });
  await flush();
  assert.equal(h.responses.at(-1).code, 0);
  assert.match(h.responses.at(-1).text, /Export failed: disk full;.*completed: 0; possibly unfinished: failed; output may remain/);
  assert.equal(fs.streams[0].aborted, true);
  assert.notEqual(fs.streams[0].closed, true);
});

for (const cancellation of ['Cancel', 'pagehide']) {
test(`flat ${cancellation} during a pending write preserves the partial-output failure report`, async () => {
  let began, rejectWrite;
  const started = new Promise((resolve) => { began = resolve; });
  const writing = new Promise((_, reject) => { rejectWrite = reject; });
  const fs = filesystem(async (stage) => {
    if (stage === 'write') { began(); await writing; }
    if (stage === 'abort') rejectWrite(new Error('write aborted'));
  });
  const h = await bridge(fs);
  h.api.pc_extract_with_options_start(16, h.legacy.length, 1);
  await flush();
  const result = h.workers[0].onmessage({ data: { code: 1, bytes: output('waiting', Uint8Array.of(1)) } });
  await started;
  if (cancellation === 'pagehide') h.events.pagehide();
  else assert.equal(h.api.pc_cancel(), 1);
  assert.equal(h.responses.length, 0);
  await result;
  await flush();
  assert.equal(h.responses.at(-1).code, 0);
  assert.match(h.responses.at(-1).text, /Cancelled:.*completed: 0; possibly unfinished: waiting; output may remain/);
  assert.equal(fs.streams[0].aborted, true);
  assert.notEqual(fs.streams[0].closed, true);
});
}
