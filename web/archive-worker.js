import { validateExtractOptions, validateZipOptions, packetSize, MAX_ARCHIVE, MAX_ENTRIES, MAX_PATH, MAX_TOTAL } from './archive-limits.js';
import init from './worker-core.js';

self.onmessage = async ({ data: request }) => {
  try {
    if (request?.operation === 'list' || request?.operation === 'test') {
      const testing = request.operation === 'test';
      const packet = request.packet;
      if (!(packet instanceof ArrayBuffer) || !packet.byteLength || packet.byteLength > MAX_ARCHIVE) {
        throw new Error(`Invalid ${request.operation} request.`);
      }
      const wasm = await init({ module_or_path: new URL('./worker-core_bg.wasm', import.meta.url) });
      const pointer = wasm.pw_input_resize(packet.byteLength) >>> 0;
      if (!pointer) throw new Error('Archive request is too large.');
      new Uint8Array(wasm.memory.buffer, pointer, packet.byteLength).set(new Uint8Array(packet));
      const ok = (testing ? wasm.pw_test() : wasm.pw_list()) === 1;
      const length = wasm.pw_output_len() >>> 0;
      const limit = testing ? (ok ? 20 : 4096) : Math.min(MAX_ARCHIVE, 9 + MAX_ENTRIES * (32 + MAX_PATH));
      if (length > limit || (testing && ok && length !== 20)) {
        throw new Error('Worker returned an invalid or oversized report.');
      }
      const result = new Uint8Array(wasm.memory.buffer, wasm.pw_output_ptr() >>> 0, length).slice();
      if (!testing && ok && (length < 9 || result[0] !== 0x50 || result[1] !== 0x5a
          || result[2] !== 0x4c || result[3] !== 0x32)) {
        throw new Error('Invalid archive metadata version.');
      }
      self.postMessage({ code: ok ? 1 : 0, bytes: result }, [result.buffer]);
      return;
    }
    if (request?.operation === 'extract' || request?.operation === 'extract-v1') {
      const versioned = request.operation === 'extract-v1';
      const packet = request.packet;
      const headerSize = versioned ? 8 : 0;
      const maxRequest = MAX_ARCHIVE + MAX_ENTRIES * (4 + MAX_PATH) + 8 + headerSize;
      if (!(packet instanceof ArrayBuffer) || packet.byteLength < 8 + headerSize || packet.byteLength > maxRequest) {
        throw new Error('Invalid extraction request.');
      }
      const view = new DataView(packet);
      if (versioned) {
        if (view.getUint32(0, true) !== 1) throw new Error('Unsupported extract request version.');
        validateExtractOptions({ paths: view.getUint32(4, true) });
      }
      const selectedCount = view.getUint32(headerSize, true);
      if (!selectedCount || selectedCount > MAX_ENTRIES) throw new Error('Invalid selection count.');
      const wasm = await init({ module_or_path: new URL('./worker-core_bg.wasm', import.meta.url) });
      const pointer = wasm.pw_input_resize(packet.byteLength) >>> 0;
      if (!pointer) throw new Error('Archive request is too large.');
      new Uint8Array(wasm.memory.buffer, pointer, packet.byteLength).set(new Uint8Array(packet));
      self.postMessage({ progress: { phase: 'extract', done: 0, total: selectedCount } });
      const ok = (versioned ? wasm.pw_extract_v1() : wasm.pw_extract()) === 1;
      const length = wasm.pw_output_len() >>> 0;
      if (length > MAX_TOTAL + MAX_ENTRIES * (12 + MAX_PATH) + 4) {
        throw new Error('Worker output exceeds the extraction limit.');
      }
      const result = new Uint8Array(wasm.memory.buffer, wasm.pw_output_ptr() >>> 0, length).slice();
      self.postMessage({ code: ok ? 1 : 0, bytes: result }, [result.buffer]);
      return;
    }
    const versioned = request?.operation === 'create-v1';
    if (!versioned && !Array.isArray(request)) throw new Error('Invalid archive request.');
    const options = versioned ? validateZipOptions(request.options) : null;
    const files = versioned ? request.files : request;
    if (!Array.isArray(files)) throw new Error('Invalid archive request.');
    const size = packetSize(files) + (versioned ? 12 : 0);
    const wasm = await init({ module_or_path: new URL('./worker-core_bg.wasm', import.meta.url) });
    const pointer = wasm.pw_input_resize(size) >>> 0;
    if (!pointer) throw new Error('Archive request is too large.');
    const view = new DataView(wasm.memory.buffer);
    const bytes = new Uint8Array(wasm.memory.buffer);
    let at = pointer;
    if (versioned) {
      view.setUint32(at, 1, true);
      view.setUint32(at + 4, options.compression, true);
      view.setUint32(at + 8, options.level, true);
      at += 12;
    }
    view.setUint32(at, files.length, true);
    at += 4;
    let done = 0;
    const interval = Math.max(1, Math.ceil(files.length / 100));
    for (const file of files) {
      const name = new TextEncoder().encode(file.name);
      const content = new Uint8Array(await file.arrayBuffer());
      if (content.length !== file.size) throw new Error('File changed while reading.');
      view.setUint32(at, name.length, true);
      view.setUint32(at + 4, content.length, true);
      at += 8;
      bytes.set(name, at);
      at += name.length;
      bytes.set(content, at);
      at += content.length;
      done++;
      if (done % interval === 0 || done === files.length) {
        self.postMessage({ progress: { phase: 'read', done, total: files.length } });
      }
    }
    self.postMessage({ progress: { phase: 'compress', done, total: files.length } });
    const ok = (versioned ? wasm.pw_create_v1() : wasm.pw_create()) === 1;
    const length = wasm.pw_output_len() >>> 0;
    if (length > MAX_ARCHIVE) throw new Error('Worker output exceeds the archive limit.');
    const result = new Uint8Array(wasm.memory.buffer, wasm.pw_output_ptr() >>> 0, length).slice();
    self.postMessage({ code: ok ? 1 : 0, bytes: result }, [result.buffer]);
  } catch (error) {
    self.postMessage({ code: 0, bytes: new TextEncoder().encode(String(error.message ?? error).slice(0, 4096)) });
  }
};
