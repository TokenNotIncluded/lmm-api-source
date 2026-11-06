/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createCliHeaderTemplate } from './cli-header-templates'

test('CLI templates preserve the protocol headers, ordering and passthrough operation', () => {
  const expected = {
    codex: [
      'Originator',
      'Session_id',
      'Thread_id',
      'Session-Id',
      'Thread-Id',
      'X-Client-Request-Id',
      'User-Agent',
      'X-Codex-Beta-Features',
      'X-Codex-Turn-State',
      'X-Codex-Turn-Metadata',
      'X-Codex-Window-Id',
      'X-Codex-Parent-Thread-Id',
      'X-OpenAI-Subagent',
      'X-OpenAI-Memgen-Request',
      'X-ResponsesAPI-Include-Timing-Metrics',
      'X-OpenAI-Internal-Codex-Responses-Lite',
    ],
    claude: [
      'X-Stainless-Arch',
      'X-Stainless-Lang',
      'X-Stainless-Os',
      'X-Stainless-Package-Version',
      'X-Stainless-Retry-Count',
      'X-Stainless-Runtime',
      'X-Stainless-Runtime-Version',
      'X-Stainless-Timeout',
      'User-Agent',
      'X-App',
      'Anthropic-Beta',
      'Anthropic-Dangerous-Direct-Browser-Access',
      'Anthropic-Version',
    ],
  }
  for (const client of ['codex', 'claude'] as const) {
    assert.deepEqual(createCliHeaderTemplate(client), {
      operations: [
        { mode: 'pass_headers', value: expected[client], keep_origin: true },
      ],
    })
  }
})

test('editing one CLI template cannot modify another consumer or a later template', () => {
  for (const client of ['codex', 'claude'] as const) {
    const original = createCliHeaderTemplate(client)
    const edited = createCliHeaderTemplate(client)
    edited.operations[0].value.splice(0, 1, 'X-Custom-Header')
    edited.operations[0].keep_origin = false
    edited.operations.push({ mode: 'delete', value: [], keep_origin: false })
    assert.deepEqual(createCliHeaderTemplate(client), original)
  }
})
