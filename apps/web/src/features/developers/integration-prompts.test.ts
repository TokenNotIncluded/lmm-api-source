/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { parse } from '@babel/parser'

import {
  OAUTH_PROMPT,
  buildPricingExample,
  buildPricingPrompt,
} from './integration-prompts'

const rootUrl = 'https://relay.example.test:8443/gateway'
const PRICING_EXAMPLE = buildPricingExample(rootUrl)
const PRICING_PROMPT = buildPricingPrompt(rootUrl)

test('copyable pricing instructions use current native catalog units and sources', () => {
  assert.match(PRICING_EXAMPLE, /\/core\/v1\/pricing\/public/)
  assert.match(PRICING_EXAMPLE, /catalog\.schema_version !== 1/)
  assert.match(PRICING_PROMPT, /\/core\/v1\/pricing\/session/)
  assert.match(PRICING_PROMPT, /rate_basis/)
  assert.match(PRICING_PROMPT, /do not multiply them twice/)
  assert.match(PRICING_PROMPT, /integer-ratio prices per image/)
  assert.doesNotMatch(
    PRICING_PROMPT + PRICING_EXAMPLE,
    /GET \/api\/pricing|blob\/main\/apps\/lmm-extensions/
  )
})

test('OAuth instructions limit clients, require an explicit loopback port and distinguish API keys', () => {
  assert.match(
    OAUTH_PROMPT,
    /read-only lmm client and optionally enabled lmm-pi/
  )
  assert.match(OAUTH_PROMPT, /127\.0\.0\.1:<nonzero-port>/)
  assert.match(OAUTH_PROMPT, /Claude\/Gemini native examples use API keys/)
  assert.doesNotMatch(OAUTH_PROMPT, /lmm-dsh|blob\/main\/apps\/lmm-extensions/)
})

test('configured pricing URLs keep path prefixes without redefining the trusted OAuth issuer', () => {
  assert.ok(PRICING_PROMPT.includes(`Provider: ${rootUrl}`))
  assert.ok(PRICING_EXAMPLE.includes(`${rootUrl}/core/v1/pricing/public`))
  assert.doesNotMatch(
    PRICING_PROMPT + PRICING_EXAMPLE,
    /https:\/\/api\.lmm\.best/
  )
  assert.match(OAUTH_PROMPT, /Trusted issuer: https:\/\/api\.lmm\.best/)
  const quoted = buildPricingExample("https://relay.test/customer's-gateway")
  assert.doesNotThrow(() => parse(quoted, { sourceType: 'module' }))
})
