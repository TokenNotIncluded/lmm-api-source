// SPDX-License-Identifier: AGPL-3.0-or-later
// The only inference caller is Zed's native Inline Assistant.
import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { once } from 'node:events'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { setTimeout as pause } from 'node:timers/promises'
import { promisify } from 'node:util'
import { approveBrowser, openCoreTls } from '../ci/managed-host-fixture.mjs'

const run = promisify(execFile)
const input = []
for await (const part of process.stdin) input.push(part)
const { coreBase, issuer, refresh, cap } = JSON.parse(Buffer.concat(input))
const root = await mkdtemp(join(tmpdir(), 'lmm-zed-'))
const config = join(root, 'config', 'zed')
const state = join(root, 'state')
const env = { ...process.env, HOME: root, XDG_CONFIG_HOME: join(root, 'config'),
  XDG_DATA_HOME: join(root, 'data'), XDG_STATE_HOME: state, XDG_CACHE_HOME: join(root, 'cache'),
  LMM_MODELS_DEV_CACHE: join(root, 'metadata.json'), ZED_ALLOW_EMULATED_GPU: '1',
  NODE_EXTRA_CA_CERTS: process.env.FIXTURE_TLS_CERT, BROWSER: '/bin/true',
  NO_PROXY: '127.0.0.1,localhost,pi-issuer.example.test', no_proxy: '127.0.0.1,localhost,pi-issuer.example.test',
  HTTP_PROXY: 'http://127.0.0.1:1', HTTPS_PROXY: 'http://127.0.0.1:1', ALL_PROXY: 'http://127.0.0.1:1' }
const execute = (binary, args) => run(binary, args, { env, cwd: root, timeout: 15000, maxBuffer: 256 * 1024 })
const bridgeBin = process.env.ZED_BRIDGE_BIN
const cliargs = cmd => [cmd, '--issuer', issuer]
const key = (...args) => execute('xdotool', ['key', '--clearmodifiers', ...args])
let tunnel, login, bridge
let stage = 'private host setup'

async function until(check, timeout = 30000) {
  const end = Date.now() + timeout
  while (Date.now() < end) {
    const result = await check()
    if (result) return result
    await pause(220)
  }
  throw new Error('Timed out')
}

