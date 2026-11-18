/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  DEFAULT_ASSISTANT_TOOL_POLICY,
  assistantPolicyMatchesCatalog,
  assistantPolicyMatchesBackendCatalog,
  assistantToolPolicySchema,
  isAssistantToolEnabled,
  parseAssistantToolCatalog,
  parseAssistantToolPolicy,
  updateAssistantToolPolicy,
} from './assistant-tool-policy'

const catalogPayload = {
  success: true,
  data: {
    groups: [
      {
        id: 'api_keys',
        label: 'API keys',
        tools: [
          {
            name: 'list_my_keys',
            label: 'List API keys',
            description: 'View key settings.',
            effect: 'read_only',
            access: 'user',
          },
          {
            name: 'create_my_key',
            label: 'Create an API key',
            description: 'Create a key after confirmation.',
            effect: 'confirmation',
            access: 'user',
          },
        ],
      },
    ],
  },
}

test('known switches match the catalog and reject unknown tools or groups', () => {
  const policy = parseAssistantToolPolicy(
    '{"version":1,"groups":{"api_keys":false},"tools":{"list_my_keys":false}}'
  )
  assert.ok(policy)
  assert.equal(
    assistantPolicyMatchesBackendCatalog(policy, catalogPayload),
    true
  )
  for (const scope of ['groups', 'tools'] as const) {
    assert.equal(
      assistantPolicyMatchesBackendCatalog(
        {
          ...policy,
          [scope]: { ...policy[scope], new_server_identifier: false },
        },
        catalogPayload
      ),
      false
    )
  }
})

test('legacy empty policy and missing maps retain the default enabled behavior', () => {
  for (const raw of [
    '',
    '   ',
    DEFAULT_ASSISTANT_TOOL_POLICY,
    '{"version":1}',
  ]) {
    const policy = parseAssistantToolPolicy(raw)
    assert.deepEqual(policy, { version: 1, groups: {}, tools: {} })
    assert.ok(policy)
    assert.equal(
      isAssistantToolEnabled(policy, 'api_keys', 'list_my_keys'),
      true
    )
  }
})

test('group disable wins over a true tool override and enabling restores saved individual choices', () => {
  const initial = parseAssistantToolPolicy(
    '{"version":1,"groups":{},"tools":{"list_my_keys":true,"create_my_key":false}}'
  )
  assert.ok(initial)
  const disabled = parseAssistantToolPolicy(
    updateAssistantToolPolicy(initial, 'groups', 'api_keys', false)
  )
  assert.ok(disabled)
  assert.equal(
    isAssistantToolEnabled(disabled, 'api_keys', 'list_my_keys'),
    false
  )
  assert.deepEqual(disabled.tools, initial.tools)
  const enabled = parseAssistantToolPolicy(
    updateAssistantToolPolicy(disabled, 'groups', 'api_keys', true)
  )
  assert.ok(enabled)
  assert.equal(
    isAssistantToolEnabled(enabled, 'api_keys', 'list_my_keys'),
    true
  )
  assert.equal(
    isAssistantToolEnabled(enabled, 'api_keys', 'create_my_key'),
    false
  )
  assert.deepEqual(enabled.tools, initial.tools)
  assert.deepEqual(initial.groups, {})
})

test('rejects malformed policies, duplicate or escaped duplicate keys and oversized bytes', () => {
  for (const raw of [
    'null',
    '[]',
    '{',
    '{"version":2}',
    '{"version":1,"groups":null}',
    '{"version":1,"tools":{"list_my_keys":"false"}}',
    '{"version":1,"unexpected":{}}',
    '{"version":1,"version":1}',
    '{"version":1,"tools":{"list_my_keys":true,"list_my_keys":false}}',
    '{"version":1,"tools":{"list_my_keys":true,"\\u006cist_my_keys":false}}',
    '{"version":1,"tools":{"__proto__":true}}',
    ' '.repeat(262_145),
    `{"version":1,"${'界'.repeat(87_382)}":false}`,
  ]) {
    assert.equal(parseAssistantToolPolicy(raw), null, raw.slice(0, 120))
    assert.equal(assistantToolPolicySchema.safeParse(raw).success, false)
  }
})

