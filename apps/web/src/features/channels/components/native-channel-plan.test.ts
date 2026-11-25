/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import test from 'node:test'

import type {
  NativeConfiguration,
  NativeModelRoute,
  NativeTarget,
} from '../native-api'
import {
  diffNativeChannelCatalog,
  mergeChannelModelIds,
  planNativeChannelModels,
} from './native-channel-plan'

const groups = ['Enterprise', '国产']
const ids = [
  'gpt-4.1',
  'gpt-4.1-mini',
  'gpt-4.1-nano',
  'o3',
  'o3-mini',
  'o4-mini',
  'text-embedding-3-large',
  'text-embedding-3-small',
  'whisper-1',
]
const models = ids.map((id) => ({ upstreamModel: id, publicModel: id }))
const input = { channelId: 72, models, groups, speeds: ['standard'] as const }
function target(
  id: number,
  channel: number,
  model: string,
  priority: number
): NativeTarget {
  return {
    id,
    upstream_id: channel,
    upstream_model: model,
    priority,
    weight: 1,
    enabled: true,
    speeds: ['standard'],
    payload: 'adapt',
    timeout_ms: 30000,
  }
}

function snapshot(bound = false): NativeConfiguration {
  const routes: NativeModelRoute[] = []
  for (const model of ids) {
    for (const group of groups) {
      const id = 100 + routes.length
      const targets = [target(id, 10, model, 3)]
      if (bound) targets.push(target(id + 1000, input.channelId, model, 4))
      routes.push({
        id,
        model,
        group,
        targets,
        enabled: true,
        speeds: ['standard', 'fast'],
        failover: {
          max_attempts: 1,
          retry_on: [],
          allow_replay_after_send: false,
        },
      })
    }
  }
  return {
    version: { routing: 2, pricing: 5 },
    secret_storage_enabled: true,
    routing: {
      version: 2,
      upstreams: [10, 72].map((id) => ({
        id,
        enabled: true,
        protocol: 'openai_chat',
        endpoint: 'https://example.invalid',
        credential: { id, revision: 1 },
      })),
      groups: groups.map((name) => ({ name, enabled: true })),
      routes,
      aliases: [],
    },
    prices: {
      version: 5,
      rules: [
        {
          id: 8,
          model: ids[0],
          group: groups[0],
          speed: 'standard',
          rates: {
            input: 1000,
            output: 2000,
            cache_read: 0,
            cache_write: 0,
            request: 0,
          },
          group_multiplier_ppm: 1000000,
          speed_multiplier_ppm: 1000000,
        },
      ],
    },
    bindings: {
      names: {},
      contexts: [],
      vendors: {},
      models: {},
      discoveries: {},
      price_drafts: {},
    },
  }
}

test('nine OpenAI-compatible IDs across Enterprise and 国产 produce eighteen additive backup bindings and preserve the primary', () => {
  const config = snapshot()
  const before = structuredClone(config)
  const selected = mergeChannelModelIds(
    ids.slice(0, 5),
    [ids[1], ...ids.slice(5)].join(',\n')
  )
  assert.deepEqual(selected, ids)
  const plan = planNativeChannelModels(config, {
    ...input,
    models: selected.map((id) => ({ upstreamModel: id, publicModel: id })),
    priceTemplateRuleId: 8,
  })
  assert.equal(plan.rows.length, 18)
  assert.equal(new Set(plan.rows.map((row) => row.routeId)).size, 18)
  for (const row of plan.rows) {
    assert.equal(row.action, 'add_target')
    assert.equal(row.publicModel, row.upstreamModel)
    assert.equal(row.priority, 4)
    assert.deepEqual(
      row.existingTargets,
      config.routing.routes.find((r) => r.id === row.routeId)?.targets
    )
    assert.equal(row.existingTargets[0].priority, 3)
  }
  assert.equal(plan.missingPrices.length, 17)
  assert.equal(plan.proposedPrices.length, 17)
  assert.ok(plan.proposedPrices.every((rule) => rule.templateRuleId === 8))
  assert.deepEqual(config, before)
})

