/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider, initReactI18next } from 'react-i18next'

import { parseNativeDocument, type NativePrices } from '../native-api'
import { NativePriceRuleList } from './native-price-rule-list'

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const rule = {
  id: 1,
  model: 'synthetic-model',
  group: 'default',
  speed: 'standard',
  rates: {
    input: 1,
    output: 1250000,
    cache_read: 0,
    cache_write: 250000,
    request: 10000,
  },
  group_multiplier_ppm: 1250000,
  speed_multiplier_ppm: 2000000,
}
function render(rules: NativePrices['rules']) {
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <NativePriceRuleList prices={{ version: 4, rules }} />
    </I18nextProvider>
  )
}
const amounts = (html: string) =>
  [...html.matchAll(/<dd[^>]*>(.*?)<\/dd>/g)].map((match) => match[1])

test('published rates keep exact decimals, separate units and a real zero', () => {
  const html = render([rule])
  assert.deepEqual(amounts(html), [
    '0.000001',
    '1.25',
    '0',
    '0.25',
    '0.01',
    '1.25 × 2',
  ])
  assert.match(html, /Input price \(USD \/ 1M tokens\)/)
  assert.match(html, /Request fee \(USD \/ request\)/)
  assert.match(html, /Published version/)
  assert.match(html, /Token pricing/)
})

test('tiered summaries preserve repeating fractions and large exact decimal numerators', () => {
  const exact = (
    parseNativeDocument(
      '{"tiered":{"rates":{"input":{"numerator":9223372036854775807,"denominator":1000000000000000000}}}}'
    ).tiered as { rates: { input: unknown } }
  ).rates.input
  const html = render([
    {
      ...rule,
      billing_mode: 'tiered_decimal',
      rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
      tiered: {
        tiers: [
          {
            rates: {
              input: exact,
              output: { numerator: 1, denominator: 3 },
              cache_read: { numerator: 0, denominator: 1 },
              cache_write: { numerator: 1, denominator: 2 },
            },
          },
          {
            rates: {
              input: { numerator: 10, denominator: 1 },
              output: { numerator: 1, denominator: 3 },
              cache_read: { numerator: 0, denominator: 1 },
              cache_write: { numerator: 1, denominator: 2 },
            },
          },
        ],
      },
    },
  ])
  assert.deepEqual(amounts(html), [
    '9.223372036854775807 / 10',
    '1/3',
    '0',
    '0.5',
    '—',
    '1.25 × 2',
  ])
  assert.match(html, /Tiered pricing/)
})

test('upstream pricing placeholders are inapplicable and never displayed as free prices', () => {
  const html = render([
    {
      ...rule,
      billing_mode: 'upstream_cost',
      rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
    },
  ])
  assert.deepEqual(amounts(html).slice(0, 5), ['—', '—', '—', '—', '—'])
  assert.match(html, /Upstream cost/)
})

test('unsupported or incomplete rate information remains unknown', () => {
  const html = render([{ ...rule, rates: { ...rule.rates, input: undefined } }])
  assert.deepEqual(amounts(html), [
    'Unknown',
    'Unknown',
    'Unknown',
    'Unknown',
    'Unknown',
    'Unknown',
  ])
  assert.match(html, /synthetic-model/)
})
