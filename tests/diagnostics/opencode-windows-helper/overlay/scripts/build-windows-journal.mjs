import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstat, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { helperSources, helperExecutable, helperReceipt, fileRecord, inspectExecutable, verifyWindowsJournalBuild } from './verify-windows-journal-build.mjs';

assert.equal(process.platform, 'win32', 'The package helper must be compiled on Windows');
const root = fileURLToPath(new URL('../', import.meta.url));
assert(process.env.SystemRoot && /^[A-Za-z]:\\/.test(process.env.SystemRoot), 'Missing Windows system root');
const framework = resolve(process.env.SystemRoot, 'Microsoft.NET/Framework64/v4.0.30319');
const tools = ['csc.exe', 'mscorlib.dll', 'System.dll', 'System.Core.dll', 'System.Runtime.Serialization.dll', 'System.Xml.dll'];
const sourceRecords = () => Promise.all(helperSources.map((path) => fileRecord(resolve(root, path), path)));
const toolRecords = () => Promise.all(tools.map((name) => fileRecord(resolve(framework, name), name, 32 * 1024 * 1024)));
for (const path of [helperExecutable, helperReceipt]) {
  await assert.rejects(lstat(resolve(root, path)), { code: 'ENOENT' }, `Refusing to overwrite an existing ${path}`);
}
const sources = await sourceRecords();
const toolchainBefore = await toolRecords();
const args = [
  '/noconfig', '/nostdlib+', '/target:exe', '/platform:anycpu', '/optimize+', '/debug-',
  '/langversion:5', '/main:LmmMagpieJournalProgram', `/out:${resolve(root, helperExecutable)}`,
  ...tools.slice(1).map((name) => `/reference:${resolve(framework, name)}`),
  ...helperSources.map((path) => resolve(root, path)),
];
const started = performance.now();
const compile = spawnSync(resolve(framework, 'csc.exe'), args, {
  cwd: root, encoding: 'utf8', windowsHide: true, timeout: 60000, maxBuffer: 1024 * 1024,
});
let processExited = false;
if (Number.isSafeInteger(compile.pid) && compile.pid > 0) {
  try { process.kill(compile.pid, 0); } catch (error) { processExited = error.code === 'ESRCH'; }
}
// Preserve actual compiler support/errors. Do not assume /deterministic or infer
// a reproducible binary; the receipt binds this actual output and these inputs.
process.stdout.write(compile.stdout ?? '');
process.stderr.write(compile.stderr ?? '');
const compiler = {
  path: resolve(framework, 'csc.exe'), arguments: args, exit_code: compile.status,
  signal: compile.signal, elapsed_ms: Math.floor(performance.now() - started),
  pid: compile.pid ?? null, process_exited: processExited,
  stdout_sha256: createHash('sha256').update(compile.stdout ?? '').digest('hex'),
  stderr_sha256: createHash('sha256').update(compile.stderr ?? '').digest('hex'),
};
console.log(JSON.stringify({ event: 'windows_journal_compiler', ...compiler }));
assert.ifError(compile.error);
assert.equal(compile.status, 0, 'Windows helper compilation failed');
assert.equal(compile.signal, null);
assert(processExited, 'Compiler process did not leave a verifiable exited state');
assert.deepEqual(await sourceRecords(), sources, 'Helper source changed during compilation');
const toolchainAfter = await toolRecords();
assert.deepEqual(toolchainAfter, toolchainBefore, 'Compiler/reference inputs changed during compilation');
const executable = await fileRecord(resolve(root, helperExecutable), helperExecutable);
const pe = inspectExecutable(await readFile(resolve(root, helperExecutable)));
await writeFile(resolve(root, helperReceipt), JSON.stringify({
  schema: 'lmm-windows-journal-build.v1', platform: process.platform, arch: process.arch,
  built_at: new Date().toISOString(), sources, compiler,
  toolchain_before: toolchainBefore, toolchain_after: toolchainAfter, executable, pe,
}, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify(await verifyWindowsJournalBuild(root), null, 2));
