import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { approveBrowser, boundedBody, openCoreTls } from './managed-host-fixture.mjs';

const input = [];
for await (const chunk of process.stdin) {
  input.push(chunk);
  assert.ok(Buffer.concat(input).length < 8192);
}
const { coreBase, issuer, refresh, cap } = JSON.parse(Buffer.concat(input));
const root = await mkdtemp(join(tmpdir(), 'magpie-gateway-'));
const processes = new Set();
let tunnel;
let phase = 'setup';
let env;

function start(args, inspect = () => {}) {
  const child = spawn(process.env.MAGPIE_BIN, args, { env, stdio: ['ignore', 'pipe', 'pipe'],
    detached: process.platform === 'linux' });
  processes.add(child);
  const deadline = setTimeout(() => stop(child), 90_000);
  let stdout = '', errSize = 0;
  child.stdout.on('data', bytes => {
    stdout += bytes.toString();
    if (Buffer.byteLength(stdout) > 1024 * 1024) { stop(child); return; }
    inspect(stdout);
  });
  child.stderr.on('data', bytes => {
    errSize += bytes.length;
    if (errSize > 1024 * 1024) stop(child);
  });
  const done = once(child, 'close').then(([code]) => {
    assert.equal(code, 0, 'Magpie process failed at ' + phase);
    return stdout;
  }).finally(() => {clearTimeout(deadline); processes.delete(child);});
  done.catch(() => {});
  return { child, done };
}
function stop(child) {
  if (child.exitCode !== null || !child.pid) return;
  try { process.kill(-child.pid, 'SIGKILL'); } catch { child.kill('SIGKILL'); }
}
const parse = async response => {
  assert.equal(response.status, 200, 'unexpected HTTP status at ' + phase);
  return JSON.parse(await boundedBody(response));
};
const core = path => fetch(issuer + path, { headers: { authorization: 'Bearer ' + tunnel.getAccess() },
  signal: AbortSignal.timeout(10_000), redirect: 'error' }).then(parse);

function chat(body, type) {
  if (!type?.includes('text/event-stream')) {
    assert.ok(type?.includes('application/json'));
    const data = JSON.parse(body);
    assert.equal(data.choices.length, 1);
    return { text: data.choices[0].message.content, stop: data.choices[0].finish_reason, usage: data.usage };
  }
  let text = '', stop, usage, done = false;
  for (const event of body.replaceAll('\r\n', '\n').split('\n\n')) {
    const payload = event.split('\n').filter(line => line.startsWith('data:')).map(line => line.slice(5).trimStart()).join('\n');
    if (!payload) continue;
    assert.ok(!done, 'SSE data followed its terminal marker');
    if (payload === '[DONE]') { done = true; continue; }
    const row = JSON.parse(payload);
    assert.ok(!row.error && Array.isArray(row.choices));
    for (const choice of row.choices) {
      assert.equal(choice.index, 0);
      text += choice.delta?.content ?? '';
      if (choice.finish_reason) { assert.ok(!stop); stop = choice.finish_reason; }
    }
    if (row.usage) usage = row.usage;
  }
  assert.ok(done, 'missing SSE terminal marker');
  return { text, stop, usage };
}

