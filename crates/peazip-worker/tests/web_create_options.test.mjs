import assert from 'node:assert/strict';
import { test } from 'node:test';
import { moduleFor } from './web-worker-harness.mjs';

async function worker() {
  const messages = [];
  const calls = [];
  let size;
  const wasm = {
    memory: { buffer: new ArrayBuffer(4096) },
    pw_input_resize: (length) => { size = length; return 16; },
    pw_create: () => { calls.push('legacy'); return 1; },
    pw_create_v1: () => { calls.push('v1'); return 1; },
    pw_create_v2: () => { calls.push('v2'); return 1; },
    pw_output_len: () => 0,
    pw_output_ptr: () => 16,
  };
  const self = { postMessage: (message) => messages.push(message) };
  let inits = 0;
  await moduleFor('archive-worker.js', { self }, async () => { inits++; return wasm; });
  return { self, messages, calls, wasm, get size() { return size; }, get inits() { return inits; } };
}

const invalidOptions = [undefined, null, {}, [], { compression: 0 },
  { compression: 0, level: 1 }, { compression: 1, level: 9 },
  { compression: 2, level: 0 }, { compression: 2, level: 10 },
  { compression: 2, level: 1.5 }, { compression: 2, level: '6' },
  { compression: 3, level: 0 }, { compression: 2, level: 6, extra: true }];

test('create-v1 rejects bad options before metadata, file reads or Wasm init', async () => {
  const harness = await worker();
  let reads = 0;
  const file = { get name() { reads++; throw new Error('metadata read'); },
    arrayBuffer() { reads++; throw new Error('content read'); } };
  for (const options of invalidOptions) {
    await harness.self.onmessage({ data: { operation: 'create-v1', options, files: [file] } });
    assert.equal(harness.messages.at(-1).code, 0);
    assert.match(new TextDecoder().decode(harness.messages.at(-1).bytes), /compression options/);
  }
  assert.equal(reads, 0);
  assert.equal(harness.inits, 0);
  assert.deepEqual(harness.calls, []);
  await harness.self.onmessage({ data: { operation: 'create-v3', files: [file] } });
  assert.equal(harness.messages.at(-1).code, 0);
  assert.equal(reads, 0);
});

test('create-v1 writes exact version/options header; legacy packet and progress remain unchanged', async () => {
  for (const options of [null, { compression: 0, level: 0 }, { compression: 1, level: 0 },
    { compression: 2, level: 1 }, { compression: 2, level: 9 }]) {
    const harness = await worker();
    const content = new Uint8Array([0, 127, 255]);
    let reads = 0;
    const files = [{ name: '你好.bin', size: content.length,
      async arrayBuffer() { reads++; return content.buffer; } }];
    await harness.self.onmessage({ data: options ? { operation: 'create-v1', options, files } : files });
    assert.equal(reads, 1);
    assert.deepEqual(harness.calls, [options ? 'v1' : 'legacy']);
    const view = new DataView(harness.wasm.memory.buffer);
    let at = 16;
    if (options) {
      assert.deepEqual([0, 4, 8].map((offset) => view.getUint32(at + offset, true)),
        [1, options.compression, options.level]);
      at += 12;
    }
    assert.equal(view.getUint32(at, true), 1);
    const nameLength = view.getUint32(at + 4, true);
    assert.equal(nameLength, new TextEncoder().encode(files[0].name).length);
    assert.equal(view.getUint32(at + 8, true), 3);
    const packet = new Uint8Array(harness.wasm.memory.buffer, at + 12, nameLength + 3);
    assert.equal(new TextDecoder().decode(packet.subarray(0, nameLength)), '你好.bin');
    assert.deepEqual([...packet.subarray(nameLength)], [...content]);
    assert.equal(harness.size, 4 + 8 + nameLength + 3 + (options ? 12 : 0));
    assert.deepEqual(harness.messages.filter((m) => m.progress).map((m) => ({ ...m.progress })),
      [{ phase: 'read', done: 1, total: 1 }, { phase: 'compress', done: 1, total: 1 }]);
    assert.equal(harness.messages.at(-1).code, 1);
  }
});

