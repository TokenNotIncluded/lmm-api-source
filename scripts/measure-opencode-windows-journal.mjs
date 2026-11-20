import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {execFileSync, spawnSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import {performance} from 'node:perf_hooks';

// Diagnostic control only. Product source and all security/test decisions are fixed.
const BASE = 'f2d2386da2663d3972f8c4bd71f7f0e7bd6c2490';
const TREE = '760843d322fb56285ee0e805d11237648f37c076';
const INPUTS = 'scripts/opencode-windows-journal-inputs.json';
const selected = [
  'rotation journal recovers the same operation without keeping secrets and rejects symlinks',
  'response loss and plugin restart recover the same rotation; usage reports lifetime reservations',
];
const phases = ['script_enter', 'input_ready', 'compile_begin', 'compile_end', 'storage_begin', 'storage_end', 'result_ready'];
const [mode, productArg, controlArg, evidenceArg] = process.argv.slice(2);
const product = path.resolve(productArg || '.');
const control = path.resolve(controlArg || '.');
const evidence = path.resolve(evidenceArg || '.');
const hash = b => createHash('sha256').update(b).digest('hex');
const blob = b => createHash('sha1').update(Buffer.concat([Buffer.from('blob ' + b.length + '\0'), b])).digest('hex');
const readJSON = p => JSON.parse(fs.readFileSync(p, 'utf8'));
function write(name, value) {
  fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2) + '\n', {flag: 'wx'});
}
function file(root, relative) {
  assert.match(relative, /^[A-Za-z0-9_./-]+$/);
  assert(!relative.startsWith('/') && !relative.split('/').includes('..'));
  const resolved = path.join(root, ...relative.split('/'));
  const stat = fs.lstatSync(resolved);
  assert(stat.isFile() && !stat.isSymbolicLink());
  const bytes = fs.readFileSync(resolved);
  return {path: relative, bytes: bytes.length, sha256: hash(bytes), blob: blob(bytes)};
}
function git(root, args) {
  return execFileSync('git', ['-C', root, ...args], {encoding: 'utf8', timeout: 15000, maxBuffer: 1024 * 1024, windowsHide: true}).trim();
}
function manifest(inputs) {
  return inputs.baseline.map(entry => file(product, entry.path));
}
function expected(inputs, overlay) {
  return inputs.baseline.map(entry => {
    const changed = overlay ? inputs.overlays.find(x => x.product_path === entry.path) : undefined;
    return {path: entry.path, bytes: changed ? changed.after.bytes : entry.bytes,
      sha256: changed ? changed.after.sha256 : entry.sha256, blob: changed ? changed.after.blob : entry.blob};
  });
}
function invariants(inputs) {
  assert.equal(inputs.product_sha, BASE);
  assert.equal(inputs.product_tree, TREE);
  assert.equal(inputs.baseline.length, 53);
  assert.equal(new Set(inputs.baseline.map(x => x.path)).size, 53);
  assert.deepEqual(inputs.overlays.map(x => x.product_path).sort(), [
    'dist/magpie-windows-journal.js', 'src/magpie-windows-journal.ps1', 'src/magpie-windows-journal.ts',
  ]);
  assert.equal(process.env.GITHUB_EVENT_NAME, 'workflow_dispatch');
  assert.equal(process.env.DIAGNOSTIC_MODE, 'opencode_windows_journal');
  assert.equal(process.env.PRODUCT_SHA, BASE);
  assert.match(process.env.GITHUB_SHA || '', /^[0-9a-f]{40}$/);
  assert.equal(git(control, ['rev-parse', 'HEAD']), process.env.GITHUB_SHA);
  assert.equal(git(product, ['rev-parse', 'HEAD']), BASE);
  assert.equal(git(product, ['rev-parse', 'HEAD^{tree}']), TREE);
  assert.equal(process.platform, 'win32');
  assert.equal(process.version, 'v22.23.5');
  const leaves = git(product, ['ls-tree', '-r', '--full-tree', 'HEAD']).split('\n').map(line => {
    const match = /^([0-9]{6}) blob ([0-9a-f]{40})\t(.+)$/.exec(line);
    assert(match);
    return {mode: match[1], blob: match[2], path: match[3]};
  });
  assert.deepEqual(leaves, inputs.baseline.map(x => ({mode: x.mode, blob: x.blob, path: x.path})));
}
function controlRecords(inputs) {
  return ['.github/workflows/core-protocol.yml', 'scripts/measure-opencode-windows-journal.mjs', INPUTS,
    ...inputs.overlays.map(x => x.control_path)].map(p => file(control, p));
}
function parseEvents(text) {
  const events = [];
  for (const line of text.split(/\r?\n/)) {
    const marker = 'lmm_windows_journal_phase_measurement ';
    const index = line.indexOf(marker);
    if (index < 0) continue;
    const event = JSON.parse(line.slice(index + marker.length));
    assert.deepEqual(Object.keys(event).sort(), ['complete','elapsed_ms','incomplete_line','invalid_lines','operation','overflow','parser_errors','phases','process_failed'].sort());
    assert(['pending','begin','other'].includes(event.operation));
    for (const key of ['elapsed_ms','invalid_lines','parser_errors']) assert(Number.isSafeInteger(event[key]) && event[key] >= 0 && event[key] <= 180000);
    for (const key of ['complete','incomplete_line','overflow','process_failed']) assert.equal(typeof event[key], 'boolean');
    assert(Array.isArray(event.phases) && event.phases.length <= 7);
    let priorChild = 0, priorParent = 0;
    for (const [i, item] of event.phases.entries()) {
      assert.deepEqual(Object.keys(item).sort(), ['phase','child_elapsed_ms','observed_parent_elapsed_ms'].sort());
      assert.equal(item.phase, phases[i]);
      assert(Number.isSafeInteger(item.child_elapsed_ms) && item.child_elapsed_ms >= priorChild && item.child_elapsed_ms <= 60000);
      assert(Number.isSafeInteger(item.observed_parent_elapsed_ms) && item.observed_parent_elapsed_ms >= priorParent && item.observed_parent_elapsed_ms <= 180000);
      priorChild = item.child_elapsed_ms; priorParent = item.observed_parent_elapsed_ms;
    }
    assert.equal(event.complete, event.phases.length === 7);
    events.push(event);
  }
  return events;
}
function resultCases(text) {
  return text.split(/\r?\n/).flatMap(line => {
    const match = /^(not ok|ok) [0-9]+ - (.+)$/.exec(line);
    return match && selected.includes(match[2]) ? [{name: match[2], passed: match[1] === 'ok'}] : [];
  });
}
function counters(text) {
  return Object.fromEntries(['tests','pass','fail','cancelled','skipped','todo'].map(name => {
    const matches = [...text.matchAll(new RegExp('^# ' + name + ' ([0-9]+)$', 'gm'))];
    return [name, matches.length === 1 ? Number(matches[0][1]) : null];
  }));
}
let current = ['prepare','run','finalize'].includes(mode) ? mode : 'invalid';
try {
  assert(['prepare','run','finalize'].includes(mode));
  assert.notEqual(evidence, product);
  assert.notEqual(evidence, control);
  const inputs = readJSON(path.join(control, INPUTS));
  if (mode === 'prepare') fs.mkdirSync(evidence, {recursive: false});
  assert(fs.statSync(evidence).isDirectory());
  if (mode === 'prepare') {
    write('identity-before.json', {node: process.version, platform: process.platform, arch: process.arch,
      control_sha: git(control, ['rev-parse', 'HEAD']), control_tree: git(control, ['rev-parse', 'HEAD^{tree}']),
      product_sha: git(product, ['rev-parse', 'HEAD']), product_tree: git(product, ['rev-parse', 'HEAD^{tree}'])});
    write('source-before-overlay.json', manifest(inputs));
    write('control-inputs.json', controlRecords(inputs));
  }
  invariants(inputs);
  if (mode === 'prepare') {
    const before = manifest(inputs);
    assert.deepEqual(before, expected(inputs, false));
    const controls = controlRecords(inputs);
    assert.deepEqual(controls, readJSON(path.join(evidence, 'control-inputs.json')));
    for (const entry of inputs.overlays) {
      assert.deepEqual(file(control, entry.control_path), {path: entry.control_path, ...entry.after});
      fs.copyFileSync(path.join(control, entry.control_path), path.join(product, entry.product_path));
    }
    const after = manifest(inputs);
    assert.deepEqual(after, expected(inputs, true));
    const csharp = fs.readFileSync(path.join(product, 'src/magpie-windows-journal.ps1'), 'utf8').split("$source = @'\n")[1].split("\n'@")[0];
    assert.equal(hash(Buffer.from(csharp)), inputs.csharp_sha256);
    write('source-ready.json', after);
    write('preparation.json', {
      qualification: 'DIAGNOSTIC_ONLY_NOT_PRODUCT_QUALIFICATION', control_sha: process.env.GITHUB_SHA,
      control_tree: git(control, ['rev-parse', 'HEAD^{tree}']), product_sha: BASE, product_tree: TREE,
      node: process.version, platform: process.platform, arch: process.arch,
      runner_image: process.env.ImageOS || null, runner_image_version: process.env.ImageVersion || null,
      original_helper_timeout_ms: 5000, original_helper_max_buffer: 4096,
      product_files: 53, overlays: inputs.overlays, selected_tests: selected, prewarm: false, retries: 0,
    });
  } else if (mode === 'run') {
    assert.deepEqual(manifest(inputs), readJSON(path.join(evidence, 'source-ready.json')));
    assert.deepEqual(controlRecords(inputs), readJSON(path.join(evidence, 'control-inputs.json')));
    const typescript = readJSON(path.join(product, 'node_modules/typescript/package.json'));
    write('runtime-toolchain.json', {node: process.version, node_sha256: hash(fs.readFileSync(process.execPath)),
      typescript: typescript.version, typescript_manifest_sha256: hash(fs.readFileSync(path.join(product, 'node_modules/typescript/package.json')))});
    assert.equal(typescript.version, '7.0.2');
    const escaped = selected.map(name => name.replace(/[\\^$.*+?()[\]{}|]/g, '\\$&'));
    const argv = ['--experimental-strip-types', '--test', '--test-name-pattern=^(' + escaped.join('|') + ')$', 'test/magpie.test.ts'];
    write('test-command.json', {executable: process.execPath, argv, cwd: 'immutable-f2d-product-with-three-declared-overlays', invocation_count: 1, timeout_ms: 180000, helper_timeout_ms: 5000});
    const began = performance.now();
    const result = spawnSync(process.execPath, argv, {
      cwd: product, encoding: 'utf8', windowsHide: true,
      timeout: 180000, maxBuffer: 2 * 1024 * 1024,
    });
    const elapsed = Math.floor(performance.now() - began);
    const stdout = result.stdout || '', stderr = result.stderr || '';
    fs.writeFileSync(path.join(evidence, 'unit.stdout.log'), stdout, {flag: 'wx'});
    fs.writeFileSync(path.join(evidence, 'unit.stderr.log'), stderr, {flag: 'wx'});
    const text = stdout + '\n' + stderr;
    // Persist original status/logs before any evidence completeness assertion.
    write('process-result.json', {exit_code: result.status, signal: result.signal || null, error_code: result.error?.code || null, elapsed_ms: elapsed, pid: result.pid});
    const events = parseEvents(text), cases = resultCases(stdout), counts = counters(stdout.replace(/\r\n/g, '\n'));
    const traceIntegrity = events.every(event => !event.overflow && !event.incomplete_line
      && event.invalid_lines === 0 && event.parser_errors === 0
      && (event.process_failed ? event.phases.length > 0 : event.complete));
    write('safe-phase-events.json', events);
    write('result.json', {qualification: 'DIAGNOSTIC_ONLY_NOT_PRODUCT_QUALIFICATION',
      source: BASE, exit_code: result.status, signal: result.signal || null, cases, counts,
      phase_event_count: events.length, trace_integrity: traceIntegrity,
      selected_once: cases.length === 2 && selected.every(name => cases.filter(x => x.name === name).length === 1),
      elapsed_ms: elapsed, retries: 0, prewarm: false,
      interpretation: 'Any failure remains a failure; a passing measurement is not a root-cause fix.'});
    assert.equal(cases.length, 2);
    assert(selected.every(name => cases.filter(x => x.name === name).length === 1));
    assert.equal(counts.tests, 2);
    assert.equal(counts.skipped, 0);
    assert.equal(counts.todo, 0);
    assert(events.length > 0);
    if (result.status !== 0) process.exitCode = result.status || 1;
    else {
      assert.equal(counts.pass, 2); assert.equal(counts.fail, 0); assert.equal(counts.cancelled, 0);
      assert(cases.every(x => x.passed));
      assert(traceIntegrity);
    }
  } else {
    const after = manifest(inputs);
    write('source-after.json', after);
    const unchanged = JSON.stringify(after) === JSON.stringify(readJSON(path.join(evidence, 'source-ready.json')));
    const controlUnchanged = JSON.stringify(controlRecords(inputs)) === JSON.stringify(readJSON(path.join(evidence, 'control-inputs.json')));
    write('final-source-receipt.json', {product_sha: BASE, control_sha: process.env.GITHUB_SHA, source_stable: unchanged, control_stable: controlUnchanged, helper_budget_ms: 5000, no_retry: true, qualification: 'DIAGNOSTIC_ONLY'});
    assert(unchanged && controlUnchanged);
  }
} catch {
  // Do not print arbitrary subprocess errors, command text, private paths or buffers.
  if (fs.existsSync(evidence) && fs.statSync(evidence).isDirectory()) {
    const target = path.join(evidence, 'control-' + current + '-failure.json');
    if (!fs.existsSync(target)) fs.writeFileSync(target, JSON.stringify({stage: current, status: 'FAILED', original_logs_preserved: true, qualification: 'DIAGNOSTIC_ONLY'}) + '\n');
  }
  console.error('windows_journal_measurement_control_failed', current);
  process.exitCode = 1;
}
