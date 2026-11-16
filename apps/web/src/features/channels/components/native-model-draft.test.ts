/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { NativeConfiguration } from '../native-api'
import {
  microsToUsd,
  modelIdentity,
  newModelTarget,
  readNativeModelDraft,
  replaceModelTargets,
  usdToMicros,
} from './native-model-draft'

const draftInput = {
  expected_version: { routing: 4, pricing: 3 },
  route: {
    id: 3,
    model: 'public-model',
    group: 'default',
    enabled: true,
    speeds: ['standard'],
    failover: {
      max_attempts: 2,
      retry_on: ['connect'],
      allow_replay_after_send: false,
    },
    targets: [
      {
        id: 21,
        upstream_id: 1,
        upstream_model: 'source-model',
        enabled: true,
        priority: 0,
        weight: 1,
        speeds: ['standard'],
        payload: { transparent: { protocol: 'openai_chat' } },
        timeout_ms: 30_000,
        client_protocols: ['openai_chat'],
      },
    ],
  },
  metadata: {
    display_name: 'Public model',
    description: 'Synthetic',
    vendor_id: null,
  },
  prices: [
    {
      id: 1,
      model: 'public-model',
      group: 'default',
      speed: 'standard',
      rates: {
        input: 1_000_001,
        output: 2_000_000,
        cache_read: 0,
        cache_write: 0,
        request: 0,
      },
      group_multiplier_ppm: 1_000_000,
      speed_multiplier_ppm: 1_000_000,
    },
  ],
  contexts: [
    {
      upstream_id: 1,
      upstream_model: 'source-model',
      endpoint: 'https://supplier.example/v1/',
      protocol: 'openai_chat',
      max_input_tokens: 8192,
      max_output_tokens: 4096,
      contract_revision: 'source-contract',
      missing_cached_tokens_is_zero: true,
    },
  ],
}
function fixture() {
  const draft = readNativeModelDraft(JSON.stringify(draftInput))
  assert.ok(draft)
  const configuration: NativeConfiguration = {
    version: { routing: 4, pricing: 3 },
    secret_storage_enabled: true,
    routing: {
      version: 4,
      upstreams: [
        {
          id: 1,
          endpoint: 'https://supplier.example/v1/',
          protocol: 'openai_chat',
          credential: { id: 1, revision: 1 },
          enabled: true,
        },
        {
          id: 2,
          endpoint: 'https://other.example/v1/',
          protocol: 'anthropic_messages',
          credential: { id: 2, revision: 1 },
          enabled: true,
        },
      ],
      groups: [{ name: 'default', enabled: true }],
      aliases: [],
      routes: [
        {
          ...draftInput.route,
          targets: [{ ...draftInput.route.targets[0], id: 80 }],
        },
      ],
    },
    prices: { version: 3, rules: draftInput.prices },
    bindings: {
      names: { 1: 'Supplier', 2: 'Other supplier' },
      contexts: [
        ...draftInput.contexts,
        {
          upstream_id: 2,
          upstream_model: 'reviewed-model',
          endpoint: 'https://other.example/v1/',
          protocol: 'anthropic_messages',
          max_input_tokens: 16384,
          max_output_tokens: 8192,
          contract_revision: 'reviewed-contract',
          missing_cached_tokens_is_zero: false,
        },
      ],
      vendors: {},
      models: {},
      discoveries: {},
      price_drafts: {},
    },
  }
  return { draft, configuration }
}

test('USD edits convert exactly to integer micro-USD and reject precision loss', () => {
  for (const [usd, micro] of [
    ['0', 0],
    ['0.000001', 1],
    ['1.234567', 1_234_567],
    ['9007199254.740991', Number.MAX_SAFE_INTEGER],
  ] as const) {
    assert.equal(usdToMicros(usd), micro)
    assert.equal(microsToUsd(micro), usd)
  }
  for (const invalid of ['', '-1', '1e6', '0.0000001', '9007199254.740992']) {
    assert.equal(usdToMicros(invalid), null)
  }
})

test('model identity edits update all price bindings without losing CAS or routing policies', () => {
  const { draft } = fixture()
  const next = modelIdentity(
    modelIdentity(draft, 'model', 'new-public-model'),
    'group',
    'premium'
  )
  assert.equal(next.route.model, 'new-public-model')
  assert.equal(next.prices[0].model, 'new-public-model')
  assert.equal(next.prices[0].group, 'premium')
  assert.deepEqual(next.route.targets, draft.route.targets)
  assert.deepEqual(next.route.failover, draft.route.failover)
  assert.deepEqual(next.contexts, draft.contexts)
  assert.deepEqual(next.expected_version, { routing: 4, pricing: 3 })
})

test('target changes require the exact supplier/model contract and retain globally unique target IDs', () => {
  const { draft, configuration } = fixture()
  const target = {
    ...draft.route.targets[0],
    upstream_id: 2,
    upstream_model: 'unreviewed-model',
  }
  const next = replaceModelTargets(draft, [target], configuration)
  assert.deepEqual(next.contexts, [
    {
      upstream_id: 2,
      upstream_model: 'unreviewed-model',
      endpoint: 'https://other.example/v1/',
      protocol: 'anthropic_messages',
      max_input_tokens: '',
      max_output_tokens: '',
      contract_revision: '',
    },
  ])
  const reviewed = replaceModelTargets(
    draft,
    [{ ...target, upstream_model: 'reviewed-model' }],
    configuration
  )
  assert.deepEqual(reviewed.contexts, [configuration.bindings.contexts[1]])
  assert.equal(newModelTarget(draft, configuration).id, 81)
  assert.deepEqual(replaceModelTargets(draft, [], configuration).contexts, [])
  assert.equal(draft.contexts[0].contract_revision, 'source-contract')
})
