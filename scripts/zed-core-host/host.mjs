// SPDX-License-Identifier: AGPL-3.0-or-later
// The only inference caller is Zed's native Inline Assistant.
import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { once } from 'node:events'
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { setTimeout as pause } from 'node:timers/promises'
import { promisify } from 'node:util'
import { approveBrowser, openCoreTls } from '../ci/managed-host-fixture.mjs'

const run = promisify(execFile)
const input = []
for await (const part of process.stdin) input.push(part)
const { coreBase, issuer: configuredIssuer, refresh, cap, inlineMode } = JSON.parse(Buffer.concat(input))
const issuer = new URL(configuredIssuer).origin
assert.ok(['text', 'tools'].includes(inlineMode))
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
let tunnel, login, bridge, win
let stage = 'private host setup'
const captureDir = process.env.ZED_NATIVE_IMAGES
const nativePrompt = 'Write a short reply for this selection'

async function windowSnapshot(name) {
  const file = join(root, 'window.png')
  await execute('import', ['-window',win,file])
  const { stdout } = await execute('tesseract', [file,'stdout','--psm','11'])
  if (/lmmk_|lmm_[ar]t_|Bearer\\s+[A-Za-z0-9_-]{12,}|[?&](?:code|state)=/i.test(stdout)) throw new Error('Private observation')
  if (name) {
    await mkdir(captureDir,{recursive:true})
    await copyFile(file,join(captureDir,name + '.png'))
  }
  return stdout.toLowerCase().replace(/\\s+/g,' ')
}

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

  // This isolated browser launcher receives the ordinary authorization link chosen by the CLI.
  // The existing Core browser-consent driver opens it over TLS and follows the real PKCE callback.
  const tools = join(root, 'bin')
  await mkdir(tools, { mode: 0o700 })
  const linkFile = join(root, 'authorization.url')
  await writeFile(join(tools, 'xdg-open'), '#!/bin/sh\numask 077\nprintf "%s" "$1" > "$LMM_ZED_AUTH_URL"\n', { mode: 0o700 })
  env.PATH = tools + ':' + env.PATH
  env.LMM_ZED_AUTH_URL = linkFile
  stage = 'Zed bridge emits an ordinary browser authorization URL'
  login = spawn(bridgeBin, cliargs('login'), { env, cwd: root, stdio: ['ignore','pipe','ignore'], timeout: 45000 })
  const closed = once(login, 'close')
  const target = new URL(await until(async () => {
    if (login.exitCode !== null) throw new Error('login process finished before requesting browser')
    return readFile(linkFile, 'utf8').catch(() => undefined)
  }, 20000))
  assert.equal(target.origin, issuer)
  assert.equal(target.searchParams.get('client_id'), 'lmm-zed')
  stage = 'Core ordinary user consent and loopback PKCE callback'
  await approveBrowser(target.href, { issuer, refresh, cap })
  stage = 'Zed persists the OAuth exchange and exits login'
  assert.equal((await closed)[0], 0)
  assert.ok(tunnel.getAccess())

  stage = 'account catalog and genuine Zed provider settings'
  const catalog = JSON.parse((await execute(bridgeBin, cliargs('catalog'))).stdout)
  assert.equal(catalog.length, 1)
  const route = catalog[0]
  assert.ok(route.apis.includes('openai-completions'))
  const model = { provider: 'lmm', model: route.id }
  const inlineSettings = inlineMode === 'text' ? { inline_assistant_use_streaming_tools: false } : {}
  await writeFile(join(config, 'settings.json'), JSON.stringify({ auto_update: false,
    telemetry: { metrics: false, diagnostics: false }, agent: { default_model: model, inline_assistant_model: model,
      ...inlineSettings },
    edit_predictions: { disabled_globs: ['**/*'] } }))
  await writeFile(join(config, 'keymap.json'), JSON.stringify([{ context: 'Editor && mode == full',
    bindings: { f8: ['assistant::InlineAssist', { prompt: nativePrompt }] } }]))
  const portServer = net.createServer()
  portServer.listen(0, '127.0.0.1')
  await once(portServer, 'listening')
  const port = portServer.address().port
  await new Promise(resolve => portServer.close(resolve))
  const startArgs = [...cliargs('start'), '--port', String(port)]
  if (inlineMode === 'tools') {
    // The isolated supplier implements this tool. Use the bridge's supported
    // per-model capability override and its catalog-derived token limits.
    const configured = JSON.parse((await execute(bridgeBin, cliargs('settings'))).stdout)
    const offered = configured.language_models.openai_compatible.lmm.available_models
    assert.equal(offered.length, 1)
    assert.equal(offered[0].name, route.id)
    const limits = { max_tokens: offered[0].max_tokens, max_output_tokens: offered[0].max_output_tokens }
    const override = join(root, 'model-override.json')
    await writeFile(override, JSON.stringify([{ id: route.id, ...limits, capabilities: { tools: true } }]))
    startArgs.push('--models', override)
  }
  bridge = spawn(bridgeBin, startArgs, { env, cwd: root,
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
  win = await until(async () => {
    for (const id of await windows()) {
      if ((await execute('xdotool',['getwindowname',id])).stdout.includes('lmm-host-document')) return id
    }
  })
  await execute('xdotool', ['windowactivate','--sync',win])
  stage = 'native file is visible and the editor has focus'
  await key('Escape', 'ctrl+a')
  await windowSnapshot('document')
  stage = 'documented inline action opens its native prompt'
  let visiblePrompt = false
  for (let openings = 0; openings < 2 && !visiblePrompt; openings++) {
    await key('F8')
    const untilTime = Date.now() + 6000
    do {
      // Read the native focused text via Zed's normal Copy command. The document
      // and the prompt have different content, so an unopened input cannot pass.
      await key('ctrl+a', 'ctrl+c')
      const selected = await execute('xclip', ['-selection','clipboard','-o']).catch(() => undefined)
      visiblePrompt = selected?.stdout === nativePrompt
      if (!visiblePrompt) await pause(350)
    } while (!visiblePrompt && Date.now() < untilTime)
  }
  assert.ok(visiblePrompt, 'native prompt was not rendered')
  await windowSnapshot('native-prompt')
  stage = 'visible native prompt submits to the LMM bridge'
  await key('Return')
  const call = await until(() => tunnel.calls[0]?.body ? tunnel.calls[0] : undefined, 45000)
  assert.equal(call.path, '/v1/chat/completions')
  assert.equal(call.status, 200)
  assert.equal(call.group, route.group)
  assert.match(call.type, /text\/event-stream/i)
  const frames = call.body.split(/\r?\n/).filter(l => l.startsWith('data:')).map(l => l.slice(5).trim())
  assert.equal(frames.at(-1), '[DONE]')
  const rows = frames.slice(0,-1).map(s => JSON.parse(s))
  const choices = rows.flatMap(value => value.choices ?? [])
  const endings = choices.map(choice => choice.finish_reason).filter(Boolean)
  const toolPieces = choices.flatMap(choice => choice.delta?.tool_calls ?? [])
  let text
  if (inlineMode === 'tools') {
    assert.deepEqual(endings, ['tool_calls'])
    assert.ok(toolPieces.length >= 2)
    assert.ok(toolPieces.every(piece => piece.index === 0))
    assert.equal(toolPieces.map(piece => piece.function?.name ?? '').join(''), 'rewrite_section')
    const args = toolPieces.map(piece => piece.function?.arguments ?? '').join('')
    text = JSON.parse(args).replacement_text
  } else {
    assert.deepEqual(endings, ['stop'])
    assert.equal(toolPieces.length, 0)
    text = choices.map(choice => choice.delta?.content ?? '').join('')
  }
  assert.ok(typeof text === 'string' && text.length > 0 && text !== original)
  stage = 'Zed saves the native editor result'
  // Saving a manually edited buffer accepts a completed inline transformation.
  await until(async () => {
    await key('ctrl+s')
    return (await readFile(file,'utf8')).trim() === text.trim()
  }, 20000)
  await windowSnapshot('saved-result')
  assert.equal(tunnel.calls.length, 1)
  assert.match(call.requestId, /^[a-f0-9]{64}$/)
  process.stdout.write(JSON.stringify({ host: version, provider: 'lmm-zed', request_id: call.requestId,
    granted_model: route.id, upstream_calls: 1, native_saved_reply: true, inline_assistant_mode: inlineMode, finish_reason: endings[0], tool_fragments: toolPieces.length, stop_count: 1, oauth_consent: true }))
} catch (error) {
  if (win) await windowSnapshot('failure').catch(() => {})
  const safeCode = /^[A-Z_]+$/.test(error?.code ?? '') ? error.code : error?.name
  const reason = error?.message === 'Timed out' ? 'deadline' : safeCode
  const status = tunnel?.calls.map(c => {
    let code
    if (c.status >= 400) {
      try {
        const value = JSON.parse(c.body)?.error?.code
        if (typeof value === 'string' && /^[a-z0-9_.-]{1,80}$/.test(value)) code = value
      } catch {}
    }
    return { path: c.path, status: c.status, code }
  })
  console.error('Zed native acceptance failed at ' + stage + ' (' + reason + '); model HTTP ' + JSON.stringify(status ?? []))
  process.exitCode = 1
} finally {
  if (bridge?.pid) { try { process.kill(-bridge.pid,'SIGTERM') } catch {} }
  if (login?.exitCode === null) login.kill()
  await tunnel?.close()
  await rm(root,{ recursive: true, force: true })
}
