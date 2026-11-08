const MAX_ENTRIES = 4096;
const MAX_OUTPUT_ITEMS = 16384;
const MAX_PATH = 1024;
const MAX_FILE = 16 * 1024 * 1024;
const MAX_PAYLOAD = 64 * 1024 * 1024;
const fold = (name) => name.normalize('NFKC').toUpperCase().toLowerCase();

export function parseEntries(packet) {
  if (!(packet instanceof Uint8Array)) throw new Error('Expected Uint8Array packet');
  const view = new DataView(packet.buffer, packet.byteOffset, packet.byteLength);
  let offset = 0;
  const u32 = () => {
    if (offset + 4 > packet.length) throw new Error('Truncated packet');
    const value = view.getUint32(offset, true);
    offset += 4;
    return value;
  };
  const count = u32();
  if (!count || count > MAX_ENTRIES) throw new Error('Entry limit exceeded');
  const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
  const entries = [];
  const nodes = new Map();
  let payload = 0;
  for (let i = 0; i < count; i++) {
    const kind = u32();
    const pathLength = u32();
    const dataLength = u32();
    if (kind > 1 || !pathLength || pathLength > MAX_PATH || dataLength > MAX_FILE) {
      throw new Error('Invalid kind or entry limits');
    }
    payload += dataLength;
    if (payload > MAX_PAYLOAD) throw new Error('Payload limit exceeded');
    if (offset + pathLength + dataLength > packet.length) throw new Error('Truncated entry');
    const path = decoder.decode(packet.subarray(offset, offset + pathLength));
    offset += pathLength;
    if (kind === 1 && (!path.endsWith('/') || dataLength !== 0)) {
      throw new Error('Directory must end in / and contain no data');
    }
    const name = kind === 1 ? path.slice(0, -1) : path;
    if (/[\\:\u0000-\u001f\u007f-\u009f]/u.test(name)) throw new Error('Unsafe path');
    const parts = name.split('/');
    if (parts.some((part) => !part || part === '.' || part === '..' ||
        /[. ]$/u.test(part) || /[\\/:\u0000-\u001f\u007f-\u009f]/u.test(fold(part)) ||
        fold(part) === '.' || fold(part) === '..')) throw new Error('Unsafe path');
    for (let depth = 1; depth <= parts.length; depth++) {
      const original = parts.slice(0, depth).join('/');
      const key = fold(original);
      const explicit = depth === parts.length;
      const type = explicit ? kind : 1;
      const previous = nodes.get(key);
      if (previous && (previous.name !== original || previous.kind !== type ||
          (explicit && previous.explicit))) throw new Error('Duplicate path or file-parent conflict');
      nodes.set(key, { name: original, kind: type, explicit: explicit || previous?.explicit });
      if (nodes.size > MAX_OUTPUT_ITEMS) throw new Error('Directory tree entry limit exceeded');
    }
    entries.push({ path, parts, kind, data: packet.slice(offset, offset + dataLength) });
    offset += dataLength;
  }
  if (offset !== packet.length) throw new Error('Trailing packet bytes');
  return entries;
}

export async function exportEntries(parent, packet, job, onProgress) {
  let done = 0;
  let pending = null;
  let stream = null;
  let closing = false;
  let aborting = null;
  const requestAbort = () => {
    if (stream && !closing && !aborting) {
      const writer = stream;
      aborting = Promise.resolve().then(() => writer.abort(job.controller.signal.reason))
        .then(() => null, (error) => error);
    }
  };
  job.controller.signal.addEventListener('abort', requestAbort);
  const check = () => job.controller.signal.throwIfAborted();
  const absent = async (directory, name) => {
    check();
    try {
      await directory.getDirectoryHandle(name);
    } catch (error) {
      check();
      if (error.name === 'NotFoundError') return;
      throw error;
    }
    check();
    throw new Error(`Destination already exists: ${name}`);
  };
  try {
    check();
    const entries = parseEntries(packet);
    check();
    job.outputName = `PeaZip-${crypto.randomUUID()}`;
    pending = job.outputName;
    await absent(parent, job.outputName);
    check();
    // Browser create has no atomic noclobber: an external actor can insert or
    // replace an entry between this check and create (including cached parents).
    const root = await parent.getDirectoryHandle(job.outputName, { create: true });
    check();
    const directories = new Map([['', root]]);
    for (const entry of entries) {
      check();
      pending = entry.path;
      let directory = root;
      const directoryDepth = entry.kind === 1 ? entry.parts.length : entry.parts.length - 1;
      for (let depth = 1; depth <= directoryDepth; depth++) {
        check();
        const key = entry.parts.slice(0, depth).join('/');
        if (!directories.has(key)) {
          const name = entry.parts[depth - 1];
          await absent(directory, name);
          check();
          const created = await directory.getDirectoryHandle(name, { create: true });
          directories.set(key, created);
          check();
        }
        directory = directories.get(key);
      }
      if (entry.kind === 0) {
        const name = entry.parts.at(-1);
        await absent(directory, name);
        check();
        const file = await directory.getFileHandle(name, { create: true });
        check();
        stream = await file.createWritable();
        check();
        await stream.write(entry.data);
        check();
        closing = true;
        await stream.close();
        stream = null;
        closing = false;
      }
      // Once close starts it cannot be undone; record successful commitment
      // even if cancellation arrived while close was in flight.
      done++;
      pending = null;
      onProgress(done, entries.length);
      check();
    }
    return `Extracted ${done} entries to ${job.outputName}`;
  } catch (error) {
    let abortFailure = '';
    if (stream && !closing) {
      requestAbort();
      const failure = await aborting;
      if (failure) abortFailure = `; stream abort failed: ${failure.message}`;
    }
    const state = job.controller.signal.aborted ? 'Cancelled' : 'Export failed';
    throw new Error(`${state}: ${error?.message ?? String(error)}; output: ${job.outputName ?? '(not created)'}; completed: ${done}; possibly unfinished: ${pending ?? '(none)'}; output may remain${abortFailure}`, { cause: error });
  } finally {
    job.controller.signal.removeEventListener('abort', requestAbort);
  }
}
