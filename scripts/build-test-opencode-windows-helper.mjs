import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {execFileSync, spawnSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';

// Independent control branch: immutable e1f plus the four test-oracle source overlays.
// This does not replace the original nine-row OpenCode qualification matrix.
const BASE = 'e1f65f4e4d3dee914b09025697e0530ff6b70188';
const TREE = '7aed4829a5bb19bd914f889fb1476f8354761246';
const INPUTS = 'scripts/opencode-windows-helper-inputs.json';
const SELF = 'scripts/build-test-opencode-windows-helper.mjs';
const EXE = 'test/windows-journal-acl/lmm-test-acl.exe';
const BUILD = 'test/windows-journal-acl/build.json';
const selected = [
  'rotation journal recovers the same operation without keeping secrets and rejects symlinks',
  'response loss and plugin restart recover the same rotation; usage reports lifetime reservations',
];
const [mode, productArg, controlArg, evidenceArg] = process.argv.slice(2);
const product = path.resolve(productArg || '.');
const control = path.resolve(controlArg || '.');
const evidence = path.resolve(evidenceArg || '.');
const hash = b => createHash('sha256').update(b).digest('hex');
const blob = b => createHash('sha1').update(Buffer.concat([Buffer.from('blob ' + b.length + '\0'), b])).digest('hex');
const readJSON = p => JSON.parse(fs.readFileSync(p, 'utf8'));
const qualification = 'WINDOWS_INDEPENDENT_ACL_ORACLE_AND_ORIGINAL_UNITS_ONLY';
function write(name, value) {
  fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2) + '\n', {flag: 'wx'});
}
function file(root, relative) {
  assert.match(relative, /^[A-Za-z0-9_./-]+$/);
  assert(!relative.startsWith('/') && !relative.split('/').includes('..'));
  const p = path.join(root, ...relative.split('/'));
  const stat = fs.lstatSync(p);
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.size <= 8 * 1024 * 1024);
  const b = fs.readFileSync(p);
  return {path: relative, bytes: b.length, sha256: hash(b), blob: blob(b)};
}
function git(root, args) {
  return execFileSync('git', ['-C', root, ...args], {encoding: 'utf8', timeout: 15000, maxBuffer: 1024 * 1024, windowsHide: true}).trim();
}
function expected(inputs, changed) {
  const entries = new Map(inputs.baseline.map(x => [x.path, {path: x.path, bytes: x.bytes, sha256: x.sha256, blob: x.blob}]));
  if (changed) for (const entry of inputs.overlays) {
    if (entry.operation === 'delete') entries.delete(entry.product_path);
    else entries.set(entry.product_path, {path: entry.product_path, ...entry.after});
  }
  return [...entries.values()].sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
}
function sourceRecords(inputs, changed = true) {
  return expected(inputs, changed).map(x => file(product, x.path));
}
function controlRecords(inputs) {
  return ['.gitattributes', '.github/workflows/core-protocol.yml', SELF, INPUTS,
    ...inputs.overlays.flatMap(x => x.control_path ? [x.control_path] : [])].map(p => file(control, p));
}
function checkSource(inputs) {
  assert.deepEqual(sourceRecords(inputs), expected(inputs, true));
  assert.deepEqual(controlRecords(inputs), readJSON(path.join(evidence, 'control-inputs.json')));
  const found = [];
  function walk(dir, prefix = '') {
    for (const item of fs.readdirSync(dir, {withFileTypes: true})) {
      if (!prefix && ['.git', 'node_modules'].includes(item.name)) continue;
      const relative = prefix + item.name;
      assert(!item.isSymbolicLink(), 'Unexpected source symlink');
      if (item.isDirectory()) walk(path.join(dir, item.name), relative + '/');
      else { assert(item.isFile()); found.push(relative); }
    }
  }
  walk(product);
  const generated = [EXE, BUILD].filter(p => fs.existsSync(path.join(product, p)));
  assert.deepEqual(found.sort(), [...expected(inputs, true).map(x => x.path), ...generated].sort());
}
function invariants(inputs) {
  assert.equal(inputs.product_sha, BASE); assert.equal(inputs.product_tree, TREE);
  assert.equal(inputs.baseline.length, 59);
  assert.equal(new Set(inputs.baseline.map(x => x.path)).size, 59);
  assert.equal(inputs.overlays.length, 4);
  assert.equal(process.env.GITHUB_EVENT_NAME, 'workflow_dispatch');
  assert.equal(process.env.DIAGNOSTIC_MODE, 'opencode_windows_helper');
  assert.equal(process.env.PRODUCT_SHA, BASE);
  assert.equal(process.platform, 'win32'); assert.equal(process.version, 'v22.23.5');
  assert.match(process.env.GITHUB_SHA || '', /^[a-f0-9]{40}$/);
  assert.equal(git(control, ['rev-parse', 'HEAD']), process.env.GITHUB_SHA);
  assert.equal(git(product, ['rev-parse', 'HEAD']), BASE);
  assert.equal(git(product, ['rev-parse', 'HEAD^{tree}']), TREE);
  const leaves = git(product, ['ls-tree', '-r', '--full-tree', 'HEAD']).split('\n').map(line => {
    const match = /^([0-9]{6}) blob ([a-f0-9]{40})\t(.+)$/.exec(line); assert(match);
    return {mode: match[1], blob: match[2], path: match[3]};
  });
  assert.deepEqual(leaves, inputs.baseline.map(x => ({mode: x.mode, blob: x.blob, path: x.path})));
}
function exited(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return false;
  try { process.kill(pid, 0); return false; } catch (error) { return error.code === 'ESRCH'; }
}
function command(label, executable, argv, timeout, options = {}) {
  write(label + '-command.json', {executable, argv, timeout_ms: timeout, cwd: product, invocation_count: 1});
  const began = performance.now();
  const result = spawnSync(executable, argv, {cwd: product, windowsHide: true, encoding: 'utf8',
    timeout, maxBuffer: 4 * 1024 * 1024, ...options});
  const stdout = result.stdout || '', stderr = result.stderr || '';
  fs.writeFileSync(path.join(evidence, label + '.stdout.log'), stdout, {flag: 'wx'});
  fs.writeFileSync(path.join(evidence, label + '.stderr.log'), stderr, {flag: 'wx'});
  const status = {exit_code: result.status, signal: result.signal || null, error_code: result.error?.code || null,
    pid: result.pid ?? null, process_exited: exited(result.pid), elapsed_ms: Math.floor(performance.now() - began)};
  write(label + '-process.json', status); // Save original status before assertions.
  return {result, status, stdout, stderr};
}
function succeeded(run) {
  assert.equal(run.status.error_code, null); assert.equal(run.status.signal, null);
  assert.equal(run.status.exit_code, 0); assert.equal(run.status.process_exited, true);
}
async function verifyBuild() {
  const module = await import(pathToFileURL(path.join(product, 'scripts/verify-windows-journal-build.mjs')).href);
  return module.verifyWindowsJournalBuild(product);
}
function counts(text) {
  return Object.fromEntries(['tests','pass','fail','cancelled','skipped','todo'].map(name => {
    const rows = [...text.matchAll(new RegExp('^# ' + name + ' ([0-9]+)$', 'gm'))];
    return [name, rows.length === 1 ? Number(rows[0][1]) : null];
  }));
}
let stage = ['prepare','build','compile','test','finalize'].includes(mode) ? mode : 'invalid';
try {
  assert.notEqual(stage, 'invalid'); assert.notEqual(evidence, product); assert.notEqual(evidence, control);
  const inputs = readJSON(path.join(control, INPUTS));
  if (mode === 'prepare') fs.mkdirSync(evidence, {recursive: false});
  assert(fs.statSync(evidence).isDirectory());
  if (mode === 'prepare') {
    write('identity-before.json', {node: process.version, platform: process.platform, arch: process.arch,
      control_sha: git(control, ['rev-parse', 'HEAD']), control_tree: git(control, ['rev-parse', 'HEAD^{tree}']),
      product_sha: git(product, ['rev-parse', 'HEAD']), product_tree: git(product, ['rev-parse', 'HEAD^{tree}'])});
    write('source-before-overlay.json', sourceRecords(inputs, false));
    write('control-inputs.json', controlRecords(inputs));
  }
  invariants(inputs);
  if (mode === 'prepare') {
    assert.deepEqual(sourceRecords(inputs, false), expected(inputs, false));
    for (const entry of inputs.overlays) {
      const target = path.join(product, entry.product_path);
      if (entry.operation === 'delete') fs.unlinkSync(target);
      else {
        assert.deepEqual(file(control, entry.control_path), {path: entry.control_path, ...entry.after});
        fs.mkdirSync(path.dirname(target), {recursive: true});
        fs.copyFileSync(path.join(control, entry.control_path), target);
      }
    }
    checkSource(inputs);
    assert(!fs.existsSync(path.join(product, EXE)) && !fs.existsSync(path.join(product, BUILD)));
    const core = file(product, inputs.csharp_core.path);
    assert.equal(core.bytes, 12846); assert.equal(core.sha256, inputs.csharp_core.sha256);
    await verifyBuild(); // Only validates the existing product; never executes or rebuilds its exe.
    write('source-ready.json', sourceRecords(inputs));
    write('preparation.json', {qualification, control_sha: process.env.GITHUB_SHA,
      control_tree: git(control, ['rev-parse', 'HEAD^{tree}']), product_sha: BASE, product_tree: TREE,
      node: process.version, platform: process.platform, arch: process.arch,
      runner_image: process.env.ImageOS || null, runner_image_version: process.env.ImageVersion || null,
      baseline_files: 59, effective_source_files: expected(inputs, true).length, overlays: inputs.overlays,
      helper_timeout_ms: 5000, helper_max_buffer: 4096, original_tests: 52, original_acl_calls: 7,
      selected_required_tests: selected, prewarm: false, retries: 0});
  } else {
    checkSource(inputs);
    if (mode === 'build') {
      const npm = path.join(path.dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js');
      const typescript = file(product, 'node_modules/typescript/package.json');
      assert.equal(readJSON(path.join(product, typescript.path)).version, '7.0.2');
      write('runtime-toolchain.json', {node: process.version, node_sha256: hash(fs.readFileSync(process.execPath)),
        npm_cli_sha256: hash(fs.readFileSync(npm)), typescript});
      succeeded(command('typescript-build', process.execPath, [npm, 'run', 'build'], 120000));
      checkSource(inputs); // Generated dist must equal the reviewed real tsc output.
      succeeded(command('typecheck', process.execPath, [npm, 'run', 'typecheck'], 120000));
      write('typescript-result.json', {status: 'PASS', actual_build: true, dist_matches_reviewed_overlay: true});
    } else if (mode === 'compile') {
      succeeded(command('windows-oracle-compile', process.execPath, ['test/build-windows-journal-acl.mjs'], 75000));
      succeeded(command('windows-oracle-verify', process.execPath, ['test/build-windows-journal-acl.mjs', '--verify'], 15000));
      write('oracle-generated-inputs.json', [EXE, BUILD].map(p => file(product, p)));
      write('unchanged-product-build-verification.json', await verifyBuild());
      checkSource(inputs);
    } else if (mode === 'test') {
      await verifyBuild();
      assert.deepEqual([EXE, BUILD].map(p => file(product, p)), readJSON(path.join(evidence, 'oracle-generated-inputs.json')));
      assert.equal(readJSON(path.join(product, 'package.json')).scripts.test,
        'node --experimental-strip-types --test test/*.test.ts');
      // First execution of the independent oracle and the unchanged product exe happens in the original suite.
      // No warm-up, altered concurrency, test filters or retries.
      const run = command('unit', process.execPath, ['--experimental-strip-types', '--test', 'test/*.test.ts'], 180000);
      const text = run.stdout.replace(/\r\n/g, '\n');
      const cases = [...text.matchAll(/^(not ok|ok) [0-9]+ - (.+)$/gm)].map(x => ({name: x[2], passed: x[1] === 'ok'}));
      const totals = counts(text);
      const required = selected.map(name => ({name, occurrences: cases.filter(x => x.name === name).length,
        passed: cases.filter(x => x.name === name).length === 1 && cases.find(x => x.name === name).passed}));
      write('unit-result.json', {qualification, ...run.status, counts: totals, cases, required_tests: required,
        invocation_count: 1, prewarm: false, retries: 0});
      succeeded(run);
      assert.deepEqual(totals, {tests: 52, pass: 52, fail: 0, cancelled: 0, skipped: 0, todo: 0});
      assert.equal(cases.length, 52); assert(cases.every(x => x.passed));
      assert(required.every(x => x.occurrences === 1 && x.passed));
      await verifyBuild();
    } else {
      // The workflow obtains an actual OS process inventory before this step.
      const processes = readJSON(path.join(evidence, 'helper-process-cleanup.json'));
      write('source-after.json', sourceRecords(inputs));
      const sourceStable = JSON.stringify(sourceRecords(inputs)) === JSON.stringify(readJSON(path.join(evidence, 'source-ready.json')));
      const controlStable = JSON.stringify(controlRecords(inputs)) === JSON.stringify(readJSON(path.join(evidence, 'control-inputs.json')));
      const generated = [];
      fs.mkdirSync(path.join(evidence, 'actual-build'), {recursive: false});
      for (const p of [EXE, BUILD]) if (fs.existsSync(path.join(product, p))) {
        const entry = file(product, p); generated.push(entry);
        fs.copyFileSync(path.join(product, p), path.join(evidence, 'actual-build', path.basename(p)));
      }
      const unit = fs.existsSync(path.join(evidence, 'unit-result.json')) ? readJSON(path.join(evidence, 'unit-result.json')) : null;
      const unitVerified = unit?.exit_code === 0 && JSON.stringify(unit.counts) === JSON.stringify({tests: 52, pass: 52, fail: 0, cancelled: 0, skipped: 0, todo: 0});
      const generatedStable = fs.existsSync(path.join(evidence, 'oracle-generated-inputs.json'))
        && JSON.stringify(generated) === JSON.stringify(readJSON(path.join(evidence, 'oracle-generated-inputs.json')));
      const failedStages = ['prepare','build','compile','test','finalize','invalid']
        .filter(name => fs.existsSync(path.join(evidence, 'control-' + name + '-failure.json')));
      const clean = processes.owned_helper_pids.length === 0 && processes.uninspectable_helper_count === 0
        && processes.owned_oracle_pids.length === 0 && processes.uninspectable_oracle_count === 0;
      write('final-receipt.json', {qualification, product_sha: BASE, product_tree: TREE, control_sha: process.env.GITHUB_SHA,
        source_stable: sourceStable, control_stable: controlStable, generated_files: generated, failed_stages: failedStages,
        helper_and_oracle_processes_exited: clean, generated_oracle_stable: generatedStable, original_units_verified: unitVerified,
        status: sourceStable && controlStable && clean && unitVerified && generatedStable && failedStages.length === 0 ? 'PASS_PENDING_ORIGINAL_NINE_ROW_MATRIX' : 'FAILED_OR_INCOMPLETE',
        helper_timeout_ms: 5000, helper_max_buffer: 4096, no_retry: true, no_prewarm: true});
      assert(sourceStable && controlStable && clean && unitVerified && generatedStable && failedStages.length === 0);
    }
  }
} catch {
  if (fs.existsSync(evidence) && fs.statSync(evidence).isDirectory()) {
    const target = path.join(evidence, 'control-' + stage + '-failure.json');
    if (!fs.existsSync(target)) fs.writeFileSync(target, JSON.stringify({stage, status: 'FAILED', qualification, original_logs_preserved: true}) + '\n');
  }
  console.error('windows_acl_oracle_control_failed', stage);
  process.exitCode = 1;
}
