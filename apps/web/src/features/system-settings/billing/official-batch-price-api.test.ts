/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import {
  readNativeRoutingScope,
  type NativeConfiguration,
} from '@/features/channels/native-api'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  previewMissingOfficialPrices,
  publishMissingOfficialPrices,
} from './official-price-api'
import { writeOfficialToml } from './official-price-toml'

const adapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const digest = 'a'.repeat(64)
const quote = {
  source: 'models.dev/test',
  source_unit: 'USD/1M tokens',
  rates: { input: 10, output: 20, cache_read: 0, cache_write: 0, request: 0 },
}
const drafts = new Map([['new', writeOfficialToml('new', quote)]])
const rule = {
  id: 1,
  model: 'manual',
  group: 'g',
  speed: 'standard',
  group_multiplier_ppm: 1000000,
  speed_multiplier_ppm: 1000000,
  rates: quote.rates,
}
function source(): NativeConfiguration {
  return {
    version: { routing: 2, pricing: 4 },
    routing: {
      version: 2,
      groups: [{ name: 'g', enabled: true }],
      upstreams: [],
      routes: [
        {
          model: 'new',
          group: 'g',
          speeds: ['standard'],
          enabled: true,
          targets: [
            { enabled: true, upstream_id: 3, upstream_model: 'mapped' },
          ],
        },
      ],
    },
    prices: { version: 4, rules: [rule] },
    bindings: {
      official_prices: {
        models: { kept: quote },
        group_multipliers_ppm: { g: 1000000 },
        speed_multipliers_ppm: { fast: 1000000, ultrafast: 1000000 },
      },
      discoveries: {
        '3': { models: ['mapped'], observed_at: 1, pending_models: ['mapped'] },
      },
      names: { '3': 'fixed' },
      price_drafts: {},
    },
  } as unknown as NativeConfiguration
}
function login() {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-prices',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role: 10, username: 'synthetic-admin' },
    session: {
      sid: 'synthetic-prices',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
  const scope = readNativeRoutingScope()
  assert.ok(scope)
  return scope
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previousAuth })
})

test('one full-document PUT with the reviewed digest, including a lost-receipt readback', async () => {
  const scope = login()
  for (const outcome of ['acknowledged', 'lost', 'wrong-version']) {
    const raw = source()
    let current = raw
    let prepared: NativeConfiguration | undefined
    const calls: string[] = []
    api.defaults.adapter = async (config) => {
      calls.push(config.method!)
      let data: unknown
      if (config.method === 'get') data = current
      else if (config.method === 'post') {
        assert.equal(config.url, '/core/v1/admin/routing/configuration/preview')
        const proposal = JSON.parse(config.data)
        assert.deepEqual(proposal.expected_version, raw.version)
        assert.deepEqual(proposal.prices.rules, raw.prices.rules)
        assert.deepEqual(proposal.bindings.official_prices.models.kept, quote)
        prepared = {
          ...raw,
          version: { routing: 3, pricing: 5 },
          routing: proposal.routing,
          prices: {
            version: 5,
            rules: [rule, { ...rule, id: 2, model: 'new' }],
          },
          bindings: structuredClone(proposal.bindings),
        }
        delete prepared.bindings.discoveries['3'].pending_models
        data = {
          ...prepared,
          saved: false,
          expected_version: raw.version,
          confirmation_digest: digest,
        }
      } else {
        assert.equal(config.method, 'put')
        assert.equal(config.url, '/core/v1/admin/routing/configuration')
        const body = JSON.parse(config.data)
        assert.equal(body.confirmation_digest, digest)
        assert.deepEqual(body.expected_version, { routing: 2, pricing: 4 })
        assert.deepEqual([body.routing.version, body.prices.version], [3, 5])
        assert.ok(prepared)
        if (outcome !== 'wrong-version') current = prepared
        if (outcome === 'lost') throw new Error('synthetic lost reply')
        data = {
          version:
            outcome === 'wrong-version'
              ? { routing: 3, pricing: 4 }
              : current.version,
        }
      }
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: { success: true, data },
      }
    }
    const preview = await previewMissingOfficialPrices(scope, raw, drafts)
    assert.deepEqual(preview.generatedRules, [
      { model: 'new', group: 'g', speed: 'standard', enabled: true },
    ])
    assert.deepEqual(preview.promoted, [{ channel: '3', model: 'mapped' }])
    if (outcome === 'wrong-version') {
      await assert.rejects(
        publishMissingOfficialPrices(scope, preview),
        /unknown/
      )
    } else {
      await publishMissingOfficialPrices(scope, preview)
    }
    assert.deepEqual(
      calls,
      outcome === 'acknowledged'
        ? ['get', 'post', 'put']
        : ['get', 'post', 'put', 'get']
    )
  }
})

test('stale configuration, official prices and operator-owned rules cannot start a batch', async () => {
  const scope = login()
  const raw = source()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.method, 'get')
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: raw },
    }
  }
  for (const id of ['kept', 'manual']) {
    await assert.rejects(
      previewMissingOfficialPrices(
        scope,
        raw,
        new Map([[id, writeOfficialToml(id, quote)]])
      ),
      /existing/i
    )
  }
  await assert.rejects(
    previewMissingOfficialPrices(
      scope,
      { ...raw, version: { routing: 1, pricing: 4 } },
      drafts
    ),
    /snapshot changed/
  )
  assert.equal(calls, 3)
})

test('the compiled preview cannot change another rule, route, or binding', async () => {
  const scope = login()
  const raw = source()
  for (const corruption of ['old-price', 'route', 'binding', 'added-base']) {
    const calls: string[] = []
    api.defaults.adapter = async (config) => {
      calls.push(config.method!)
      if (config.method === 'get')
        return {
          config,
          status: 200,
          statusText: 'OK',
          headers: {},
          data: { success: true, data: raw },
        }
      assert.equal(config.method, 'post')
      const proposed = JSON.parse(config.data)
      if (corruption === 'old-price') proposed.prices.rules[0].rates.output++
      if (corruption === 'route') proposed.routing.routes[0].enabled = false
      if (corruption === 'binding') proposed.bindings.names['3'] = 'unexpected'
      if (corruption === 'added-base')
        proposed.bindings.official_prices.models.new.rates.output++
      const data = { ...proposed, saved: false, confirmation_digest: digest }
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: { success: true, data },
      }
    }
    await assert.rejects(
      previewMissingOfficialPrices(scope, raw, drafts),
      /existing group price|Invalid price publication preview/i
    )
    assert.deepEqual(calls, ['get', 'post'])
  }
})
