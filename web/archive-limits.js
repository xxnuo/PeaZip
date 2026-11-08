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

// create-v1 wire values: Automatic=0, Stored=1, Deflate=2; unused levels must be zero.
export function validateZipOptions(options) {
  if (!options || typeof options !== 'object' || Array.isArray(options)
      || Object.keys(options).some((key) => key !== 'compression' && key !== 'level')
      || !Number.isInteger(options.compression) || !Number.isInteger(options.level)
      || !((options.compression === 0 || options.compression === 1) && options.level === 0
        || options.compression === 2 && options.level >= 1 && options.level <= 9)) {
    throw new Error('Invalid ZIP compression options.');
  }
  return options;
}

// extract-v1 wire modes: Full=0, FilesOnly=1. Flattening belongs to the Rust core.
export function validateExtractOptions(options) {
  if (!options || typeof options !== 'object' || Array.isArray(options)
      || Object.keys(options).some((key) => key !== 'paths')
      || (options.paths !== 0 && options.paths !== 1)) {
    throw new Error('Invalid extraction path mode.');
  }
  return options;
}

// create-v2 adds format: ZIP=0, TAR=1, TAR.GZ=2. Unused fields must be zero.
export function validateCreateOptions(options) {
  if (!options || typeof options !== 'object' || Array.isArray(options)
      || Object.keys(options).some((key) => !['format', 'compression', 'level'].includes(key))) {
    throw new Error('Invalid archive creation options.');
  }
  const { format, compression, level } = options;
  if (format === 0) {
    validateZipOptions({ compression, level });
  } else if (!((format === 1 && compression === 0 && level === 0)
      || (format === 2 && compression === 0 && Number.isInteger(level) && level >= 1 && level <= 9))) {
    throw new Error('Invalid archive creation options.');
  }
  return options;
}
