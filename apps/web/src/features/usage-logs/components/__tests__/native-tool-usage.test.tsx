/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import { usageLogSchema } from '../../data/schema'
import { parseLogsPage } from '../../lib/history-response'
import { LogRequestSummary } from '../log-request-summary'
import { LogUsageDisplay } from '../log-usage-display'

const i18n = createInstance()
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } })

const base = {
  id: 1,
  user_id: 7,
  created_at: 1,
  type: 2,
  content: 'Completed',
  pricing_dimension: 'tool',
  billing_method: 'per_successful_request',
  tool_kind: 'deepseek_search',
  token_usage_available: false,
  prompt_tokens: null,
  completion_tokens: null,
}

test('native tool logs preserve zero and successful requests without displaying token quantities', () => {
  for (const tool_requests of [0, 1]) {
    const result = parseLogsPage(
      {
        success: true,
        data: {
          items: [{ ...base, tool_requests }],
          total: 1,
          page: 1,
          page_size: 20,
        },
      },
      'common'
    )
    if (result.kind !== 'common') assert.fail('expected common page')
    const log = result.items[0]
    assert.equal(log.tool_requests, tool_requests)
    assert.equal(log.prompt_tokens, null)
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <LogUsageDisplay log={log} other={null} />
        <LogRequestSummary log={log} other={null} onClose={() => {}} />
      </I18nextProvider>
    )
    assert.match(html, new RegExp(`Successful tool requests.*${tool_requests}`))
    assert.match(html, /deepseek_search/)
    assert.doesNotMatch(
      html,
      /Input Tokens|Output Tokens|Cache read tokens|0 \/ 0/
    )
  }
})

test('unknown tool usage remains unknown even when other contains a stale zero', () => {
  const log = usageLogSchema.parse({
    ...base,
    type: 5,
    state: 'reconcile',
    tool_requests: null,
  })
  const html = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <LogUsageDisplay log={log} other={{ tool_requests: 0 }} />
    </I18nextProvider>
  )
  assert.match(html, /Successful tool requests.*Unknown/)
})

test('grounding summary preserves text tokens and separates unbilled retrieval input', () => {
  const log = usageLogSchema.parse({
    ...base,
    pricing_dimension: 'text',
    billing_method: 'measured_usage',
    tool_kind: null,
    tool_requests: null,
    grounded_requests: 1,
    token_usage_available: true,
    prompt_tokens: 100,
    completion_tokens: 10,
  })
  const html = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <LogRequestSummary
        log={log}
        other={{ grounded_requests: 1, tool_input_tokens: 40 }}
        onClose={() => {}}
      />
    </I18nextProvider>
  )
  assert.match(html, /Input Tokens.*100/)
  assert.match(html, /Output Tokens.*10/)
  assert.match(html, /Grounded requests.*1/)
  assert.match(html, /Search retrieval tokens \(not billed\).*40/)
  assert.doesNotMatch(html, /Successful tool requests|>140</)
})

test('native tool and grounding per-request counters reject malformed values', () => {
  for (const field of ['tool_requests', 'grounded_requests']) {
    for (const value of [-1, 2, 0.5, '1']) {
      assert.equal(
        usageLogSchema.safeParse({ ...base, [field]: value }).success,
        false
      )
    }
  }
})
