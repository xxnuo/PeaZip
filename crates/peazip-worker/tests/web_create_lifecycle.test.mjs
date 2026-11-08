import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { moduleFor } from './web-worker-harness.mjs';

async function harness(options) {
  const tasks = [], events = {}, inputs = [], workers = [], replies = [];
  const memory = { buffer: new ArrayBuffer(8192) };
  let length;
  const channel = {};
  const document = { body: { append() {} }, createElement: () => {
    const listeners = {};
    const input = { listeners, addEventListener: (name, callback) => { listeners[name] = callback; },
      click() {}, remove() { this.removed = true; } };
    inputs.push(input);
    return input;
  } };
  class Worker {
    constructor(url) { this.url = url; workers.push(this); }
    postMessage(message) { this.message = message; }
    terminate() { this.terminated = true; }
  }
  const bridge = await moduleFor('archive-bridge.js', { document, Worker,
    queueMicrotask: (callback) => tasks.push(callback),
    window: { addEventListener: (name, callback) => { events[name] = callback; }, showDirectoryPicker() {
      throw new Error('hidden page must not open a picker');
    } },
  });
  bridge.__nsh_finalize({ exports: { memory, pc_progress() {},
    pc_response_begin: (size) => { length = size; return 16; },
    pc_response_end: (code) => {
      replies.push({ code, bytes: new Uint8Array(memory.buffer, 16, length).slice() });
      channel.resume?.(code);
    },
  } });
  const api = bridge.__nsh_imports().peazip;
  function startCreate() {
    assert.equal(api.pc_create_archive_start(...options), 1);
    inputs.at(-1).files = [{ name: 'file', size: 3 }];
    inputs.at(-1).listeners.change();
    return workers.at(-1);
  }
  function result(worker, code = 1) {
    worker.onmessage({ data: { code, bytes: Uint8Array.of(1, 2, 3) } });
    assert.equal(worker.terminated, true);
  }
  function deliver() {
    assert.equal(tasks.length, 1);
    tasks.shift()();
    return replies.at(-1);
  }
  return { api, tasks, events, inputs, workers, replies, channel, startCreate, result, deliver };
}

