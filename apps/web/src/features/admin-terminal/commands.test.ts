/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { NativeConfiguration } from '@/features/channels/native-api'

import { catalogCoverage } from './catalog'
import { help, parseScript, safeScript } from './commands'

test('terminal validates all commands before execution and retains only registered arguments', () => {
  assert.throws(
    () => parseScript('help\nlogs.query {"http_status":900}'),
    /line 2/
  )
  assert.throws(
    () => parseScript('logs.query {"password":"dummy"}'),
    /unrecognized_keys/
  )
  assert.throws(() => parseScript('fetch {}'), /unknown_command/)
  const script = parseScript(
    '# one request\nlogs.query {"request_id":"sk-secretvalue","settlement_status":"pending","http_status":400}'
  )
  assert.equal(script[0].line, 2)
  assert.ok(!safeScript(script).includes('secretvalue'))
  assert.ok(Object.hasOwn(help, 'changes.confirm'))
})
test('catalog compares upstream IDs and checks price coverage for mapped public models', () => {
  const config = {
    routing: {
      routes: [
        {
          model: 'public',
          group: 'a',
          speeds: ['standard'],
          targets: [{ upstream_id: 1, upstream_model: 'raw' }],
        },
      ],
    },
    prices: { rules: [{ model: 'public', group: 'a', speed: 'standard' }] },
    bindings: {
      discoveries: { 1: { models: ['gone', 'raw'] } },
      official_prices: { models: { waiting: {} } },
    },
  } as unknown as NativeConfiguration
  assert.deepEqual(catalogCoverage(config, 1, ['raw', 'new', 'waiting']), {
    missing_price_routes: [],
    unbound: ['new', 'waiting'],
    newly_discovered: ['new', 'waiting'],
    onboarded: ['raw'],
    no_longer_offered: ['gone'],
    missing_price: ['new'],
  })
})