test('bridge validates before picker and preserves cancellation, progress and legacy dispatch', async () => {
  const inputs = [];
  const workers = [];
  const responses = [];
  const progress = [];
  const document = { body: { append() {} }, createElement: () => {
    const listeners = {};
    const input = { listeners, addEventListener: (event, callback) => { listeners[event] = callback; },
      click() {}, remove() { this.removed = true; } };
    inputs.push(input);
    return input;
  } };
  class Worker {
    constructor() { workers.push(this); }
    postMessage(message) { this.message = message; }
    terminate() { this.terminated = true; }
  }
  const bridge = await moduleFor('archive-bridge.js', {
    document, Worker, window: { addEventListener() {} },
  });
  bridge.__nsh_finalize({ exports: {
    memory: { buffer: new ArrayBuffer(4096) },
    pc_response_begin: () => 16, pc_response_end: (code) => responses.push(code),
    pc_progress: (...args) => progress.push(args),
  } });
  const api = bridge.__nsh_imports().peazip;
  assert.equal(api.pc_create_with_options_start(2, 0), 0);
  assert.equal(api.pc_create_with_options_start(7, 0), 0);
  assert.equal(inputs.length, 0);
  for (const options of [null, { compression: 1, level: 0 }, { compression: 2, level: 9 }]) {
    assert.equal(options ? api.pc_create_with_options_start(options.compression, options.level)
      : api.pc_create_start(), 1);
    const input = inputs.at(-1);
    input.files = [{ name: 'file', size: 3 }];
    input.listeners.change();
    const current = workers.at(-1);
    if (options) {
      assert.equal(current.message.operation, 'create-v1');
      assert.deepEqual({ ...current.message.options }, options);
    } else assert.ok(Array.isArray(current.message));
    current.onmessage({ data: { progress: { phase: 'read', done: 1, total: 1 } } });
    current.onmessage({ data: { progress: { phase: 'compress', done: 1, total: 1 } } });
    assert.equal(api.pc_cancel(), 1);
    assert.equal(current.terminated, true);
    assert.equal(input.removed, true);
    // A terminated worker cannot complete or overwrite the cancelled job.
    current.onmessage({ data: { code: 1, bytes: new Uint8Array([1]) } });
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(responses.at(-1), 2);
    assert.equal(api.pc_cancel(), 0);
  }
  assert.deepEqual(responses, [2, 2, 2]);
  assert.deepEqual(progress, Array.from({ length: 3 }, () => [[0, 1, 1], [1, 1, 1]]).flat());
  assert.equal(api.pc_create_with_options_start(0, 0), 1);
  inputs.at(-1).listeners.cancel();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(responses.at(-1), 2);
});

const invalidCreateOptions = [undefined, null, {}, [], { format: 0 },
  { format: 0, compression: 0, level: 1 }, { format: 0, compression: 2, level: 0 },
  { format: 1, compression: 1, level: 0 }, { format: 1, compression: 0, level: 1 },
  { format: 2, compression: 1, level: 6 }, { format: 2, compression: 0, level: 0 },
  { format: 2, compression: 0, level: 10 }, { format: 2, compression: 0, level: '6' },
  { format: 3, compression: 0, level: 1 }, { format: 3, compression: 1, level: 0 },
  { format: 3, compression: 1, level: 10 }, { format: 3, compression: 1, level: 257 },
  { format: 3, compression: 2, level: 6 }, { format: 3, compression: -1, level: 0 },
  { format: 3, compression: 1, level: 1.5 }, { format: 3, compression: 1, level: '6' },
  { format: 3, compression: '0', level: 0 }, { format: 3, compression: 0, level: null },
  { format: 3, compression: 1, level: NaN }, { format: 3, compression: 1, level: Infinity },
  { format: 3, compression: 0, level: 0, extra: true },
  { format: 4, compression: 0, level: 0 }, { format: '1', compression: 0, level: 0 },
  { format: 1, compression: 0, level: 0, extra: true }];

test('create-v2 validates format, unused fields and levels before metadata, reads or Wasm', async () => {
  const h = await worker();
  let reads = 0;
  const file = { get name() { reads++; throw new Error('metadata read'); },
    arrayBuffer() { reads++; throw new Error('content read'); } };
  for (const options of invalidCreateOptions) {
    await h.self.onmessage({ data: { operation: 'create-v2', options, files: [file] } });
    assert.equal(h.messages.at(-1).code, 0);
    assert.match(new TextDecoder().decode(h.messages.at(-1).bytes), /options/);
  }
  assert.equal(reads, 0);
  assert.equal(h.inits, 0);
  assert.deepEqual(h.calls, []);
});

test('create-v2 writes the exact sixteen-byte header and preserves read/compress progress', async () => {
  for (const [format, compression, level] of [[0, 0, 0], [0, 1, 0], [0, 2, 1], [0, 2, 9],
    [1, 0, 0], [2, 0, 1], [2, 0, 9], [3, 0, 0],
    ...Array.from({ length: 9 }, (_, index) => [3, 1, index + 1])]) {
    const h = await worker();
    const content = Uint8Array.of(0, 127, 255);
    const files = [{ name: '你好.bin', size: 3, arrayBuffer: async () => content.buffer }];
    await h.self.onmessage({ data: { operation: 'create-v2', options: { format, compression, level }, files } });
    assert.deepEqual(h.calls, ['v2']);
    const view = new DataView(h.wasm.memory.buffer);
    assert.deepEqual([0, 4, 8, 12, 16].map((offset) => view.getUint32(16 + offset, true)),
      [2, format, compression, level, 1]);
    const name = new TextEncoder().encode(files[0].name);
    assert.equal(h.size, 16 + 4 + 8 + name.length + 3);
    const bytes = new Uint8Array(h.wasm.memory.buffer, 16 + 16 + 12, name.length + 3);
    assert.deepEqual([...bytes], [...name, ...content]);
    assert.deepEqual(h.messages.filter((m) => m.progress).map((m) => ({ ...m.progress })),
      [{ phase: 'read', done: 1, total: 1 }, { phase: 'compress', done: 1, total: 1 }]);
    assert.equal(h.messages.at(-1).code, 1);
  }
});

