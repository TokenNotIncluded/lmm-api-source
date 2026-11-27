// SPDX-License-Identifier: AGPL-3.0-or-later
// GitHub Actions only. The production Pi executable loads the production LMM extension.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { cp, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { createInterface } from 'node:readline'
import { setTimeout as pause } from 'node:timers/promises'
import { openCoreTls } from './managed-host-fixture.mjs'

function coreFailureReason({ status, body }) {
  if (status === 413) {
    try {
      const error = JSON.parse(body)?.error
      if (error?.code === 'relay_limit_exceeded' &&
          error.message === 'relay_limit_exceeded: local model input budget') return 'local_input_budget'
    } catch { /* An unrecognized Core response has no safe diagnostic label. */ }
  }
  return 'unknown'
}

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const input = JSON.parse(Buffer.concat(chunks).toString('utf8'))
const issuer = new URL(input.issuer)
assert.equal(issuer.protocol, 'https:')
assert.equal(issuer.hostname.endsWith('.test'), true)
const temporary = await mkdtemp(join(tmpdir(), 'core-pi-host-'))
const agent = join(temporary, 'agent')
const plugin = join(temporary, 'extension')
const source = resolve(process.env.PI_HOST_PACKAGE)
const bin = resolve(process.env.PI_HOST_BIN)
const env = {
  PATH: process.env.PATH, HOME: temporary, XDG_CONFIG_HOME: temporary,
  LANG: 'C.UTF-8', PI_CODING_AGENT_DIR: agent, PI_TELEMETRY: '0',
  PI_OFFLINE: '1', PI_SKIP_VERSION_CHECK: '1', LMM_ISSUER: issuer.origin,
  NODE_EXTRA_CA_CERTS: process.env.FIXTURE_TLS_CERT,
  LMM_MODELS_DEV_CACHE: join(temporary, 'metadata.json'),
}
let tunnel, child, terminalMessage
let stage = 'private fixture setup'
try {
  await mkdir(agent, { mode: 0o700 })
  await mkdir(plugin)
  await cp(join(source, 'src'), join(plugin, 'src'), { recursive: true })
  await cp(join(source, 'package.json'), join(plugin, 'package.json'))
  await writeFile(env.LMM_MODELS_DEV_CACHE, JSON.stringify({ version: 1, source: 'https://models.dev/api.json?type=all',
    fetchedAt: Date.now(), data: { fixture: { models: { [randomUUID()]: {
      limit: { context: 100, output: 20 }, modalities: { input: ['text'], output: ['text'] },
      reasoning: false, tool_call: false,
    } } } } }), { mode: 0o600 })
  await writeFile(join(agent, 'auth.json'), JSON.stringify({ lmm: {
    type: 'oauth', access: input.access, refresh: input.refresh, scope: input.scope,
    expires: Date.now() + 1000 * input.lifetime, lmm_issuer: issuer.origin,
    lmm_resource: input.resource, lmm_session: randomUUID(),
  } }), { mode: 0o600 })
  await writeFile(join(agent, 'settings.json'), JSON.stringify({ defaultProvider: 'lmm', retry: { enabled: false } }))
  tunnel = await openCoreTls({ coreBase: input.coreBase, issuer: issuer.origin,
    keyFile: process.env.FIXTURE_TLS_KEY, certFile: process.env.FIXTURE_TLS_CERT })

  stage = 'native extension install'
  await new Promise((done, fail) => {
    const install = spawn(bin, ['install', plugin], { cwd: temporary, env, stdio: 'ignore', timeout: 30000 })
    install.once('error', fail)
    install.once('exit', code => code === 0 ? done() : fail(new Error('install')))
  })
  child = spawn(bin, ['--mode', 'rpc', '--no-approve', '--no-session', '--no-context-files', '--no-tools',
    '--no-skills', '--no-prompt-templates', '--no-themes', '--system-prompt', 'Reply briefly.'], {
    cwd: temporary, env, stdio: ['pipe', 'pipe', 'ignore'], timeout: 65000,
  })
  const lines = createInterface({ input: child.stdout })
  const waiting = new Map()
  lines.on('line', line => {
    let event
    try { event = JSON.parse(line) } catch { return }
    if (event.type === 'response' && waiting.has(event.id)) {
      const waiter = waiting.get(event.id)
      waiting.delete(event.id)
      if (event.success) waiter.resolve(event.data)
      else waiter.reject(new Error('RPC command rejected'))
    } else if (event.type === 'agent_end') {
      terminalMessage = event.messages?.findLast(message => message.role === 'assistant')
    }
  })
  const stop = () => { for (const w of waiting.values()) w.reject(new Error('host exited')); waiting.clear() }
  child.once('error', stop)
  child.once('exit', stop)
  async function rpc(type, properties = {}) {
    if (child.exitCode !== null || child.signalCode !== null) throw new Error('host exited')
    const id = randomUUID()
    const response = new Promise((resolve, reject) => waiting.set(id, { resolve, reject }))
    child.stdin.write(JSON.stringify({ id, type, ...properties }) + '\n')
    return response
  }
  stage = 'real host catalog discovery'
  let models = []
  for (let attempt = 0; attempt < 30; attempt++) {
    models = (await rpc('get_available_models')).models.filter(m => m.provider === 'lmm')
    if (models.length) break
    await pause(300)
  }
  assert.equal(models.length, 1)
  const selected = models[0]
  stage = 'real host model selection'
  const active = await rpc('set_model', { provider: selected.provider, modelId: selected.id })
  assert.equal(active.id, selected.id)
  assert.equal(active.provider, selected.provider)
  stage = 'real host prompt and terminal'
  await rpc('prompt', { message: 'Reply briefly.' })
  for (let i = 0; !terminalMessage && i < 200 && child.exitCode === null && child.signalCode === null; i++) await pause(100)
  stage = 'provider and model identity on assistant result'
  assert.equal(terminalMessage?.provider, selected.provider)
  assert.equal(terminalMessage?.model, selected.id)
  stage = 'assistant stop reason'
  assert.equal(terminalMessage?.stopReason, 'stop')
  stage = 'output token receipt'
  assert.equal(terminalMessage.usage.output, 20)
  stage = 'assistant text integrity'
  assert.equal(terminalMessage.content.filter(c => c.type === 'text').map(c => c.text).join(''), 'local stream answer')
  stage = 'same-request Core response'
  const calls = tunnel.calls.filter(call => call.method === 'POST')
  assert.equal(calls.length, 1)
  const [call] = calls
  assert.ok(['/v1/chat/completions', '/v1/responses', '/v1/messages'].includes(call.path))
  assert.equal(call.status, 200)
  assert.equal(typeof call.requestId, 'string')
  assert.ok(call.requestId.length > 0 && call.requestId.length <= 256)
  process.stdout.write(JSON.stringify({ requestId: call.requestId, stopReason: terminalMessage.stopReason }))
} catch {
  const reasons = ['stop', 'length', 'toolUse', 'error', 'aborted']
  const stop = reasons.includes(terminalMessage?.stopReason) ? terminalMessage.stopReason : 'unavailable'
  const requests = (tunnel?.calls ?? []).map(({ path, status, body }) => ({
    reason: coreFailureReason({ status, body }),
    path: ['/v1/chat/completions', '/v1/responses', '/v1/messages'].includes(path) ? path : 'other', status,
  }))
  // Report only the allowlisted route, status and diagnostic label.
  console.error('Real Pi acceptance failed during ' + stage + ' ' + JSON.stringify({ stop, requests }))
  process.exitCode = 1
} finally {
  if (child && child.exitCode === null && child.signalCode === null) {
    const exited = new Promise(done => child.once('exit', done))
    child.kill('SIGKILL')
    await exited
  }
  await tunnel?.close()
  await rm(temporary, { recursive: true, force: true })
}
