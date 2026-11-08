import assert from 'node:assert/strict';
import { test } from 'node:test';
import { moduleFor } from './web-worker-harness.mjs';

async function listWorker(output, ok = 1) {
  const messages = [];
  let calls = 0;
  const wasm = {
    memory: { buffer: new ArrayBuffer(output.length + 32) },
    pw_input_resize: () => 8,
    pw_list: () => { calls++; return ok; },
    pw_output_len: () => output.length,
    pw_output_ptr: () => 16,
  };
  new Uint8Array(wasm.memory.buffer, 16, output.length).set(output);
  const self = { postMessage: (message) => messages.push(message) };
  await moduleFor('archive-worker.js', { self }, async () => wasm);
  await self.onmessage({ data: { operation: 'list', packet: new ArrayBuffer(1) } });
  assert.equal(calls, 1);
  return messages.at(-1);
}

test('list worker only forwards successful metadata with the PZL2 header', async () => {
  const valid = Uint8Array.of(0x50, 0x5a, 0x4c, 0x32, 0, 0, 0, 0, 0);
  const response = await listWorker(valid);
  assert.equal(response.code, 1);
  assert.deepEqual([...response.bytes], [...valid]);
  for (const old of [new Uint8Array(), new Uint8Array(5), valid.subarray(0, 8),
    Uint8Array.of(0x50, 0x5a, 0x4c, 0x31, 0, 0, 0, 0, 0)]) {
    const rejected = await listWorker(old);
    assert.equal(rejected.code, 0);
    assert.match(new TextDecoder().decode(rejected.bytes), /metadata version/);
  }
  const failure = await listWorker(new TextEncoder().encode('invalid archive'), 0);
  assert.equal(failure.code, 0);
  assert.equal(new TextDecoder().decode(failure.bytes), 'invalid archive');
});

test('list metadata limit includes the nine-byte PZL2 header and rejects one byte over', async () => {
  const maximum = 9 + 4096 * (32 + 1024);
  const output = new Uint8Array(maximum);
  output.set([0x50, 0x5a, 0x4c, 0x32]);
  const accepted = await listWorker(output);
  assert.equal(accepted.code, 1);
  assert.equal(accepted.bytes.length, maximum);
  const rejected = await listWorker(new Uint8Array(maximum + 1));
  assert.equal(rejected.code, 0);
  assert.match(new TextDecoder().decode(rejected.bytes), /oversized report/);
});
