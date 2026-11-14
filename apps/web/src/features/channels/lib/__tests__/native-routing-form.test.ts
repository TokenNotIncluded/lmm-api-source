/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  readChannelDraft,
  readModelDraft,
  toggleChannelAttachment,
  updateChannelAttachment,
  updateModelIdentity,
} from '../native-routing-form'

function fixture() {
  const draft = readChannelDraft(
    JSON.stringify({
      expected_version: { routing: 5, pricing: 3 },
      id: 2,
      name: 'Provider',
      tag: 'test',
      endpoint: 'https://provider.example/v1',
      protocol: 'openai_responses',
      enabled: true,
      attachments: [1, 2].map((route_id) => ({
        route_id,
        upstream_model: 'shared',
        priority: 7,
        weight: 3,
        timeout_ms: 10_000,
        speeds: ['standard'],
        enabled: false,
        client_protocols: ['openai_responses'],
        payload: { legacy_openai: { allow_speed: true } },
      })),
      contexts: [
        {
          upstream_id: 2,
          upstream_model: 'shared',
          protocol: 'openai_responses',
          endpoint: 'https://provider.example/v1',
          max_input_tokens: 65_536,
          max_output_tokens: 8192,
          contract_revision: 'provider-docs-v2',
          reservation: {
            mode: 'openai_responses_exact_v1',
            count_timeout_ms: 100,
          },
          responses_continuity: { default_store: false },
        },
      ],
    })
  )
  assert.ok(draft)
  return draft
}

test('removing one model binding retains a provider context used by another group', () => {
  const draft = fixture()
  const one = toggleChannelAttachment(
    draft,
    { id: 1, model: 'public-a', speeds: ['standard'] },
    false
  )
  assert.equal(one.attachments.length, 1)
  assert.deepEqual(one.contexts, draft.contexts)
  const none = toggleChannelAttachment(
    one,
    { id: 2, model: 'public-b', speeds: ['standard'] },
    false
  )
  assert.equal(none.contexts.length, 0)
  assert.deepEqual(none.expected_version, { routing: 5, pricing: 3 })
})

test('mapping an upstream model preserves its reviewed advanced contract without overwriting a shared destination', () => {
  const draft = fixture()
  const mapped = updateChannelAttachment(draft, 1, {
    upstream_model: 'mapped',
    priority: 1,
  })
  assert.equal(mapped.contexts.length, 2)
  assert.deepEqual(mapped.contexts[1], {
    ...draft.contexts[0],
    upstream_model: 'mapped',
  })
  assert.equal(mapped.attachments[0].enabled, false)
  assert.deepEqual(mapped.attachments[0].client_protocols, ['openai_responses'])
  assert.deepEqual(mapped.attachments[0].payload, draft.attachments[0].payload)
  const destination = updateChannelAttachment(mapped, 2, {
    upstream_model: 'mapped',
  })
  assert.deepEqual(destination.contexts, [mapped.contexts[1]])
})

test('new bindings require explicit provider limits and retain the route speeds', () => {
  const added = toggleChannelAttachment(
    fixture(),
    { id: 3, model: 'public-c', speeds: ['fast'] },
    true
  )
  assert.equal(added.attachments[2].timeout_ms, 30_000)
  assert.deepEqual(added.attachments[2].speeds, ['fast'])
  assert.equal(added.contexts[1].contract_revision, '')
  assert.equal(added.contexts[1].max_input_tokens, 0)
  assert.deepEqual(
    toggleChannelAttachment(
      added,
      { id: 3, model: 'public-c', speeds: ['fast'] },
      true
    ),
    added
  )
})

test('a model or group rename updates price identities and retains advanced billing data', () => {
  const draft = readModelDraft(
    JSON.stringify({
      expected_version: { routing: 5, pricing: 3 },
      route: {
        id: 1,
        model: 'public-a',
        group: 'default',
        enabled: false,
        targets: [],
        speeds: ['standard'],
        failover: {
          max_attempts: 1,
          retry_on: [],
          allow_replay_after_send: false,
        },
      },
      metadata: {
        display_name: 'public-a',
        description: 'Keep this text',
        vendor_id: null,
      },
      contexts: [],
      prices: [
        {
          id: 1,
          model: 'public-a',
          group: 'default',
          speed: 'standard',
          rates: {
            input: 2,
            output: 3,
            cache_read: 1,
            cache_write: 2,
            request: 0,
          },
          group_multiplier_ppm: 1_000_000,
          speed_multiplier_ppm: 1_000_000,
          billing_mode: 'tiered_decimal',
          tiered: { retained: 'exact' },
        },
      ],
    })
  )
  assert.ok(draft)
  const changed = updateModelIdentity(
    updateModelIdentity(draft, 'model', 'public-b'),
    'group',
    'team'
  )
  assert.equal(changed.route.model, 'public-b')
  assert.equal(changed.prices[0].model, 'public-b')
  assert.equal(changed.prices[0].group, 'team')
  assert.deepEqual(changed.prices[0].tiered, { retained: 'exact' })
  assert.equal(changed.metadata.display_name, 'public-b')
  assert.equal(changed.metadata.description, 'Keep this text')
})

test('malformed advanced input stays outside the visual editor instead of inventing defaults', () => {
  assert.equal(readChannelDraft('{'), null)
  assert.equal(
    readChannelDraft(JSON.stringify({ ...fixture(), attachments: [null] })),
    null
  )
  assert.equal(
    readChannelDraft(JSON.stringify({ ...fixture(), id: 1.5 })),
    null
  )
  assert.equal(readModelDraft('{"route":null}'), null)
})
