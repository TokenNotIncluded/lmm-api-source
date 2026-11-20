import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { lstat, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const helperDirectory = 'src/windows-journal';
export const helperSources = ['JournalStorage.cs', 'Program.cs'].map((name) => `${helperDirectory}/${name}`);
export const helperExecutable = `${helperDirectory}/lmm-magpie-journal.exe`;
export const helperReceipt = `${helperDirectory}/build.json`;
const packageRoot = fileURLToPath(new URL('../', import.meta.url));
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

export async function fileRecord(path, label = path, maximum = 1024 * 1024) {
  const info = await lstat(path);
  assert(info.isFile() && !info.isSymbolicLink() && info.size > 0 && info.size <= maximum, `Invalid build input: ${label}`);
  const bytes = await readFile(path);
  assert.equal(bytes.length, info.size);
  return { path: label, bytes: bytes.length, sha256: sha256(bytes) };
}

export function inspectExecutable(bytes) {
  assert(bytes.length >= 512 && bytes.length <= 1024 * 1024, 'Invalid helper size');
  assert.equal(bytes.toString('ascii', 0, 2), 'MZ');
  const pe = bytes.readUInt32LE(0x3c);
  assert(pe >= 0x40 && pe + 24 <= bytes.length, 'Invalid PE offset');
  assert.equal(bytes.readUInt32LE(pe), 0x4550);
  assert.equal(bytes.readUInt16LE(pe + 4), 0x14c, 'Expected AnyCPU PE machine');
  const sections = bytes.readUInt16LE(pe + 6);
  const optionalSize = bytes.readUInt16LE(pe + 20);
  const characteristics = bytes.readUInt16LE(pe + 22);
  const optional = pe + 24;
  assert(sections > 0 && sections <= 32 && optionalSize >= 224);
  assert(optional + optionalSize + sections * 40 <= bytes.length);
  assert((characteristics & 2) !== 0 && (characteristics & 0x2000) === 0);
  assert.equal(bytes.readUInt16LE(optional), 0x10b, 'Expected PE32');
  assert(bytes.readUInt32LE(optional + 92) >= 15);
  const cliRva = bytes.readUInt32LE(optional + 96 + 14 * 8);
  const cliSize = bytes.readUInt32LE(optional + 96 + 14 * 8 + 4);
  assert(cliRva > 0 && cliSize >= 72);
  let cli = -1;
  for (let i = 0; i < sections; i++) {
    const section = optional + optionalSize + i * 40;
    const address = bytes.readUInt32LE(section + 12);
    const rawSize = bytes.readUInt32LE(section + 16);
    const raw = bytes.readUInt32LE(section + 20);
    if (cliRva >= address && cliRva + 72 <= address + rawSize) {
      assert.equal(cli, -1, 'Ambiguous CLI section');
      cli = raw + cliRva - address;
    }
  }
  assert(cli >= 0 && cli + 72 <= bytes.length);
  assert(bytes.readUInt32LE(cli) >= 72);
  const flags = bytes.readUInt32LE(cli + 16);
  assert((flags & 1) !== 0 && (flags & (2 | 16 | 0x20000)) === 0, 'Helper must be IL-only AnyCPU');
  assert(bytes.readUInt32LE(cli + 20) !== 0, 'Missing managed entry point');
  return { machine: 'I386', format: 'PE32', cli_flags: flags, il_only: true, required_32_bit: false, preferred_32_bit: false };
}

export async function verifyWindowsJournalBuild(root = packageRoot) {
  const receiptPath = resolve(root, helperReceipt);
  await fileRecord(receiptPath, helperReceipt, 32768);
  const receipt = JSON.parse(await readFile(receiptPath, 'utf8'));
  assert.equal(receipt.schema, 'lmm-windows-journal-build.v1');
  assert.equal(receipt.platform, 'win32');
  assert.equal(receipt.compiler.exit_code, 0);
  assert.equal(receipt.compiler.signal, null);
  assert.equal(receipt.compiler.process_exited, true);
  assert(receipt.compiler.arguments.includes('/platform:anycpu'));
  assert(receipt.compiler.arguments.includes('/main:LmmMagpieJournalProgram'));
  const sources = await Promise.all(helperSources.map((path) => fileRecord(resolve(root, path), path)));
  assert.deepEqual(receipt.sources, sources, 'Helper source differs from the Windows build receipt');
  const executable = await fileRecord(resolve(root, helperExecutable), helperExecutable);
  assert.deepEqual(receipt.executable, executable, 'Helper binary differs from the Windows build receipt');
  const pe = inspectExecutable(await readFile(resolve(root, helperExecutable)));
  assert.deepEqual(receipt.pe, pe);
  assert.deepEqual(receipt.toolchain_before, receipt.toolchain_after, 'Compiler/reference inputs changed during build');
  assert.deepEqual(receipt.toolchain_before.map((file) => file.path), [
    'csc.exe', 'mscorlib.dll', 'System.dll', 'System.Core.dll', 'System.Runtime.Serialization.dll', 'System.Xml.dll',
  ]);
  for (const file of receipt.toolchain_before) {
    assert(Number.isSafeInteger(file.bytes) && file.bytes > 0 && /^[a-f0-9]{64}$/.test(file.sha256));
  }
  return { schema: 'lmm-windows-journal-verification.v1', sources, executable, pe, build_receipt: await fileRecord(receiptPath, helperReceipt) };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(JSON.stringify(await verifyWindowsJournalBuild(), null, 2));
}