for (const options of [[2, 0, 6], [3, 0, 0], [3, 1, 9]]) {
  describe(`create-v2 lifecycle ${options.join('/')}`, () => {
    for (const invalidate of ['cancel', 'pagehide']) {
      test(`logical create: hold first success microtask then ${invalidate} settles code2 without starting list`, async () => {
        const h = await harness(options);
        const token = h.api.pc_create_operation_begin();
        assert.ok(token);
        h.result(h.startCreate());
        assert.equal(h.api.pc_create_operation_begin(), 0);
        if (invalidate === 'cancel') assert.equal(h.api.pc_cancel(), 1);
        else h.events.pagehide();
        assert.equal(h.api.pc_create_operation_valid(token), 0);
        assert.equal(h.api.pc_list_start(16, 3), 0);
        assert.equal(h.deliver().code, 2);
        assert.equal(h.replies[0].bytes.length, 0);
        assert.equal(h.workers.length, 1);
        assert.equal(h.api.pc_create_operation_end(token), 1);
      });

      test(`logical create: hold second success microtask then ${invalidate} prevents final publication`, async () => {
        const h = await harness(options);
        const token = h.api.pc_create_operation_begin();
        h.result(h.startCreate());
        assert.equal(h.deliver().code, 1);
        assert.equal(h.api.pc_create_operation_valid(token), 1);
        assert.equal(h.api.pc_list_start(16, 3), 1);
        assert.equal(h.workers[1].message.operation, 'list');
        h.result(h.workers[1]);
        if (invalidate === 'cancel') assert.equal(h.api.pc_cancel(), 1);
        else h.events.pagehide();
        assert.equal(h.deliver().code, 2);
        assert.equal(h.api.pc_create_operation_valid(token), 0);
        assert.equal(h.api.pc_create_operation_end(token), 1);
      });
    }

    for (const stage of ['first', 'second']) {
      for (const code of [0, 1]) {
        test(`logical create: ${stage} reply code${code} already in Rust channel remains cancellable before token check`, async () => {
          const h = await harness(options);
          const token = h.api.pc_create_operation_begin();
          h.result(h.startCreate(), stage === 'first' ? code : 1);
          if (stage === 'second') {
            h.deliver();
            assert.equal(h.api.pc_list_start(16, 3), 1);
            h.result(h.workers[1], code);
          }
          assert.equal(h.deliver().code, code);
          // pc_response_end has delivered, but the async Rust receiver has not resumed yet.
          assert.equal(h.api.pc_cancel(), 1);
          assert.equal(h.api.pc_create_operation_valid(token), 0);
          assert.equal(h.api.pc_list_start(16, 3), 0);
          assert.equal(h.api.pc_create_operation_begin(), 0);
          assert.equal(h.api.pc_create_operation_end(token), 1);
        });
      }
    }

    test('pagehide between stages invalidates token with no active worker; pageshow only enables new starts after end', async () => {
      const h = await harness(options);
      const token = h.api.pc_create_operation_begin();
      h.result(h.startCreate());
      h.deliver();
      h.events.pagehide();
      assert.equal(h.api.pc_create_operation_valid(token), 0);
      assert.equal(h.api.pc_create_operation_begin(), 0);
      assert.equal(h.api.pc_create_start(), 0);
      assert.equal(h.api.pc_create_with_options_start(0, 0), 0);
      assert.equal(h.api.pc_create_archive_start(0, 0, 0), 0);
      assert.equal(h.api.pc_list_start(16, 3), 0);
      assert.equal(h.api.pc_test_start(16, 3), 0);
      assert.equal(h.api.pc_extract_start(16, 3), 0);
      assert.equal(h.api.pc_extract_with_options_start(16, 3, 0), 0);
      assert.equal(h.api.pc_download(16, 3, 0), 0);
      h.events.pageshow();
      assert.equal(h.api.pc_create_operation_valid(token), 0);
      assert.equal(h.api.pc_create_operation_begin(), 0);
      h.api.pc_create_operation_end(token);
      const retry = h.api.pc_create_operation_begin();
      assert.ok(retry && retry !== token);
      assert.equal(h.api.pc_create_operation_end(token), 0);
      h.result(h.startCreate());
      assert.equal(h.deliver().code, 1);
      assert.equal(h.api.pc_create_operation_valid(retry), 1);
      assert.equal(h.api.pc_list_start(16, 3), 1);
      h.result(h.workers.at(-1));
      assert.equal(h.deliver().code, 1);
      assert.equal(h.api.pc_create_operation_valid(retry), 1);
      h.api.pc_create_operation_end(retry);
    });

    test('end before queued cancellation delivery reserves the token until the old waiter is settled', async () => {
      const h = await harness(options);
      const token = h.api.pc_create_operation_begin();
      const old = h.startCreate();
      h.result(old);
      h.events.pagehide();
      h.events.pageshow();
      assert.equal(h.api.pc_create_operation_end(token), 1);
      assert.equal(h.api.pc_create_operation_begin(), 0);
      assert.equal(h.deliver().code, 2);
      const retry = h.api.pc_create_operation_begin();
      assert.ok(retry && retry !== token);
      h.startCreate();
      const count = h.replies.length;
      old.onmessage({ data: { code: 1, bytes: Uint8Array.of(9) } });
      assert.equal(h.tasks.length, 0);
      assert.equal(h.replies.length, count);
      h.api.pc_cancel();
      assert.equal(h.deliver().code, 2);
      h.api.pc_create_operation_end(retry);
    });

    test('legacy queued replies are not resurrected by hide/show and cannot overlap a new pending reply', async () => {
      const h = await harness(options);
      h.result(h.startCreate());
      h.events.pagehide();
      h.events.pageshow();
      assert.equal(h.api.pc_create_operation_begin(), 0);
      assert.equal(h.api.pc_create_start(), 0);
      assert.equal(h.deliver().code, 2);
      assert.ok(h.api.pc_create_operation_begin());
    });

    test('a synchronously resumed Rust receiver can start the list stage after old-buffer writes finish', async () => {
      const h = await harness(options);
      const token = h.api.pc_create_operation_begin();
      h.result(h.startCreate());
      let stages = 0;
      h.channel.resume = (code) => {
        assert.equal(code, 1);
        assert.equal(h.api.pc_create_operation_valid(token), 1);
        if (++stages === 1) assert.equal(h.api.pc_list_start(16, 3), 1);
        else h.api.pc_create_operation_end(token);
      };
      h.deliver();
      assert.equal(h.workers.length, 2);
      h.result(h.workers[1]);
      h.deliver();
      assert.equal(stages, 2);
      assert.ok(h.api.pc_create_operation_begin());
    });
  });
}
