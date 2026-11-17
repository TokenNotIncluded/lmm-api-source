#!/usr/bin/env node
// Keep independent package release archives self-contained, from one maintained runtime.
import { readdir, readFile, writeFile, access } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const source = await readFile(join(root, 'scripts/shared/models-dev.ts'), 'utf8');
const javascript = stripTypeScriptTypes(source, { mode: 'strip' })
  .replace(/[ \t]+$/gm, '').replace(/\n{3,}/g, '\n\n');
const check = process.argv.includes('--check');
const exists = (path) => access(path).then(() => true, () => false);
const targets = [];
for (const entry of await readdir(join(root, 'packages'), { withFileTypes: true })) {
  if (!entry.isDirectory()) continue;
  const directory = join(root, 'packages', entry.name);
  // DSH consumes its pinned Pi provider submodule rather than a second runtime.
  if (await exists(join(directory, 'vendor/pi-lmm-provider/src'))) continue;
  if (await exists(join(directory, 'src'))) {
    const extension = await exists(join(directory, 'tsconfig.json')) ? 'ts' : 'mjs';
    targets.push([join(directory, 'src/models-dev-cache.' + extension), extension === 'ts' ? source : javascript]);
  } else if (await exists(join(directory, 'codewhale.mjs'))) {
    targets.push([join(directory, 'models-dev-cache.mjs'), javascript]);
  }
}
const changed = [];
for (const [path, content] of targets) {
  const current = await readFile(path, 'utf8').catch(() => undefined);
  if (current === content) continue;
  changed.push(path.slice(root.length + 1));
  if (!check) await writeFile(path, content);
}
if (check && changed.length) {
  console.error('Outdated model metadata runtime copies:\n' + changed.join('\n'));
  process.exitCode = 1;
} else {
  console.log(JSON.stringify({ copies: targets.length, changed: changed.length, check }));
}
