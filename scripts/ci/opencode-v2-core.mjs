// An installed OpenCode 2 host owns the credential and issues the model request.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { once } from 'node:events';
import { execFileSync, spawn } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { approveBrowser, boundedBody, openCoreTls } from './managed-host-fixture.mjs';

const config = JSON.parse(await new Response(process.stdin).text());
const home = await mkdtemp(join(tmpdir(), 'opencode-core-'));
const bin = process.env.OPENCODE_V2_BIN;
const plugin = process.env.OPENCODE_V2_PLUGIN;
const certFile = process.env.FIXTURE_TLS_CERT;
const pause = ms => new Promise(done => setTimeout(done, ms));
let stage = 'binary version';
let host, tunnel, endpoint, password;
async function stop() {
  if (!host || host.exitCode !== null || host.signalCode !== null) return;
  const end = once(host, 'exit');
  host.kill('SIGTERM');
  await Promise.race([end, pause(2000)]);
  if (host.exitCode === null && host.signalCode === null) host.kill('SIGKILL');
  await end;
}
async function api(path, body) {
  const res = await fetch(endpoint + path, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { authorization: 'Basic ' + Buffer.from('opencode:' + password).toString('base64'),
      ...(body === undefined ? {} : {'content-type':'application/json'}) },
    body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(30000),
  });
  assert.ok(res.ok, 'native API HTTP ' + res.status);
  return res.status === 204 ? undefined : JSON.parse(await boundedBody(res, 2 * 1024 * 1024));
}
async function launch() {
  const listener = createServer();
  listener.listen(0, '127.0.0.1');
  await once(listener, 'listening');
  const port = listener.address().port;
  await new Promise(done => listener.close(done));
  endpoint = 'http://127.0.0.1:' + port;
  password = randomBytes(32).toString('hex');
  host = spawn(bin, ['serve','--print-logs','--hostname','127.0.0.1','--port',String(port)], {
    cwd: home, env: { PATH: process.env.PATH, HOME: home, XDG_CONFIG_HOME: join(home,'config'),
      XDG_DATA_HOME: join(home,'data'), XDG_STATE_HOME: join(home,'state'), XDG_CACHE_HOME: join(home,'cache'),
      TMPDIR: home, BROWSER: 'true', NODE_EXTRA_CA_CERTS: certFile, SSL_CERT_FILE: certFile,
      OPENCODE_SERVER_PASSWORD: password }, stdio: ['ignore','pipe','pipe'],
  });
  let tail = '';
  const observe = chunk => {
    tail = (tail + chunk).slice(-4096);
    const match = tail.match(/server password\s+(\S+)\s/);
    if (match) password = match[1];
  };
  host.stdout.on('data', observe);
  host.stderr.on('data', observe);
  for (let i=0; i<180 && host.exitCode===null; i++) {
    try { if (await api('/api/info')) return; } catch {}
    await pause(250);
  }
  throw new Error('native API unavailable');
}
async function models() {
  return (await api('/api/model')).data.filter(model => model.providerID === 'lmm');
}
async function soleModel() {
  for (let i=0; i<100; i++) {
    const rows = await models();
    if (rows.length) { assert.equal(rows.length,1); return rows[0]; }
    await pause(200);
  }
  throw new Error('no authorized model');
}
try {
  const version = execFileSync(bin, ['--version'], { encoding:'utf8', timeout:10000 }).trim();
  assert.match(version, /^(?:opencode v)?2\.1\.4$/);
  tunnel = await openCoreTls({ ...config, keyFile:process.env.FIXTURE_TLS_KEY, certFile });
  await mkdir(join(home,'config','opencode'),{recursive:true});
  await writeFile(join(home,'config','opencode','opencode.json'),JSON.stringify({plugins:[{package:plugin,options:{issuer:config.issuer}}]}));
  stage = 'native integration startup';
  await launch();
  assert.equal((await models()).length,0);
  const integration=(await api('/api/integration')).data.find(row=>row.id==='lmm');
  const method=integration.methods.find(row=>row.type==='oauth');
  const {data:attempt}=await api('/api/integration/lmm/connect/oauth',{methodID:method.id});
  const url=new URL(attempt.url);
  assert.equal(url.searchParams.get('client_id'),'lmm-opencode');
  assert.equal(url.searchParams.get('code_challenge_method'),'S256');
  stage='Core browser consent and host PKCE exchange';
  await approveBrowser(url,{issuer:config.issuer,refresh:config.refresh,cap:config.cap});
  let connected=false;
  for (let i=0; i<120; i++) {
    const {data}=await api('/api/integration/lmm/connect/oauth/'+encodeURIComponent(attempt.attemptID));
    if (data.status==='complete') { connected=true; break; }
    assert.equal(data.status,'pending'); await pause(100);
  }
  assert.ok(connected);
  const original=await soleModel();
  stage='restart and catalog discovery';
  await stop();
  await launch();
  const selected=await soleModel();
  assert.equal(selected.id,original.id);
  stage='native prompt';
  const {data:session}=await api('/api/session',{title:'Core OAuth settlement',model:{providerID:'lmm',id:selected.id}});
  const sid=encodeURIComponent(session.id);
  await api('/api/session/'+sid+'/prompt',{text:'Reply briefly.'});
  await api('/api/experimental/session/'+sid+'/wait',{});
  stage='native assistant terminal';
  const {data:messages}=await api('/api/session/'+sid+'/message?type=assistant&limit=100');
  assert.ok(messages.length>0 && messages.every(m=>!m.error));
  assert.equal(messages.flatMap(m=>m.content.filter(item=>item.type==='text').map(item=>item.text)).join(''),'local stream answer');
  assert.equal(tunnel.calls.length,1);
  const [call]=tunnel.calls;
  assert.equal(call.path,'/v1/chat/completions'); assert.equal(call.status,200);
  assert.ok(call.type.startsWith('text/event-stream') && call.body.includes('[DONE]'));
  assert.ok(typeof call.requestId==='string' && call.requestId.length>0 && call.requestId.length<=256);
  process.stdout.write(JSON.stringify({version:'2.1.4',requestId:call.requestId,modelId:selected.id,group:call.group,terminal:true}));
} catch {
  const requests=(tunnel?.calls??[]).map(({status,path})=>({status,path:['/v1/chat/completions','/v1/responses','/v1/messages'].includes(path)?path:'other'}));
  console.error('OpenCode V2 acceptance failed during '+stage+' '+JSON.stringify(requests));
  process.exitCode=1;
} finally {
  await stop(); await tunnel?.close(); await rm(home,{recursive:true,force:true});
}
