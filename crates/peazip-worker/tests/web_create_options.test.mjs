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
  await harness.self.onmessage({ data: { operation: 'create-v2', files: [file] } });
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
