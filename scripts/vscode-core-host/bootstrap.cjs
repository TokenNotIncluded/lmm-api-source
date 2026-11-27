// Loaded only in the disposable extension-development copy of the built VSIX.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const manifest = require('./package.json');
const fixture = JSON.parse(fs.readFileSync(path.join(__dirname, '../fixture.json')));
const { ISSUER } = require('./dist/protocol');
const { parseToken } = require('./dist/auth');
const { MODELS_DEV_URL } = require('./dist/models-dev-cache');
const vscode = require('vscode');

exports.activate = async function activate(context) {
  assert.equal(context.extensionMode, vscode.ExtensionMode.Test);
  const base = new URL(fixture.base);
  assert.equal(base.hostname, '127.0.0.1');
  assert.equal(base.protocol, 'http:');
  assert.ok(base.port);
  const fetch = globalThis.fetch.bind(globalThis);
  // Keep the published issuer and resource binding. Only the network destination
  // changes, and every model/catalog/balance response comes from the real Core.
  const routes = new Set(['/api/oauth2/catalog', '/api/oauth2/balance', '/v1/chat/completions']);
  globalThis.fetch = (input, init) => {
    const url = new URL(input instanceof Request ? input.url : input);
    if (url.href === MODELS_DEV_URL) return Promise.reject(new Error('External metadata offline'));
    assert.equal(url.origin, ISSUER, 'unexpected test network origin');
    assert.ok(routes.has(url.pathname) && !url.search, 'unexpected test request path');
    return fetch(new URL(url.pathname, base), { ...init, redirect: 'error' });
  };
  await context.secrets.store('lmm.oauth.v1', JSON.stringify(parseToken(fixture.token)));
  return require(manifest.fixtureOriginalMain).activate(context);
};