try {
  await Promise.all(['home', 'config', 'data', 'state', 'cache'].map(name => mkdir(join(root, name), { mode: 0o700 })));
  const portFinder = net.createServer();
  portFinder.listen(0, '127.0.0.1');
  await once(portFinder, 'listening');
  const { port } = portFinder.address();
  await new Promise(resolve => portFinder.close(resolve));
  const base = 'http://127.0.0.1:' + port;
  env = {
    PATH: process.env.PATH, HOME: join(root, 'home'), XDG_CONFIG_HOME: join(root, 'config'),
    XDG_DATA_HOME: join(root, 'data'), XDG_STATE_HOME: join(root, 'state'), XDG_CACHE_HOME: join(root, 'cache'),
    MAGPIE_BUN: process.env.MAGPIE_BUN, NODE_EXTRA_CA_CERTS: process.env.FIXTURE_TLS_CERT,
    // This private home contains only the fixture's synthetic OAuth account.
    // Magpie suppresses even signed-in OAuth plugins when account discovery is off.
    DO_NOT_TRACK: '1', NO_COLOR: '1', BROWSER: '/bin/true',
    MAGPIE_ADDR: '127.0.0.1:' + port,
  };
  tunnel = await openCoreTls({ coreBase, issuer, keyFile: process.env.FIXTURE_TLS_KEY, certFile: process.env.FIXTURE_TLS_CERT });
  const plugin = new URL('../../packages/opencode-lmm-auth/dist/magpie.js', import.meta.url);
  const entry = join(root, 'lmm-magpie.js');
  // The official entry's issuer option is the sole change to the deployed plugin.
  await writeFile(entry, 'import plugin from ' + JSON.stringify(plugin.href) + ';\nexport default input => plugin(input, {issuer: ' + JSON.stringify(issuer) + '});\n');

  phase = 'plugin add';
  await start(['plugin', 'add', entry]).done;
  phase = 'signin';
  let seeUrl;
  const link = new Promise(resolve => {seeUrl = resolve;});
  const login = start(['plugin', 'login', 'lmm-magpie'], stdout => {
    const line = stdout.replace(/\u001b\[[0-9;]*m/g, '').split(/\r?\n/).find(l => l.trim().startsWith(issuer + '/api/oauth2/authorize?'));
    if (line && /\r?\n/.test(stdout.slice(stdout.indexOf('/api/oauth2/authorize?')))) seeUrl(line.trim());
  });
  const authUrl = await Promise.race([link, login.done.then(() => {throw new Error('login ended before authorization URL');})]);
  assert.equal(new URL(authUrl).searchParams.get('client_id'), 'lmm-magpie');
  await approveBrowser(authUrl, { issuer, refresh, cap });
  await login.done;
  assert.ok(tunnel.getAccess(), 'Core token was not issued');

  phase = 'authorized catalogue';
  const budget0 = await core('/api/oauth2/application/budget');
  assert.deepEqual([budget0.budget_credits, budget0.consumed_credits, budget0.reserved_credits], [cap, 0, 0]);
  const catalogue = await core('/api/oauth2/catalog');
  assert.equal(catalogue.models.length, 1);
  const route = catalogue.models[0];
  const host = JSON.parse(await start(['plugin', 'list', '--json']).done);
  const provider = host.providers.find(p => p.id === 'lmm-magpie');
  assert.ok(provider?.signedIn);
  assert.deepEqual(provider.models.map(model => model.id), [route.id]);

  phase = 'gateway startup';
  const serve = start(['serve']);
  let ready = false;
  for (let attempt = 0; attempt < 100; attempt++) {
    if (serve.child.exitCode !== null) throw new Error('Magpie serve exited');
    try {
      const info = await parse(await fetch(base + '/api/hello', { signal: AbortSignal.timeout(500) }));
      if (info.name === 'magpie') { ready = true; break; }
    } catch {}
    await delay(150);
  }
  assert.ok(ready, 'real Magpie gateway did not become ready');
  const headers = { authorization: 'Bearer magpie', 'content-type': 'application/json' };
  const offered = await parse(await fetch(base + '/v1/models', { headers, signal: AbortSignal.timeout(15_000) }));
  const ids = offered.data.map(m => m.id).filter(id => id === provider.id + '/' + route.id);
  assert.equal(ids.length, 1, 'authorized model is missing or ambiguous on Magpie HTTP');
  assert.equal(tunnel.calls.length, 0, 'read-only discovery submitted a model request');
  const cases = [];
  for (const stream of [false, true]) {
    phase = stream ? 'gateway SSE' : 'gateway JSON';
    const response = await fetch(base + '/v1/chat/completions', { method: 'POST', headers, redirect: 'error',
      signal: AbortSignal.timeout(20_000), body: JSON.stringify({ model: ids[0], messages: [{role: 'user', content: 'hello'}],
        max_tokens: 20, stream, ...(stream ? {stream_options: {include_usage: true}} : {}) }) });
    assert.equal(response.status, 200);
    const type = response.headers.get('content-type');
    assert.ok(type?.includes(stream ? 'text/event-stream' : 'application/json'));
    const seen = chat(await boundedBody(response), type);
    assert.equal(tunnel.calls.length, cases.length + 1, 'missing or repeated upstream dispatch');
    const forwarded = tunnel.calls.at(-1);
    assert.deepEqual([forwarded.path, forwarded.status, forwarded.group], ['/v1/chat/completions', 200, route.group_id]);
    assert.ok(forwarded.requestId, 'Core request id missing');
    const original = chat(forwarded.body, forwarded.type);
    assert.ok(original.text === 'local synthetic answer' || original.text === 'local stream answer');
    assert.equal(seen.text, original.text, 'host lost or rewrote model output');
    assert.equal(seen.stop, 'stop');
    assert.equal(original.stop, seen.stop);
    for (const field of ['prompt_tokens', 'completion_tokens']) {
      assert.equal(seen.usage?.[field], original.usage?.[field], 'host changed ' + field);
    }
    assert.equal(seen.usage?.prompt_tokens_details?.cached_tokens, original.usage?.prompt_tokens_details?.cached_tokens);
    assert.equal(seen.usage?.prompt_tokens, 10);
    assert.equal(seen.usage?.completion_tokens, 20);
    cases.push({ stream, request_id: forwarded.requestId });
  }
  assert.equal(new Set(cases.map(c => c.request_id)).size, 2);
  const budget = await core('/api/oauth2/application/budget');
  assert.deepEqual([budget.budget_credits, budget.consumed_credits, budget.reserved_credits, budget.remaining_credits], [cap, 98, 0, cap - 98]);
  process.stdout.write(JSON.stringify({ cases, grant_id: budget.grant_id, model: route.upstream_model, group: route.group_id, budget: cap }) + '\n');
} catch (err) {
  // Keep OAuth codes, session and bearer credentials out of Actions logs even on failure.
  console.error('Magpie acceptance failed at ' + phase + ' (' + (err?.code ?? err?.name ?? 'error') + '): ' +
    (err?.code === 'ERR_ASSERTION' ? String(err.message).split('\n')[0] : 'operation did not complete'));
  process.exitCode = 1;
} finally {
  for (const child of processes) stop(child);
  if (tunnel) await tunnel.close();
  await rm(root, { recursive: true, force: true });
}
