// Match the core's default policy; the Rust packet decoder rechecks every limit.
export const MAX_ENTRIES = 4096;
export const MAX_ENTRY = 16 * 1024 * 1024;
export const MAX_TOTAL = 64 * 1024 * 1024;
export const MAX_ARCHIVE = 32 * 1024 * 1024;
export const MAX_PATH = 1024;

export function packetSize(files) {
  if (!files.length || files.length > MAX_ENTRIES) throw new Error('Invalid number of files.');
  const encoder = new TextEncoder();
  let total = 0;
  let size = 4;
  for (const file of files) {
    const length = encoder.encode(file.name).length;
    if (!length || length > MAX_PATH) throw new Error('Filename is too long or empty.');
    if (file.size > MAX_ENTRY) throw new Error('A file exceeds the 16 MiB limit.');
    total += file.size;
    if (total > MAX_TOTAL) throw new Error('Selected files exceed the 64 MiB total limit.');
    size += 8 + length + file.size;
  }
  return size;
}
