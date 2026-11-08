import { validateCreateOptions, validateExtractOptions, validateZipOptions, packetSize, MAX_ARCHIVE, MAX_ENTRIES, MAX_PATH } from './archive-limits.js';
import { exportEntries } from './directory-export.js';

let wasm;
let active;
let pageActive = true;
let pageEpoch = 0;
let logicalCreate;
let nextCreateToken = 0;
let queuedReplies = 0;

function createOperationValid(token) {
  return pageActive && logicalCreate && logicalCreate.token === (token >>> 0) && logicalCreate.valid ? 1 : 0;
}
function finishCreateOperation() {
  if (logicalCreate?.ended && !active && queuedReplies === 0) logicalCreate = undefined;
}
function createOperationBegin() {
  if (!pageActive || !wasm || active || queuedReplies || logicalCreate || nextCreateToken === 0xffffffff) return 0;
  logicalCreate = { token: ++nextCreateToken, valid: true, ended: false };
  return logicalCreate.token;
}
function createOperationEnd(token) {
  if (!logicalCreate || logicalCreate.token !== (token >>> 0)) return 0;
  logicalCreate.ended = true;
  logicalCreate.valid = false;
  if (active?.createToken === token) cancel();
  finishCreateOperation();
  return 1;
}
function stageAllowed() {
  return pageActive && !active && !queuedReplies && (!logicalCreate || logicalCreate.valid);
}
function stageJob(fields) {
  return { ...fields, epoch: pageEpoch, createToken: logicalCreate?.token };
}
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
  queuedReplies++;
  queueMicrotask(() => {
    try {
      // A writer may have committed files: preserve its accurate partial-output report on hide.
      if (job.phase !== 'write' && (job.epoch !== pageEpoch || !pageActive
          || (job.createToken !== undefined && !createOperationValid(job.createToken)))) {
        code = 2;
        bytes = new Uint8Array();
      }
      const ptr = wasm.pc_response_begin(bytes.length) >>> 0;
      if (!ptr && bytes.length) code = 0;
      else if (bytes.length) new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
    } catch {
      code = 0;
    }
    // All writes to the old PENDING are complete before its receiver may resume.
    queuedReplies--;
    try {
      wasm.pc_response_end(code);
    } finally {
      finishCreateOperation();
    }
  });
}

function fail(error, job = active) {
  reply(0, encoder.encode(String(error.message ?? error).slice(0, 4096)), job);
}

