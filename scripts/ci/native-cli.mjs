// SPDX-License-Identifier: AGPL-3.0-or-later
// Run a stock CLI against the disposable loopback Core; never rewrite its request or response.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const client = process.argv[2]
assert.ok(['claude', 'codex'].includes(client))
const input = []
for await (const chunk of process.stdin) input.push(chunk)
const { core, key } = JSON.parse(Buffer.concat(input).toString('utf8'))
const origin = new URL(core)
assert.equal(origin.hostname, '127.0.0.1')
assert.equal(origin.protocol, 'http:')
const fixture = await mkdtemp(join(tmpdir(), 'native-cli-'))
const home = join(fixture, 'home')
const work = join(fixture, 'work')
const binary = resolve(process.env.NATIVE_CLI_BIN)
const env = { PATH: process.env.PATH, HOME: home, LANG: 'C.UTF-8', NO_COLOR: '1',
  XDG_CONFIG_HOME: join(home, 'config'), XDG_CACHE_HOME: join(home, 'cache'),
  HTTP_PROXY: 'http://127.0.0.1:9', HTTPS_PROXY: 'http://127.0.0.1:9', NO_PROXY: '127.0.0.1,localhost' }
function execute(args) {
  return new Promise((ok, fail) => {
    const proc = spawn(binary, args, { cwd: work, env, stdio: ['ignore','pipe','ignore'], timeout: 80000, killSignal: 'SIGKILL' })
    let size = 0
    const out = []
    proc.stdout.on('data', chunk => {
      size += chunk.length
      if (size > 2 * 1024 * 1024) proc.kill('SIGKILL')
      else out.push(chunk)
    })
    proc.once('error', fail)
    proc.once('close', code => code === 0 ? ok(Buffer.concat(out).toString('utf8')) : fail(new Error('CLI exit ' + code)))
  })
}
let stage = 'prepare home'
try {
  await Promise.all([mkdir(home, { mode: 0o700 }), mkdir(work, { mode: 0o700 })])
  stage = 'verify binary version'
  const versionText = (await execute(['--version'])).trim()
  const expected = process.env.NATIVE_CLI_VERSION
  assert.ok(versionText.split(/[^\d.]+/).includes(expected))
  stage = 'read scoped Core model catalog'
  const catalog = await fetch(new URL('/v1/models', core), { headers: { authorization: 'Bearer ' + key },
    signal: AbortSignal.timeout(5000), redirect: 'error' })
  assert.equal(catalog.status, 200)
  const models = (await catalog.json()).data
  assert.equal(models.length, 1)
  const model = models[0].id
  assert.match(model, /^[a-z0-9-]+$/)
  if (client === 'claude') {
    Object.assign(env, { CLAUDE_CONFIG_DIR: home, ANTHROPIC_BASE_URL: core, ANTHROPIC_API_KEY: key,
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1', CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS: '1',
      DISABLE_TELEMETRY: '1', DISABLE_AUTOUPDATER: '1', DISABLE_ERROR_REPORTING: '1',
      CLAUDE_CODE_MAX_OUTPUT_TOKENS: '1024', MAX_THINKING_TOKENS: '0',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: model, ANTHROPIC_DEFAULT_SONNET_MODEL: model, ANTHROPIC_DEFAULT_OPUS_MODEL: model })
    stage = 'Claude Code print'
    const stdout = await execute(['--print', '--output-format', 'json', '--model', model,
      '--tools', '', '--no-session-persistence', '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}',
      '--system-prompt', 'Reply briefly.', 'Reply briefly.'])
    stage = 'Claude Code terminal result'
    const terminal = JSON.parse(stdout)
    assert.equal(terminal.type, 'result')
    assert.equal(terminal.subtype, 'success')
    assert.equal(terminal.is_error, false)
    assert.equal(terminal.result, 'native answer')
  } else {
    const codex = join(home, 'codex')
    await mkdir(codex, { mode: 0o700 })
    Object.assign(env, { CODEX_HOME: codex, CORE_FIXTURE_KEY: key })
    // The CLI reads this named key from its clean process environment; its config contains no bearer.
    await writeFile(join(codex, 'config.toml'), [
      'model_provider = "fixture"',
      'model = ' + JSON.stringify(model),
      'approval_policy = "never"', 'sandbox_mode = "read-only"', 'web_search = "disabled"',
      '[features]', 'shell_tool = false', 'apply_patch_freeform = false', 'multi_agent = false',
      '[model_providers.fixture]', 'name = "Core fixture"', 'base_url = ' + JSON.stringify(core + '/v1'),
      'env_key = "CORE_FIXTURE_KEY"', 'wire_api = "responses"', 'requires_openai_auth = false',
      'supports_websockets = false', 'request_max_retries = 0', 'stream_max_retries = 0',
    ].join('\n') + '\n', { mode: 0o600 })
    stage = 'Codex exec'
    const stdout = await execute(['exec', '--skip-git-repo-check', '--ephemeral', '--json', 'Reply briefly.'])
    stage = 'Codex terminal events'
    const events = stdout.trim().split('\n').filter(Boolean).map(line => JSON.parse(line))
    assert.ok(!events.some(e => e.type === 'turn.failed' || e.type === 'error'))
    assert.equal(events.filter(e => e.type === 'turn.completed').length, 1)
    assert.deepEqual(events.filter(e => e.type === 'item.completed' && e.item?.type === 'agent_message')
      .map(e => e.item.text), ['native answer'])
  }
  process.stdout.write(JSON.stringify({ selected: model, version: expected, completed: true }))
} catch {
  console.error(client + ' native acceptance failed during ' + stage)
  process.exitCode = 1
} finally {
  await rm(fixture, { recursive: true, force: true })
}
