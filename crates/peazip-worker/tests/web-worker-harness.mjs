import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const web = new URL('../../../web/', import.meta.url);

export async function moduleFor(name, globals, init) {
  const context = vm.createContext({ URL, TextEncoder, TextDecoder, Uint8Array, DataView, ArrayBuffer,
    queueMicrotask, setTimeout, AbortController, ...globals });
  const cache = new Map();
  async function load(url) {
    if (cache.has(url.href)) return cache.get(url.href);
    const source = new vm.SourceTextModule(await readFile(fileURLToPath(url), 'utf8'), {
      context, identifier: url.href, initializeImportMeta: (meta) => { meta.url = url.href; },
    });
    cache.set(url.href, source);
    await source.link(async (specifier, parent) => {
      if (specifier === './worker-core.js') {
        return new vm.SyntheticModule(['default'], function () { this.setExport('default', init); }, { context });
      }
      return load(new URL(specifier, parent.identifier));
    });
    return source;
  }
  const module = await load(new URL(name, web));
  await module.evaluate();
  return module.namespace;
}