function create(options, version = 1) {
  if (options !== undefined) {
    try { version === 2 ? validateCreateOptions(options) : validateZipOptions(options); } catch { return 0; }
  }
  if (!stageAllowed() || !wasm || typeof Worker === 'undefined') return 0;
  const input = document.createElement('input');
  input.type = 'file';
  input.multiple = true;
  input.hidden = true;
  document.body.append(input);
  const job = stageJob({ input, worker: null });
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
      worker.postMessage(options === undefined ? files : { operation: version === 2 ? 'create-v2' : 'create-v1', options, files });
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

function inspect(ptr, length, operation) {
  if (!stageAllowed() || !wasm || typeof Worker === 'undefined') return 0;
  const job = stageJob({ worker: null, phase: operation });
  active = job;
  try {
    if (!Number.isInteger(length) || length < 1 || length > MAX_ARCHIVE) throw new Error(`Invalid ${operation} request size.`);
    const packet = new Uint8Array(wasm.memory.buffer, ptr >>> 0, length).slice();
    const url = new URL('./archive-worker.js', import.meta.url);
    url.searchParams.set('operation', operation);
    const worker = new Worker(url, { type: 'module' });
    job.worker = worker;
    worker.onmessage = ({ data }) => {
      if (active !== job) return;
      if (!(data.bytes instanceof Uint8Array)) { fail(new Error('Archive worker returned an invalid response.'), job); return; }
      reply(data.code, data.bytes, job);
    };
    worker.onerror = (event) => { event.preventDefault(); fail(new Error('Archive worker failed.'), job); };
    worker.onmessageerror = () => fail(new Error('Archive worker returned an invalid response.'), job);
    worker.postMessage({ operation, packet: packet.buffer }, [packet.buffer]);
  } catch (error) { fail(error, job); }
  return 1;
}

function extract(ptr, length, options) {
  if (options !== undefined) {
    try { validateExtractOptions(options); } catch { return 0; }
  }
  if (!stageAllowed() || !wasm || typeof Worker === 'undefined' || typeof window.showDirectoryPicker !== 'function') return 0;
  const job = stageJob({ worker: null, phase: 'pick', controller: new AbortController(), outputName: null });
  active = job;
  try {
    if (!Number.isInteger(length) || length < 8
        || length > MAX_ARCHIVE + MAX_ENTRIES * (4 + MAX_PATH) + 8) throw new Error('Invalid extraction request size.');
    const packet = new Uint8Array(length + (options === undefined ? 0 : 8));
    const offset = options === undefined ? 0 : 8;
    if (offset) {
      const header = new DataView(packet.buffer);
      header.setUint32(0, 1, true);
      header.setUint32(4, options.paths, true);
    }
    packet.set(new Uint8Array(wasm.memory.buffer, ptr >>> 0, length), offset);
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
      worker.postMessage({ operation: options === undefined ? 'extract' : 'extract-v1', packet: packet.buffer }, [packet.buffer]);
    }).catch((error) => {
      if (error.name === 'AbortError') reply(2, undefined, job);
      else fail(error, job);
    });
  } catch (error) { fail(error, job); }
  return 1;
}

function cancel() {
  if (logicalCreate) logicalCreate.valid = false;
  if (!active) return logicalCreate ? 1 : 0;
  if (active.phase === 'write') {
    // File close may already be committing: wait for the writer's accurate partial-output report.
    active.controller.abort();
  } else {
    reply(2);
  }
  return 1;
}

function download(ptr, length, format) {
  if (!pageActive || !wasm || !length || length > MAX_ARCHIVE) return 0;
  const kind = {
    0: { name: 'Archive.zip', mime: 'application/zip' },
    1: { name: 'Archive.tar', mime: 'application/x-tar' },
    2: { name: 'Archive.tar.gz', mime: 'application/gzip' },
    3: { name: 'Archive.7z', mime: 'application/x-7z-compressed' },
  }[format];
  if (!kind) return 0;
  let url;
  let link;
  try {
    const bytes = new Uint8Array(wasm.memory.buffer, ptr >>> 0, length).slice();
    url = URL.createObjectURL(new Blob([bytes], { type: kind.mime }));
    link = document.createElement('a');
    link.href = url;
    link.download = kind.name;
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
  return { peazip: {
    pc_create_operation_begin: createOperationBegin,
    pc_create_operation_valid: createOperationValid,
    pc_create_operation_end: createOperationEnd,
    pc_create_start: () => create(),
    pc_create_archive_start: (format, compression, level) => create({ format, compression, level }, 2),
    pc_create_with_options_start: (compression, level) => create({ compression, level }), pc_download: download, pc_cancel: cancel,
    pc_list_start: (ptr, length) => inspect(ptr, length, 'list'),
    pc_test_start: (ptr, length) => inspect(ptr, length, 'test'),
    pc_extract_start: (ptr, length) => extract(ptr, length),
    pc_extract_with_options_start: (ptr, length, paths) => extract(ptr, length, { paths }),
    pc_can_extract: () => typeof window.showDirectoryPicker === 'function' ? 1 : 0,
  } };
}

export function __nsh_finalize(instance) {
  wasm = instance.exports;
}

window.addEventListener('pagehide', () => {
  pageActive = false;
  pageEpoch++;
  cancel();
});
window.addEventListener('pageshow', () => { pageActive = true; });
