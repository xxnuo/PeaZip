import { packetSize, MAX_ARCHIVE } from './archive-limits.js';
import init from './worker-core.js';

self.onmessage = async ({ data: files }) => {
  try {
    const size = packetSize(files);
    const wasm = await init({ module_or_path: new URL('./worker-core_bg.wasm', import.meta.url) });
    const pointer = wasm.pw_input_resize(size) >>> 0;
    if (!pointer) throw new Error('Archive request is too large.');
    const view = new DataView(wasm.memory.buffer);
    const bytes = new Uint8Array(wasm.memory.buffer);
    let at = pointer;
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
    const ok = wasm.pw_create() === 1;
    const length = wasm.pw_output_len() >>> 0;
    if (length > MAX_ARCHIVE) throw new Error('Worker output exceeds the archive limit.');
    const result = new Uint8Array(wasm.memory.buffer, wasm.pw_output_ptr() >>> 0, length).slice();
    self.postMessage({ code: ok ? 1 : 0, bytes: result }, [result.buffer]);
  } catch (error) {
    self.postMessage({ code: 0, bytes: new TextEncoder().encode(String(error.message ?? error).slice(0, 4096)) });
  }
};