try {
  await mkdir(config, { recursive: true, mode: 0o700 })
  await writeFile(env.LMM_MODELS_DEV_CACHE, JSON.stringify({ version: 1, source: 'https://models.dev/api.json?type=all',
    fetchedAt: Date.now(), data: { fixture: { models: { [randomUUID()]: { limit: { context: 4096, output: 20 } } } } } }))
  tunnel = await openCoreTls({ coreBase, issuer, certFile: process.env.FIXTURE_TLS_CERT, keyFile: process.env.FIXTURE_TLS_KEY })
  const version = (await execute('zed', ['--version'])).stdout.trim().split(' ').slice(0,2).join(' ')

  stage = 'ordinary user OAuth authorization and consent'
  login = spawn(bridgeBin, cliargs('login'), { env, cwd: root, stdio: ['ignore','pipe','ignore'], timeout: 45000 })
  const closed = once(login, 'close')
  const lines = createInterface({ input: login.stdout })
  let approved = false
  for await (const line of lines) {
    if (!line.startsWith(issuer + '/api/oauth2/authorize?')) continue
    const target = new URL(line)
    assert.equal(target.searchParams.get('client_id'), 'lmm-zed')
    await approveBrowser(target.href, { issuer, refresh, cap })
    approved = true
    break
  }
  assert.equal(approved, true)
  assert.equal((await closed)[0], 0)

  stage = 'account catalog and genuine Zed provider settings'
  const catalog = JSON.parse((await execute(bridgeBin, cliargs('catalog'))).stdout)
  assert.equal(catalog.length, 1)
  const route = catalog[0]
  assert.ok(route.apis.includes('openai-completions'))
  const model = { provider: 'lmm', model: route.id }
  await writeFile(join(config, 'settings.json'), JSON.stringify({ auto_update: false,
    telemetry: { metrics: false, diagnostics: false }, agent: { default_model: model, inline_assistant_model: model },
    edit_predictions: { disabled_globs: ['**/*'] } }))
  const portServer = net.createServer()
  portServer.listen(0, '127.0.0.1')
  await once(portServer, 'listening')
  const port = portServer.address().port
  await new Promise(resolve => portServer.close(resolve))
  bridge = spawn(bridgeBin, [...cliargs('start'),'--port',String(port)], { env, cwd: root,
    stdio: 'ignore', detached: true })
  await until(async () => {
    const settings = JSON.parse(await readFile(join(config, 'settings.json'), 'utf8'))
    const entry = settings.language_models?.openai_compatible?.lmm
    return entry?.api_url === 'http://127.0.0.1:' + port + '/v1' && entry.available_models.some(m => m.name === route.id)
  })

  stage = 'official Zed window loads document'
  const windows = async () => {
    try { return (await execute('xdotool', ['search','--onlyvisible','--class','zed'])).stdout.trim().split(/\s+/).filter(Boolean) }
    catch { return [] }
  }
  await until(async () => (await windows()).length, 40000)
  const file = join(root, 'lmm-host-document.txt')
  const original = 'Initial fixture text'
  await writeFile(file, original)
  await execute('zed', ['--existing',file])
  const win = await until(async () => {
    for (const id of await windows()) {
      if ((await execute('xdotool',['getwindowname',id])).stdout.includes('lmm-host-document')) return id
    }
  })
  await execute('xdotool', ['windowactivate','--sync',win])
  stage = 'native Inline Assistant submits selected text'
  await key('Escape', 'ctrl+a', 'ctrl+Return')
  await pause(450)
  await execute('xdotool', ['type','--clearmodifiers','--delay','12','Replace the selection with a short plain text reply.'])
  await key('Return')
  const call = await until(() => tunnel.calls[0]?.body ? tunnel.calls[0] : undefined, 45000)
  assert.equal(call.path, '/v1/chat/completions')
  assert.equal(call.status, 200)
  assert.equal(call.group, route.group)
  assert.match(call.type, /text\/event-stream/i)
  const frames = call.body.split(/\r?\n/).filter(l => l.startsWith('data:')).map(l => l.slice(5).trim())
  assert.equal(frames.at(-1), '[DONE]')
  const rows = frames.slice(0,-1).map(s => JSON.parse(s))
  const text = rows.flatMap(r => r.choices ?? []).map(c => c.delta?.content ?? '').join('')
  assert.ok(text.length > 0 && text !== original)
  assert.deepEqual(rows.flatMap(r => r.choices ?? []).map(c => c.finish_reason).filter(Boolean), ['stop'])
  stage = 'Zed saves the native editor result'
  // Saving a manually edited buffer accepts a completed inline transformation.
  await until(async () => {
    await key('ctrl+s')
    return (await readFile(file,'utf8')).trim() === text.trim()
  }, 20000)
  assert.equal(tunnel.calls.length, 1)
  assert.match(call.requestId, /^[a-f0-9]{64}$/)
  process.stdout.write(JSON.stringify({ host: version, provider: 'lmm-zed', request_id: call.requestId,
    granted_model: route.id, upstream_calls: 1, native_saved_reply: true, stop_count: 1, oauth_consent: true }))
} catch {
  const status = tunnel?.calls.map(c => ({ path: c.path, status: c.status }))
  console.error('Zed native acceptance failed at ' + stage + '; model HTTP ' + JSON.stringify(status ?? []))
  process.exitCode = 1
} finally {
  if (bridge?.pid) { try { process.kill(-bridge.pid,'SIGTERM') } catch {} }
  if (login?.exitCode === null) login.kill()
  await tunnel?.close()
  await rm(root,{ recursive: true, force: true })
}
