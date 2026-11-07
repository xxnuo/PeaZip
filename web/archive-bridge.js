import { packetSize, MAX_ARCHIVE, MAX_ENTRIES, MAX_PATH } from './archive-limits.js';
import { exportEntries } from './directory-export.js';

let wasm;
let active;
const encoder = new TextEncoder();

function reply(code, bytes = new Uint8Array(), job = active) {
  if (!job || job !== active) return;
  active = null;
  job.input?.remove();
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

function progress(data, job) {
  if (job !== active || job.controller?.signal.aborted) return;
  const { phase, done, total } = data;
  if (!Number.isInteger(done) || !Number.isInteger(total) || done < 0 || total < 1 || total > MAX_ENTRIES || done > total) return;
  const phases = { read: 0, compress: 1, extract: 2, write: 3 };
  if (Object.hasOwn(phases, phase)) wasm.pc_progress(phases[phase], done, total);
}

function extract(ptr, length) {
  if (active || !wasm || typeof Worker === 'undefined' || typeof window.showDirectoryPicker !== 'function') return 0;
  const job = { worker: null, phase: 'pick', controller: new AbortController(), outputName: null };
  active = job;
  try {
    if (length > MAX_ARCHIVE + MAX_ENTRIES * (4 + MAX_PATH) + 8) throw new Error('Extraction request exceeds the limit.');
    const packet = new Uint8Array(wasm.memory.buffer, ptr >>> 0, length).slice();
    // Request permission during the user's action, before waiting for decompression.
    Promise.resolve(window.showDirectoryPicker({ mode: 'readwrite' })).then((parent) => {
      if (active !== job) return;
      const worker = new Worker(new URL('./archive-worker.js', import.meta.url), { type: 'module' });
      job.worker = worker;
      job.phase = 'extract';
      worker.onmessage = async ({ data }) => {
        if (active !== job) return;
        if (data.progress) { progress(data.progress, job); return; }
        worker.terminate();
        job.worker = null;
        if (data.code !== 1) { reply(0, data.bytes, job); return; }
        job.phase = 'write';
        try {
          const report = await exportEntries(parent, data.bytes, job, (done, total) => progress({ phase: 'write', done, total }, job));
          reply(1, encoder.encode(report), job);
        } catch (error) { fail(error, job); }
      };
      worker.onerror = (event) => { event.preventDefault(); fail(new Error('Archive worker failed.'), job); };
      worker.onmessageerror = () => fail(new Error('Archive worker returned an invalid response.'), job);
      worker.postMessage({ operation: 'extract', packet: packet.buffer }, [packet.buffer]);
    }).catch((error) => {
      if (error.name === 'AbortError') reply(2, undefined, job);
      else fail(error, job);
    });
  } catch (error) { fail(error, job); }
  return 1;
}

function cancel() {
  if (!active) return 0;
  if (active.phase === 'write') {
    // File close may already be committing: wait for the writer's accurate partial-output report.
    active.controller.abort();
  } else {
    reply(2);
  }
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
  return { peazip: { pc_create_start: create, pc_download: download, pc_cancel: cancel,
    pc_extract_start: extract, pc_can_extract: () => typeof window.showDirectoryPicker === 'function' ? 1 : 0,
  } };
}

export function __nsh_finalize(instance) {
  wasm = instance.exports;
}

window.addEventListener('pagehide', () => {
  cancel();
});