test('next synchronization adds a single new ID in two groups and retains a disappeared upstream binding', () => {
  const config = snapshot(true)
  const before = structuredClone(config)
  const fresh = ids.slice(1).concat('new-model', 'new-model')
  const delta = diffNativeChannelCatalog(config, input.channelId, fresh)
  assert.deepEqual(delta, {
    added: ['new-model'],
    retained: ids.slice(1),
    disappeared: [ids[0]],
  })
  const plan = planNativeChannelModels(config, {
    ...input,
    models: delta.added.map((id) => ({ upstreamModel: id, publicModel: id })),
  })
  assert.deepEqual(
    plan.rows.map((row) => [
      row.publicModel,
      row.group,
      row.action,
      row.priority,
    ]),
    [
      ['new-model', 'Enterprise', 'create_route', 0],
      ['new-model', '国产', 'create_route', 0],
    ]
  )
  assert.deepEqual(plan.proposedPrices, [])
  assert.deepEqual(config, before)
  assert.equal(
    planNativeChannelModels(config, input).rows.filter(
      (row) => row.action === 'existing'
    ).length,
    18
  )
})

test('group overrides, group-scoped aliases and failed lookups never silently rename, replace or price other bindings', () => {
  const config = snapshot(true)
  config.routing.aliases.push({
    group: '国产',
    alias: 'friendly',
    model: ids[0],
  })
  const plan = planNativeChannelModels(config, {
    ...input,
    models: [
      { upstreamModel: 'one', publicModel: 'friendly' },
      { upstreamModel: 'two', publicModel: 'friendly', groups: ['Enterprise'] },
      { upstreamModel: 'replacement', publicModel: ids[0], groups: ['国产'] },
      { upstreamModel: 'one', publicModel: 'special', groups: ['国产'] },
      { upstreamModel: 'two', publicModel: 'special', groups: ['Enterprise'] },
    ],
    priceTemplateRuleId: 999,
  })
  assert.deepEqual(
    plan.rows.map((row) => row.conflict ?? row.action),
    [
      'duplicate_public_model',
      'duplicate_public_model',
      'alias_in_use',
      'different_upstream_model',
      'create_route',
      'create_route',
    ]
  )
  assert.deepEqual(
    plan.rows
      .filter((row) => row.action === 'conflict')
      .flatMap((row) => row.missingPrices),
    []
  )
  assert.equal(plan.missingPrices.length, 2)
  assert.deepEqual(plan.proposedPrices, [])
})

test('backup retry budget and explicit-primary limits match Core without editing incumbents', () => {
  const config = snapshot()
  const route = config.routing.routes[0]
  const one = { ...input, models: [models[0]], groups: [groups[0]] }
  const row = () => planNativeChannelModels(config, one).rows[0]
  assert.deepEqual(row().failoverAfter, {
    max_attempts: 2,
    retry_on: ['connect'],
    allow_replay_after_send: false,
  })
  assert.deepEqual(row().failover, {
    max_attempts: 1,
    retry_on: [],
    allow_replay_after_send: false,
  })
  assert.equal(
    planNativeChannelModels(config, { ...one, placement: 'primary' }).rows[0]
      .priority,
    2
  )
  route.targets[0].priority = 0
  assert.equal(
    planNativeChannelModels(config, { ...one, placement: 'primary' }).rows[0]
      .conflict,
    'no_primary_priority'
  )
  route.targets[0].priority = 65535
  assert.equal(row().conflict, 'backup_priority_exhausted')
  route.targets[0].priority = 5
  for (let i = 1; i < 8; i++) {
    const id = 100 + i
    config.routing.upstreams.push({ ...config.routing.upstreams[0], id })
    route.targets.push(target(300 + i, id, ids[0], i))
  }
  assert.equal(row().conflict, 'retry_budget_exhausted')
  route.targets[4].weight = 0
  assert.equal(row().failoverAfter?.max_attempts, 8)
  route.failover.retry_on = ['timeout']
  assert.deepEqual(row().failoverAfter?.retry_on, ['timeout'])
})

test('widening a retained target explicitly lists new speeds and requires their exact published price selector', () => {
  const config = snapshot(true)
  const one = {
    ...input,
    models: [models[0]],
    groups: [groups[0]],
    speeds: ['standard', 'fast'] as const,
  }
  const row = planNativeChannelModels(config, one).rows[0]
  assert.equal(row.action, 'extend_target')
  assert.deepEqual(row.addedSpeeds, ['fast'])
  assert.equal(row.priority, 4)
  assert.deepEqual(row.missingPrices, [
    { group: 'Enterprise', model: ids[0], speed: 'fast' },
  ])
  assert.deepEqual(row.existingTargets[1].speeds, ['standard'])
})
