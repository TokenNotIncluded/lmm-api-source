import assert from 'node:assert/strict';
import { once } from 'node:events';
import { readFile } from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';

export async function boundedBody(response, limit = 256 * 1024) {
  let length = 0;
  const chunks = [];
  for await (const chunk of response.body) {
    length += chunk.length;
    assert.ok(length <= limit, 'fixture response exceeded byte limit');
    chunks.push(Buffer.from(chunk));
  }
  return Buffer.concat(chunks).toString('utf8');
}

const hop = new Set(['connection', 'keep-alive', 'proxy-connection', 'te', 'trailer', 'transfer-encoding', 'upgrade']);
const endHeaders = headers => Object.fromEntries(Object.entries(headers).filter(([name]) => !hop.has(name)));

// TLS termination only. OAuth, catalogue, model responses and billing come from Core.
export async function openCoreTls({ coreBase, issuer, keyFile, certFile }) {
  const backend = new URL(coreBase);
  const authority = new URL(issuer);
  assert.equal(backend.protocol, 'http:');
  assert.equal(backend.hostname, '127.0.0.1');
  assert.equal(authority.protocol, 'https:');
  assert.ok(['pi-issuer.example.test', '127.0.0.1'].includes(authority.hostname));
  const calls = [];
  let access;
  const server = https.createServer({ key: await readFile(keyFile), cert: await readFile(certFile) }, (req, res) => {
    if (req.headers.host !== authority.host || !req.url.startsWith('/') || req.url.startsWith('//')) {
      res.writeHead(400).end();
      return;
    }
    const path = req.url.split('?')[0];
    const upstream = http.request({ hostname: backend.hostname, port: backend.port, method: req.method,
      path: req.url, headers: { ...endHeaders(req.headers), host: authority.host }, timeout: 20_000 }, reply => {
      const isToken = path === '/api/oauth2/token' && reply.statusCode === 200;
      const isModel = path.startsWith('/v1/') && req.method === 'POST';
      const record = isModel ? { method: req.method, path, status: reply.statusCode, requestId: reply.headers['x-request-id'],
        group: req.headers['x-lmm-group'], type: reply.headers['content-type'], body: '' } : undefined;
      if (record) calls.push(record);
      const keep = isToken || isModel;
      const chunks = [];
      let bytes = 0;
      if (keep) reply.on('data', chunk => {
        bytes += chunk.length;
        if (bytes > 256 * 1024) { reply.destroy(); res.destroy(); return; }
        chunks.push(chunk);
      });
      reply.on('error', () => res.destroy());
      reply.on('end', () => {
        if (!keep || bytes > 256 * 1024) return;
        const body = Buffer.concat(chunks).toString('utf8');
        if (record) record.body = body;
        if (isToken) {
          try { access = JSON.parse(body).access_token; } catch { access = undefined; }
        }
      });
      res.writeHead(reply.statusCode, endHeaders(reply.headers));
      reply.pipe(res);
    });
    upstream.on('error', () => { if (!res.headersSent) res.writeHead(502); res.end(); });
    upstream.on('timeout', () => upstream.destroy());
    req.on('aborted', () => upstream.destroy());
    req.pipe(upstream);
  });
  server.listen(Number(authority.port || 443), '127.0.0.1');
  await once(server, 'listening');
  return { calls, getAccess: () => access, close: async () => {
    const closed = once(server, 'close');
    server.close();
    server.closeAllConnections();
    await closed;
  }};
}

const normalizeCsrf = html => {
  const match = html.match(/name=csrf value="([^"]+)"/);
  assert.ok(match, 'Core consent form lacks CSRF');
  return match[1];
};
function binding(response) {
  const sessions = response.headers.getSetCookie().filter(cookie => cookie.includes('Max-Age=300'));
  assert.equal(sessions.length, 1, 'Core consent binding missing');
  return sessions[0].split(';')[0];
}

// The caller supplies the ordinary browser session freshly issued by the disposable Core.
export async function approveBrowser(url, { issuer, refresh, cap }) {
  const start = new URL(url);
  assert.equal(start.origin, issuer);
  assert.equal(start.pathname, '/api/oauth2/authorize');
  const expectedCallback = new URL(start.searchParams.get('redirect_uri'));
  assert.equal(expectedCallback.protocol, 'http:');
  assert.equal(expectedCallback.hostname, '127.0.0.1');
  assert.ok(Number.isSafeInteger(cap) && cap > 0);
  const opts = { redirect: 'manual', signal: AbortSignal.timeout(30_000) };
  const authorize = await fetch(start, opts);
  assert.equal(authorize.status, 200, 'Core authorize failed');
  const submit = (action, prev, csrf, extra = {}) => fetch(issuer + '/api/user/auth/oauth2/' + action, {
    ...opts, method: 'POST', headers: { origin: issuer, 'content-type': 'application/x-www-form-urlencoded', cookie: binding(prev) + '; new_api_refresh=' + refresh },
    body: new URLSearchParams({ csrf, ...extra }),
  });
  const form = await submit('continue', authorize, normalizeCsrf(await boundedBody(authorize)));
  assert.equal(form.status, 200, 'Core continue failed');
  const html = await boundedBody(form);
  assert.ok(html.includes('name=budget_credits'), 'Core did not offer a finite budget');
  const consent = await submit('consent', form, normalizeCsrf(html), { decision: 'allow', budget_credits: String(cap) });
  assert.equal(consent.status, 303, 'Core consent failed');
  const callback = new URL(consent.headers.get('location'));
  assert.equal(callback.origin, expectedCallback.origin);
  assert.equal(callback.pathname, expectedCallback.pathname);
  assert.ok(callback.searchParams.get('code') && callback.searchParams.get('state') === start.searchParams.get('state'));
  const done = await fetch(callback, opts);
  assert.equal(done.status, 200, 'host did not accept Core OAuth return');
  await boundedBody(done);
}