test('7z methods reject file and payload limits before reading files or initializing Wasm', async () => {
  const h = await worker();
  let reads = 0;
  const file = { name: 'file', size: 0, arrayBuffer() { reads++; throw new Error('content read'); } };
  const tooLarge = { ...file, size: 16 * 1024 * 1024 + 1 };
  const total = Array.from({ length: 5 }, () => ({ ...file, size: 16 * 1024 * 1024 }));
  for (const [compression, level] of [[0, 0], [1, 1], [1, 9]]) {
    for (const files of [[], Array(4097).fill(file), [tooLarge], total,
      [{ ...file, name: '' }], [{ ...file, name: '你'.repeat(342) }]]) {
      await h.self.onmessage({ data: { operation: 'create-v2', options: { format: 3, compression, level }, files } });
      assert.equal(h.messages.at(-1).code, 0);
    }
  }
  assert.equal(reads, 0);
  assert.equal(h.inits, 0);
  assert.deepEqual(h.calls, []);
});

test('create-v2 bridge validates before picker and isolates cancelled/late jobs across formats', async () => {
  const inputs = [], workers = [], responses = [], progress = [];
  const document = { body: { append() {} }, createElement: () => {
    const listeners = {};
    const input = { listeners, addEventListener: (event, callback) => { listeners[event] = callback; },
      click() {}, remove() { this.removed = true; } };
    inputs.push(input);
    return input;
  } };
  class Worker {
    constructor(url) { this.url = url; workers.push(this); }
    postMessage(message) { this.message = message; }
    terminate() { this.terminated = true; }
  }
  const bridge = await moduleFor('archive-bridge.js', { document, Worker, window: { addEventListener() {} } });
  bridge.__nsh_finalize({ exports: {
    memory: { buffer: new ArrayBuffer(4096) }, pc_response_begin: () => 16,
    pc_response_end: (code) => responses.push(code), pc_progress: (...args) => progress.push(args),
  } });
  const api = bridge.__nsh_imports().peazip;
  for (const options of invalidCreateOptions) {
    if (!options || Array.isArray(options) || options.extra) continue;
    assert.equal(api.pc_create_archive_start(options.format, options.compression, options.level), 0);
  }
  assert.equal(inputs.length, 0);
  let previous;
  for (const [format, compression, level] of [[0, 0, 0], [1, 0, 0], [2, 0, 9], [3, 0, 0], [3, 1, 1], [3, 1, 9]]) {
    assert.equal(api.pc_create_archive_start(format, compression, level), 1);
    const input = inputs.at(-1);
    input.files = [{ name: 'file', size: 3 }];
    input.listeners.change();
    const current = workers.at(-1);
    assert.match(current.url.href, /\/archive-worker\.js$/);
    assert.equal(current.message.operation, 'create-v2');
    assert.deepEqual({ ...current.message.options }, { format, compression, level });
    const before = progress.length;
    for (const update of [{ phase: 'compress', done: -1, total: 1 },
      { phase: 'compress', done: 2, total: 1 }, { phase: 'read', done: 0, total: 0 },
      { phase: 'compress', done: 1, total: 4097 }, { phase: 'compress', done: 0.5, total: 1 },
      { phase: 'extract', done: 1, total: 1 }, { phase: 'write', done: 1, total: 1 }]) {
      current.onmessage({ data: { progress: update } });
    }
    assert.equal(progress.length, before);
    current.onmessage({ data: { progress: { phase: 'read', done: 1, total: 1 } } });
    current.onmessage({ data: { progress: { phase: 'compress', done: 1, total: 1 } } });
    assert.deepEqual(progress.slice(before), [[0, 1, 1], [1, 1, 1]]);
    if (previous) {
      const count = progress.length;
      previous.onmessage({ data: { progress: { phase: 'compress', done: 1, total: 1 } } });
      assert.equal(progress.length, count);
      previous.onmessage({ data: { code: 1, bytes: Uint8Array.of(1) } });
    }
    assert.equal(api.pc_cancel(), 1);
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(responses.at(-1), 2);
    assert.equal(current.terminated, true);
    assert.equal(input.removed, true);
    previous = current;
  }
  assert.deepEqual(responses, [2, 2, 2, 2, 2, 2]);
  assert.equal(workers.length, 6, 'create does not start any metadata/list worker');
});
