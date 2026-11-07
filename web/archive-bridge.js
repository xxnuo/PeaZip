import { packetSize, MAX_ARCHIVE } from './archive-limits.js';

let wasm;
let active;
const encoder = new TextEncoder();

function reply(code, bytes = new Uint8Array(), job = active) {
  if (!job || job !== active) return;
  active = null;
  job.input.remove();
  job.worker?.terminate();
  if (bytes.length > MAX_ARCHIVE) {
    code = 0;
    bytes = encoder.encode('Worker output exceeds the archive limit.');
  }
  queueMicrotask(() => {
    const ptr = wasm.pc_response_begin(bytes.length) >>> 0;
    if (!ptr && bytes.length) {
      wasm.pc_response_end(0);
      return;
    }
    if (bytes.length) new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
    wasm.pc_response_end(code);
  });
}

function fail(error, job = active) {
  reply(0, encoder.encode(String(error.message ?? error).slice(0, 4096)), job);
}

function create() {
  if (active || !wasm || typeof Worker === 'undefined') return 0;
  const input = document.createElement('input');
  input.type = 'file';
  input.multiple = true;
  input.hidden = true;
  document.body.append(input);
  const job = { input, worker: null };
  active = job;
  input.addEventListener('cancel', () => reply(2, undefined, job), { once: true });
  input.addEventListener('change', () => {
    if (active !== job) return;
    const files = Array.from(input.files ?? []);
    if (!files.length) { reply(2, undefined, job); return; }
    try {
      packetSize(files); // Reject oversized selections before reading any file bytes.
      const worker = new Worker(new URL('./archive-worker.js', import.meta.url), { type: 'module' });
      job.worker = worker;
      worker.onmessage = ({ data }) => {
        if (job !== active) return;
        if (data.progress) {
          const { phase, done, total } = data.progress;
          if (!Number.isInteger(done) || !Number.isInteger(total)) return;
          if (phase === 'read' || phase === 'compress') wasm.pc_progress(phase === 'read' ? 0 : 1, done, total);
        } else {
          reply(data.code, data.bytes, job);
        }
      };
      worker.onerror = (event) => { event.preventDefault(); fail(new Error('Archive worker failed.'), job); };
      worker.onmessageerror = () => fail(new Error('Archive worker returned an invalid response.'), job);
      worker.postMessage(files);
    } catch (error) { fail(error, job); }
  }, { once: true });
  try { input.click(); } catch (error) { fail(error, job); }
  return 1;
}

function download(ptr, length) {
  if (!wasm || !length || length > MAX_ARCHIVE) return 0;
  let url;
  let link;
  try {
    const bytes = new Uint8Array(wasm.memory.buffer, ptr >>> 0, length).slice();
    url = URL.createObjectURL(new Blob([bytes], { type: 'application/zip' }));
    link = document.createElement('a');
    link.href = url;
    link.download = 'Archive.zip';
    document.body.append(link);
    link.click();
    return 1;
  } catch { return 0; }
  finally {
    link?.remove();
    if (url) setTimeout(() => URL.revokeObjectURL(url), 30_000);
  }
}

export function __nsh_imports() {
  return { peazip: { pc_create_start: create, pc_download: download, pc_cancel: () => {
    if (!active) return 0;
    reply(2);
    return 1;
  } } };
}

export function __nsh_finalize(instance) {
  wasm = instance.exports;
}

window.addEventListener('pagehide', () => {
  reply(2);
});
