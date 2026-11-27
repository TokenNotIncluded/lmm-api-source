// Run the published LMM companion and the real CodeWhale executable against disposable Core.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { approveBrowser, openCoreTls } from './managed-host-fixture.mjs';

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const config = JSON.parse(Buffer.concat(chunks).toString('utf8'));
const home = await mkdtemp(join(tmpdir(), 'cw-core-'));
const issuer = new URL(config.issuer);
const env = {
  PATH: process.env.PATH, HOME: home, USERPROFILE: home, CODEWHALE_HOME: home,
  LANG: 'C.UTF-8', NO_COLOR: '1', LMM_CODEWHALE_HOME: join(home, 'credentials'),
  LMM_ISSUER: issuer.origin, LMM_CODEWHALE_BIN: process.env.LMM_CODEWHALE_BIN,
  LMM_MODELS_DEV_CACHE: join(home, 'metadata.json'), NODE_EXTRA_CA_CERTS: process.env.FIXTURE_TLS_CERT,
  HTTP_PROXY: 'http://127.0.0.1:1', HTTPS_PROXY: 'http://127.0.0.1:1',
  ALL_PROXY: 'http://127.0.0.1:1', NO_PROXY: '127.0.0.1,localhost,' + issuer.hostname,
  NODE_USE_ENV_PROXY: '1',
};
let tunnel;
let stage = 'official executable version';
const children = new Set();

function launch(binary, args, onStderrLine) {
  return new Promise((resolve, reject) => {
    const child = execFile(binary, args, { cwd: home, env, timeout: 75_000, maxBuffer: 512 * 1024 }, (error, stdout) => {
      children.delete(child);
      if (error) reject(new Error('host command failed'));
      else resolve(stdout);
    });
    children.add(child);
    if (onStderrLine) createInterface({ input: child.stderr }).on('line', onStderrLine);
  });
}
function companion(args, onStderrLine) {
  return launch(process.execPath, [process.env.CODEWHALE_ADAPTER_CLI, ...args], onStderrLine);
}

try {
  assert.match(await launch(env.LMM_CODEWHALE_BIN, ['--version']), /\b0\.10\.0\b/);
  tunnel = await openCoreTls({ ...config, keyFile: process.env.FIXTURE_TLS_KEY, certFile: process.env.FIXTURE_TLS_CERT });
  // The optional public metadata client can use its fresh local cache. Account admission still comes from Core.
  await writeFile(env.LMM_MODELS_DEV_CACHE, JSON.stringify({ version: 1, source: 'https://models.dev/api.json?type=all',
    fetchedAt: Date.now(), data: { fixture: { models: { [randomUUID()]: { limit: { context: 100, output: 20 },
      modalities: { input: ['text'], output: ['text'] }, reasoning: false, tool_call: false } } } } }), { mode: 0o600 });
  stage = 'companion PKCE and ordinary Core browser consent';
  const authorization = Promise.withResolvers();
  const login = companion(['login', '--no-browser'], line => {
    if (line.startsWith(issuer.origin + '/api/oauth2/authorize?')) authorization.resolve(new URL(line));
  });
  const url = await Promise.race([authorization.promise, login.then(() => { throw new Error('authorization not offered'); })]);
  assert.equal(url.searchParams.get('client_id'), 'lmm-codewhale');
  assert.equal(url.searchParams.get('code_challenge_method'), 'S256');
  await approveBrowser(url, config);
  await login;
  assert.ok(tunnel.getAccess());
  stage = 'persisted grant and account model discovery';
  const models = JSON.parse(await companion(['models']));
  assert.equal(models.length, 1);
  const [model] = models;
  assert.equal(typeof model.id, 'string');
  stage = 'published gateway output ceiling';
  const gatewayLimit = model.metadata?.gateway_max_output_tokens;
  assert.ok(Number.isSafeInteger(gatewayLimit) && gatewayLimit > 0);
  const modelLimit = model.metadata?.max_output_tokens;
  const outputLimit = Number.isSafeInteger(modelLimit) && modelLimit > 0 ? Math.min(modelLimit, gatewayLimit) : gatewayLimit;
  env.CODEWHALE_MAX_OUTPUT_TOKENS = String(outputLimit);
  stage = 'official one-shot exec';
  const terminal = await companion(['run', '--model', model.id, '--', 'exec', '--output-format', 'text', 'Reply briefly.']);
  stage = 'official host terminal and same-request Core response';
  assert.equal(tunnel.calls.length, 1);
  const [call] = tunnel.calls;
  assert.equal(call.path, '/v1/chat/completions');
  assert.equal(call.status, 200);
  assert.equal(call.group, model.group_id);
  assert.ok(typeof call.requestId === 'string' && call.requestId.length > 0 && call.requestId.length <= 256);
  const streaming = call.type?.startsWith('text/event-stream');
  let expected;
  if (streaming) {
    assert.ok(call.body.includes('[DONE]'));
    expected = 'local stream answer';
  } else {
    assert.ok(call.type?.startsWith('application/json'));
    const choice = JSON.parse(call.body).choices[0];
    assert.equal(choice.finish_reason, 'stop');
    expected = 'local synthetic answer';
  }
  assert.ok(terminal.split(/\r?\n/).some(line => line.trim() === expected));
  process.stdout.write(JSON.stringify({ version: '0.10.0', requestId: call.requestId, modelId: model.id, group: call.group, terminal: true }));
} catch {
  const requests = (tunnel?.calls ?? []).map(({ status, path, body }) => {
    let reason = 'unknown';
    try {
      const message = JSON.parse(body)?.error?.message;
      if (message === 'invalid_request: invalid output token limit') reason = 'output_limit';
      if (message === 'relay_limit_exceeded: local model input budget') reason = 'input_limit';
      if (message === 'unsupported_feature: request contains an unsupported field') reason = 'unsupported_field';
    } catch { /* Preserve unknown responses without exposing the body. */ }
    return { status, chat: path === '/v1/chat/completions', reason };
  });
  console.error('CodeWhale acceptance failed during ' + stage + ' ' + JSON.stringify(requests));
  process.exitCode = 1;
} finally {
  for (const child of children) child.kill('SIGTERM');
  await tunnel?.close();
  await rm(home, { recursive: true, force: true });
}
