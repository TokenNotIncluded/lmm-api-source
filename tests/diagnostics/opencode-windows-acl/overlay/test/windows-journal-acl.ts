import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstat, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';

type ACL = { ownerMatches: boolean; protected: boolean; ruleCount: number; onlyCurrent: boolean; fingerprint: string };
const helper = fileURLToPath(new URL('./windows-journal-acl/lmm-test-acl.exe', import.meta.url));
let verifiedBuild: Promise<void> | undefined;

async function oracleFile(name: string, maximum: number): Promise<Buffer> {
  const path = new URL(`./windows-journal-acl/${name}`, import.meta.url);
  const info = await lstat(path);
  assert(info.isFile() && !info.isSymbolicLink() && info.size > 0 && info.size <= maximum);
  const bytes = await readFile(path);
  assert.equal(bytes.length, info.size);
  return bytes;
}

async function verifyBuild(): Promise<void> {
  try {
    const [source, executable, receiptBytes] = await Promise.all([
      oracleFile('Program.cs', 1024 * 1024),
      oracleFile('lmm-test-acl.exe', 1024 * 1024),
      oracleFile('build.json', 32768),
    ]);
    const receipt = JSON.parse(receiptBytes.toString('utf8'));
    const record = (name: string, bytes: Buffer) => ({
      path: `test/windows-journal-acl/${name}`, bytes: bytes.length,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    });
    assert.equal(receipt.schema, 'lmm-windows-test-acl-build.v1');
    assert.equal(receipt.platform, 'win32');
    assert.equal(receipt.compiler.exit_code, 0);
    assert.equal(receipt.compiler.signal, null);
    assert.equal(receipt.compiler.process_exited, true);
    assert.deepEqual(receipt.sources, [record('Program.cs', source)]);
    assert.deepEqual(receipt.executable, record('lmm-test-acl.exe', executable));
  } catch {
    throw new Error('Unverified Windows test oracle build.');
  }
}

// Only callers in these tests pass their newly created, owned temporary paths.
export async function windowsTestACL(path: string, action: 'read' | 'grant_other' | 'private' = 'read'): Promise<ACL> {
  if (process.platform !== 'win32' || !process.env.SystemRoot) throw Error('Windows test helper unavailable.');
  await (verifiedBuild ??= verifyBuild());
  const started = performance.now();
  return new Promise((resolve, reject) => {
    const rejectProcess = (error: Error & { code?: string | number | null; signal?: string | null; killed?: boolean }) => {
      const knownCodes = ['ENOENT', 'EACCES', 'EPERM', 'EPIPE', 'ETIMEDOUT', 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER'];
      const knownSignals = ['SIGTERM', 'SIGKILL', 'SIGINT', 'SIGABRT'];
      reject(new Error(JSON.stringify({
        event: 'windows_test_acl_process_failure',
        action: action === 'read' || action === 'grant_other' || action === 'private' ? action : 'unknown',
        elapsed_ms: Math.max(0, Math.round(performance.now() - started)),
        pid: Number.isSafeInteger(child.pid) ? child.pid : null,
        code: typeof error.code === 'number' && Number.isSafeInteger(error.code) ? error.code
          : typeof error.code === 'string' && knownCodes.includes(error.code) ? error.code
            : error.code == null ? null : 'OTHER',
        signal: error.signal == null ? null : knownSignals.includes(error.signal) ? error.signal : 'OTHER',
        killed: error.killed === true,
      })));
    };
    const child = execFile(helper, [],
      { windowsHide: true, timeout: 5000, maxBuffer: 4096, encoding: 'utf8' }, (error, stdout) => {
        if (error) { rejectProcess(error); return; }
        try { resolve(JSON.parse(stdout) as ACL); } catch (failure) { reject(failure); }
      });
    child.stdin!.on('error', rejectProcess);
    child.stdin!.end(JSON.stringify({ path, action }));
  });
}
