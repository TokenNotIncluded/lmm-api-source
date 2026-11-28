/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { afterEach, test } from 'node:test'

import { readNativeRoutingScope } from '@/features/channels/native-api'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import type { CatalogContext, previewCatalog } from './catalog'
import { executeCommand, parseScript } from './commands'
import { executeTerminalOperation, type TerminalPreview } from './operations'

const adapter = api.defaults.adapter
const originalFetch = globalThis.fetch
const auth = useAuthStore.getState().auth
afterEach(() => {
  api.defaults.adapter = adapter
  globalThis.fetch = originalFetch
  useAuthStore.setState({ auth })
})
function fixture() {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-session',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 11, role: 10, username: 'fixture-admin' },
    session: {
      sid: 'fixture-terminal',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'fixture',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
  const scope = readNativeRoutingScope()
  assert.ok(scope)
  const context = { scope, previews: new Map<string, TerminalPreview>() }
  const routerVersion = { routing: 1, pricing: 1 }
  let state = {
    version: { ...routerVersion },
    routing: {
      version: 1,
      upstreams: [
        {
          id: 1,
          endpoint: 'https://example.invalid/v1',
          protocol: 'openai_chat',
          enabled: true,
          credential: { id: 2, revision: 1 },
        },
        {
          id: 2,
          endpoint: 'https://example.invalid/v1',
          protocol: 'gemini',
          enabled: true,
          credential: { id: 3, revision: 1 },
        },
      ],
      groups: [{ name: 'g', enabled: true }],
      routes: [],
      aliases: [],
    },
    prices: { version: 1, rules: [] },
    bindings: {
      contexts: [],
      discoveries: {
        '1': { models: ['m', 'incomplete', 'manual'], observed_at: 1 },
      },
      names: {},
      vendors: {},
      models: {},
      price_drafts: {},
      official_prices: {
        models: {
          manual: {
            source: 'manual',
            source_unit: 'USD/1M tokens',
            rates: {
              request: 0,
              input: 3,
              output: 2,
              cache_write: 0,
              cache_read: 0,
            },
          },
        },
        group_multipliers_ppm: { g: 1_500_000 },
        speed_multipliers_ppm: { fast: 2_000_000, ultrafast: 3_000_000 },
      },
    },
  }
  let writes = 0
  api.defaults.adapter = async (config) => {
    const body = config.data ? JSON.parse(config.data) : undefined
    let data: unknown
    if (config.method === 'get') data = state
    else if (config.method === 'post') {
      assert.equal(config.url, '/core/v1/admin/routing/configuration/preview')
      assert.deepEqual(body.expected_version, state.version)
      data = {
        ...body,
        prices: {
          ...body.prices,
          rules: body.bindings.official_prices?.models?.m
            ? [
                {
                  model: 'm',
                  group: 'g',
                  speed: 'fast',
                  group_multiplier_ppm: 1_500_000,
                  speed_multiplier_ppm: 2_000_000,
                },
              ]
            : [],
        },
        saved: false,
        confirmation_digest: createHash('sha256')
          .update(config.data)
          .digest('hex'),
      }
    } else {
      assert.equal(config.method, 'put')
      const { confirmation_digest, ...draft } = body
      assert.equal(
        confirmation_digest,
        createHash('sha256').update(JSON.stringify(draft)).digest('hex')
      )
      writes++
      state = {
        ...state,
        routing: body.routing,
        prices: body.prices,
        bindings: body.bindings,
        version: {
          routing: body.routing.version,
          pricing: body.prices.version,
        },
      }
      data = { version: state.version }
    }
    return {
      status: 200,
      statusText: 'OK',
      config,
      headers: {},
      data: { success: true, data },
    }
  }
  globalThis.fetch = async (url) => {
    assert.equal(url, 'https://models.dev/api.json')
    return new Response(
      JSON.stringify({
        p: {
          models: {
            m: {
              cost: { input: 0.5, output: 2, cache_read: 0.05, cache_write: 1 },
            },
            incomplete: { cost: { input: 1, output: 1 } },
          },
        },
      }),
      { headers: { 'content-type': 'application/json' } }
    )
  }
  return {
    run: (command: string, args: Record<string, unknown>) =>
      executeTerminalOperation(command, args, context),
    context,
    writes: () => writes,
    state: () => state,
  }
}
test('price preview preserves manual prices, reports missing meters and commits only a confirmed model', async () => {
  const f = fixture()
  const preview = await f.run('prices.preview', {
    provider: 'p',
    models: ['m', 'manual', 'incomplete'],
    request_rate: 0,
  })
  assert.ok('rows' in preview && preview.preview)
  assert.deepEqual(
    preview.rows.map((r) => r.status),
    ['ready', 'preserved', 'blocked']
  )
  assert.match(preview.rows[2].reason!, /rates.cache_read.*rates.cache_write/)
  const detail = preview.rows[0].details as {
    base: { input: string }
    effective: { final: { input: string } }[]
  }
  assert.equal(detail.base.input, '0.5')
  assert.equal(detail.effective[0].final.input, '1.5')
  await assert.rejects(
    f.run('changes.confirm', {
      preview: preview.preview,
      items: ['incomplete'],
    }),
    /unapproved/
  )
  assert.equal(f.writes(), 0)
  const result = await f.run('changes.confirm', {
    preview: preview.preview,
    items: ['m'],
  })
  assert.equal(result.status, 'published')
  assert.deepEqual(f.state().version, { routing: 2, pricing: 2 })
  assert.equal(f.writes(), 1)
  await assert.rejects(
    f.run('changes.confirm', { preview: preview.preview, items: ['m'] }),
    /expired/
  )
})
test('protocol rows reject incompatible selection and stale previews regenerate before publishing', async () => {
  const f = fixture()
  const preview = await f.run('protocols.preview', {
    from: 'openai_chat',
    to: 'openai_responses',
    channels: [1, 2],
  })
  assert.ok('rows' in preview && preview.preview)
  assert.deepEqual(
    preview.rows.map((r) => r.status),
    ['ready', 'blocked']
  )
  f.state().version.routing++
  f.state().routing.version++
  const conflict = await f.run('changes.confirm', {
    preview: preview.preview,
    items: ['1'],
  })
  assert.ok('replacement' in conflict && conflict.replacement.preview)
  assert.equal(f.writes(), 0)
  await f.run('changes.confirm', {
    preview: conflict.replacement.preview,
    items: ['1'],
  })
  assert.equal(f.state().routing.upstreams[0].protocol, 'openai_responses')
  assert.equal(f.state().routing.upstreams[1].protocol, 'gemini')
  assert.equal(f.writes(), 1)
})
test('a disabled operator or unconfirmed token request rate cannot publish', async () => {
  const f = fixture()
  const preview = await f.run('prices.preview', {
    provider: 'p',
    models: ['m'],
  })
  assert.ok('rows' in preview)
  assert.equal(preview.preview, null)
  assert.match(preview.rows[0].reason!, /rates.request/)
  useAuthStore.getState().auth.reset()
  await assert.rejects(
    f.run('protocols.preview', { from: 'openai_chat', to: 'openai_responses' }),
    /session changed/
  )
  assert.equal(f.writes(), 0)
})

test('published model aliases use the exact upstream source and keep the public price key', async () => {
  const f = fixture()
  Object.assign(f.state().routing, {
    routes: [
      {
        id: 1,
        group: 'g',
        model: 'public-m',
        speeds: ['standard'],
        targets: [
          { upstream_id: 1, upstream_model: 'm', speeds: ['standard'] },
        ],
      },
    ],
  })
  const draft = await f.run('prices.preview', {
    provider: 'p',
    models: ['public-m', 'm'],
    request_rate: 0,
  })
  assert.ok('rows' in draft && draft.preview)
  assert.deepEqual(
    draft.rows.map((row) => row.status),
    ['ready', 'blocked']
  )
  assert.equal(
    (draft.rows[0].details as { source: string }).source,
    'models.dev/p/m'
  )
  assert.match(draft.rows[1].reason!, /use_public_model_ids: public-m/)
})

test('all-channel previews use one configuration per page and require explicit ready IDs to confirm', async () => {
  for (const selection of [
    {},
    { all: false },
    { all: true, channels: [1] },
    { channels: [1], after: 1 },
    { all: true, after: -1 },
  ]) {
    assert.throws(() =>
      parseScript('catalog.preview ' + JSON.stringify(selection))
    )
  }
  assert.throws(() => parseScript('catalog.confirm {"all":true}'))
  const f = fixture()
  const context: CatalogContext = { ...f.context, catalogs: new Map() }
  const paths: string[] = []
  const configuration = '/core/v1/admin/routing/configuration'
  const fetchModels = '/api/channel/fetch_models/'
  const state = f.state()
  const upstreams = Array.from({ length: 102 }, (_, i) => ({
    ...state.routing.upstreams[0],
    id: 102 - i,
    enabled: i % 2 === 0,
  }))
  api.defaults.adapter = async (config) => {
    assert.equal(config.method, 'get')
    const path = config.url!
    paths.push(path)
    if (path === fetchModels + '2') throw new Error('upstream_unavailable')
    const data =
      path === configuration
        ? { ...state, routing: { ...state.routing, upstreams } }
        : ['model']
    return {
      status: 200,
      statusText: 'OK',
      config,
      headers: {},
      data: { success: true, data },
    }
  }
  const preview = async (selection: object) =>
    (await executeCommand(
      parseScript('catalog.preview ' + JSON.stringify(selection))[0],
      context
    )) as Awaited<ReturnType<typeof previewCatalog>>
  const first = await preview({ all: true })
  assert.deepEqual(paths, [
    configuration,
    ...Array.from({ length: 100 }, (_, i) => fetchModels + (i + 1)),
  ])
  assert.equal(first.rows[1].status, 'blocked')
  assert.ok(first.preview)
  assert.deepEqual(first.next, {
    all: true,
    after: 100,
    reason: 'catalog-refresh',
  })
  await assert.rejects(
    executeCommand(
      parseScript(
        'catalog.confirm ' +
          JSON.stringify({ preview: first.preview, channels: [2] })
      )[0],
      context
    ),
    /unapproved/
  )
  paths.length = 0
  const second = await preview(first.next!)
  assert.deepEqual(paths, [
    configuration,
    fetchModels + '101',
    fetchModels + '102',
  ])
  assert.equal(second.next, null)
  paths.length = 0
  await preview({ channels: [2, 1] })
  assert.deepEqual(paths, [configuration, fetchModels + '2', fetchModels + '1'])
})
