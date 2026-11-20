import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstat, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
// These shared helpers inspect file/PE bytes only. Neither the product storage
// implementation nor the product executable is invoked or compiled here.
import { fileRecord, inspectExecutable } from '../scripts/verify-windows-journal-build.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const sourcePaths = ['test/windows-journal-acl/Program.cs'];
const executablePath = 'test/windows-journal-acl/lmm-test-acl.exe';
const receiptPath = 'test/windows-journal-acl/build.json';
const tools = ['csc.exe', 'mscorlib.dll', 'System.dll', 'System.Core.dll', 'System.Runtime.Serialization.dll', 'System.Xml.dll'];
const sourceRecords = () => Promise.all(sourcePaths.map((path) => fileRecord(resolve(root, path), path)));

async function verify() {
  await fileRecord(resolve(root, receiptPath), receiptPath, 32768);
  const receipt = JSON.parse(await readFile(resolve(root, receiptPath), 'utf8'));
  assert.equal(receipt.schema, 'lmm-windows-test-acl-build.v1');
  assert.equal(receipt.platform, 'win32');
  assert.equal(receipt.compiler.exit_code, 0);
  assert.equal(receipt.compiler.signal, null);
  assert.equal(receipt.compiler.process_exited, true);
  assert(receipt.compiler.arguments.includes('/platform:anycpu'));
  assert(receipt.compiler.arguments.includes('/main:LmmWindowsTestAclProgram'));
  const compiledSources = receipt.compiler.arguments.filter((arg) => arg.endsWith('.cs'));
  assert.equal(compiledSources.length, 1);
  assert(compiledSources[0].replaceAll('\\', '/').endsWith('/test/windows-journal-acl/Program.cs'));
  const sources = await sourceRecords();
  assert.deepEqual(receipt.sources, sources, 'Test oracle source differs from its actual Windows build');
  const executable = await fileRecord(resolve(root, executablePath), executablePath);
  assert.deepEqual(receipt.executable, executable, 'Test oracle executable differs from its build receipt');
  const pe = inspectExecutable(await readFile(resolve(root, executablePath)));
  assert.deepEqual(receipt.pe, pe);
  assert.deepEqual(receipt.toolchain_before, receipt.toolchain_after);
  assert.deepEqual(receipt.toolchain_before.map((file) => file.path), tools);
  for (const file of receipt.toolchain_before) {
    assert(Number.isSafeInteger(file.bytes) && file.bytes > 0 && /^[a-f0-9]{64}$/.test(file.sha256));
  }
  return {
    schema: 'lmm-windows-test-acl-verification.v1', sources, executable, pe,
    build_receipt: await fileRecord(resolve(root, receiptPath), receiptPath),
  };
}

assert(process.argv.length === 2 || (process.argv.length === 3 && process.argv[2] === '--verify'), 'Invalid test oracle build mode');
if (process.argv[2] === '--verify') {
  console.log(JSON.stringify(await verify(), null, 2));
} else {
  assert.equal(process.platform, 'win32', 'The test oracle must be compiled on Windows');
  assert(process.env.SystemRoot && /^[A-Za-z]:\\/.test(process.env.SystemRoot), 'Missing Windows system root');
  const framework = resolve(process.env.SystemRoot, 'Microsoft.NET/Framework64/v4.0.30319');
  const toolRecords = () => Promise.all(tools.map((name) => fileRecord(resolve(framework, name), name, 32 * 1024 * 1024)));
  for (const path of [executablePath, receiptPath]) {
    await assert.rejects(lstat(resolve(root, path)), { code: 'ENOENT' }, `Refusing to overwrite an existing ${path}`);
  }
  const sources = await sourceRecords();
  const toolchainBefore = await toolRecords();
  const args = [
    '/noconfig', '/nostdlib+', '/target:exe', '/platform:anycpu', '/optimize+', '/debug-',
    '/langversion:5', '/main:LmmWindowsTestAclProgram', `/out:${resolve(root, executablePath)}`,
    ...tools.slice(1).map((name) => `/reference:${resolve(framework, name)}`),
    ...sourcePaths.map((path) => resolve(root, path)),
  ];
  const started = performance.now();
  const compile = spawnSync(resolve(framework, 'csc.exe'), args, {
    cwd: root, encoding: 'utf8', windowsHide: true, timeout: 60000, maxBuffer: 1024 * 1024,
  });
  let processExited = false;
  if (Number.isSafeInteger(compile.pid) && compile.pid > 0) {
    try { process.kill(compile.pid, 0); } catch (error) { processExited = error.code === 'ESRCH'; }
  }
  // Preserve the actual compiler result. Framework csc is not assumed to
  // support deterministic output; this receipt binds the actual binary.
  process.stdout.write(compile.stdout ?? '');
  process.stderr.write(compile.stderr ?? '');
  const compiler = {
    path: resolve(framework, 'csc.exe'), arguments: args, exit_code: compile.status,
    signal: compile.signal, elapsed_ms: Math.floor(performance.now() - started),
    pid: compile.pid ?? null, process_exited: processExited,
    stdout_sha256: createHash('sha256').update(compile.stdout ?? '').digest('hex'),
    stderr_sha256: createHash('sha256').update(compile.stderr ?? '').digest('hex'),
  };
  console.log(JSON.stringify({ event: 'windows_test_acl_compiler', ...compiler }));
  assert.ifError(compile.error);
  assert.equal(compile.status, 0, 'Windows test oracle compilation failed');
  assert.equal(compile.signal, null);
  assert(processExited, 'Compiler process did not leave a verifiable exited state');
  assert.deepEqual(await sourceRecords(), sources, 'Test oracle source changed during compilation');
  const toolchainAfter = await toolRecords();
  assert.deepEqual(toolchainAfter, toolchainBefore, 'Compiler/reference inputs changed during compilation');
  const executable = await fileRecord(resolve(root, executablePath), executablePath);
  const pe = inspectExecutable(await readFile(resolve(root, executablePath)));
  await writeFile(resolve(root, receiptPath), JSON.stringify({
    schema: 'lmm-windows-test-acl-build.v1', platform: process.platform, arch: process.arch,
    built_at: new Date().toISOString(), sources, compiler,
    toolchain_before: toolchainBefore, toolchain_after: toolchainAfter, executable, pe,
  }, null, 2) + '\n', { flag: 'wx' });
  console.log(JSON.stringify(await verify(), null, 2));
}