test('enforces the server policy budget in UTF-8 bytes before trimming', () => {
  for (const raw of [' '.repeat(262_144), '\u3000'.repeat(87_381)]) {
    assert.deepEqual(parseAssistantToolPolicy(raw), {
      version: 1,
      groups: {},
      tools: {},
    })
    assert.equal(assistantToolPolicySchema.safeParse(raw).success, true)
  }
  for (const raw of [' '.repeat(262_145), '\u3000'.repeat(87_382)]) {
    assert.equal(parseAssistantToolPolicy(raw), null)
    assert.equal(assistantToolPolicySchema.safeParse(raw).success, false)
  }
})

test('requires a successful complete catalog and rejects ambiguous identifiers or metadata', () => {
  const groups = parseAssistantToolCatalog(catalogPayload)
  assert.ok(groups)
  assert.equal(groups[0].tools.length, 2)
  const malformed = [
    null,
    { data: catalogPayload.data },
    { ...catalogPayload, success: false },
    { success: true, data: { groups: {} } },
    {
      success: true,
      data: {
        groups: [catalogPayload.data.groups[0], catalogPayload.data.groups[0]],
      },
    },
    {
      success: true,
      data: {
        groups: [
          {
            ...catalogPayload.data.groups[0],
            tools: [
              catalogPayload.data.groups[0].tools[0],
              catalogPayload.data.groups[0].tools[0],
            ],
          },
        ],
      },
    },
    {
      success: true,
      data: {
        groups: [
          {
            ...catalogPayload.data.groups[0],
            tools: [
              { ...catalogPayload.data.groups[0].tools[0], access: 'public' },
            ],
          },
        ],
      },
    },
    {
      success: true,
      data: {
        groups: [
          {
            ...catalogPayload.data.groups[0],
            tools: [
              { ...catalogPayload.data.groups[0].tools[0], effect: 'write' },
            ],
          },
        ],
      },
    },
  ]
  for (const payload of malformed) {
    assert.equal(parseAssistantToolCatalog(payload), null)
  }
  assert.equal(
    assistantPolicyMatchesCatalog(
      { version: 1, groups: {}, tools: { list_my_keys: false } },
      groups
    ),
    true
  )
  assert.equal(
    assistantPolicyMatchesCatalog(
      { version: 1, groups: { unknown: false }, tools: {} },
      groups
    ),
    false
  )
  assert.equal(
    assistantPolicyMatchesCatalog(
      { version: 1, groups: {}, tools: { unknown: false } },
      groups
    ),
    false
  )
})

test('retired rules are ignored while every group and tool choice is preserved', () => {
  const choices = {
    version: 1,
    groups: { api_keys: false },
    tools: { list_my_keys: true, create_my_key: false },
  }
  for (const rules of [
    {},
    { list_my_keys: { min_level: 1, max_level: 6 } },
    {
      prepare_weekly_discount: {
        min_level: 0,
        max_level: 6,
        discount_percent_by_level: { '1': 25 },
      },
    },
  ]) {
    const raw = JSON.stringify({ ...choices, rules })
    assert.deepEqual(parseAssistantToolPolicy(raw), choices)
    const saved = assistantToolPolicySchema.parse(raw)
    assert.deepEqual(JSON.parse(saved), choices)
    assert.equal(Object.hasOwn(JSON.parse(saved), 'rules'), false)
  }
})

test('catalog permissions distinguish users and administrator roles without member levels', () => {
  const withAccess = (access: string) => ({
    success: true,
    data: {
      groups: [
        {
          ...catalogPayload.data.groups[0],
          tools: [{ ...catalogPayload.data.groups[0].tools[0], access }],
        },
      ],
    },
  })
  for (const access of ['user', 'admin', 'root', 'mixed']) {
    const groups = parseAssistantToolCatalog(withAccess(access))
    assert.ok(groups)
    assert.equal(groups[0].tools[0].access, access)
  }
  for (const access of ['l0', 'l1', 'l5', 'l6']) {
    assert.equal(parseAssistantToolCatalog(withAccess(access)), null)
  }
})
