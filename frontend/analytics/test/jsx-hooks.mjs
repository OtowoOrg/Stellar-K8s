// Node ESM loader hook: transpile `.jsx` on import with esbuild so the React
// components can be unit tested with the built-in `node --test` runner without
// pulling in a browser DOM.

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { transform } from 'esbuild';

export async function load(url, context, nextLoad) {
  if (url.endsWith('.jsx')) {
    const source = await readFile(fileURLToPath(url), 'utf8');
    const { code } = await transform(source, {
      loader: 'jsx',
      jsx: 'automatic',
      format: 'esm',
      target: 'node20',
      sourcefile: fileURLToPath(url),
    });
    return { format: 'module', source: code, shortCircuit: true };
  }
  return nextLoad(url, context);
}
