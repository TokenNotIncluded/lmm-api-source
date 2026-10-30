/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  nativeAmountMinor,
  nativeAmountDisplay,
  nativeChannelSchema,
  nativeQuoteSchema,
  nativeCheckoutURL,
  nativeCheckoutFinished,
  readNativeCheckout,
  saveNativeCheckout,
  validateNativeQuote,
  validateNativeCheckout,
  type NativeCheckout,
  type PendingNativeCheckout,
} from './native-checkout'

const pending: PendingNativeCheckout = {
  channel: {
    id: '1',
    provider: 'stripe',
    environment: 'live',
    currency: 'USD',
  },
  request: {
    channel_id: '1',
    amount_minor: '9007199254740993',
    currency: 'USD',
    request_key: 'immutable-payment',
  },
  submitted: false,
}
const quote = {
  ...pending.request,
  order_id: 'top_42',
  account_id: '1',
  provider: 'stripe' as const,
  environment: 'live' as const,
  credit_units: '9007199254740993',
  credit_unit: 'credit_500k_usd' as const,
}
const result: NativeCheckout = {
  order_id: quote.order_id,
  state: 'credit_pending',
  currency: 'USD',
  amount_minor: quote.amount_minor,
  credit_units: quote.credit_units,
  credit_unit: quote.credit_unit,
  checkout_method: 'GET',
  provider_paid: true,
  core_credit_confirmed: false,
}

test('native payment decimal conversion remains exact beyond Number safe integers', () => {
  assert.equal(nativeAmountMinor('90071992547409.93'), '9007199254740993')
  assert.equal(nativeAmountDisplay('9007199254740993'), '90071992547409.93')
  assert.equal(nativeAmountMinor('0.01'), '1')
  assert.equal(nativeAmountMinor('01.2'), '120')
  for (const input of [
    '0',
    '-1',
    '0.001',
    'NaN',
    '1e2',
    '92233720368547758.08',
  ]) {
    assert.equal(nativeAmountMinor(input), null)
  }
  assert.equal(
    nativeQuoteSchema.safeParse({
      ...quote,
      credit_units: Number.MAX_SAFE_INTEGER + 1,
    }).success,
    false
  )
  assert.equal(
    nativeQuoteSchema.safeParse({ ...quote, credit_units: 'invalid' }).success,
    false
  )
})
test('quotes and statuses bind to the reviewed request and Core alone confirms credit', () => {
  assert.equal(validateNativeQuote(quote, pending), quote)
  assert.throws(() =>
    validateNativeQuote({ ...quote, currency: 'CNY' }, pending)
  )
  assert.throws(() =>
    validateNativeQuote({ ...quote, request_key: 'other' }, pending)
  )
  assert.throws(() =>
    validateNativeCheckout({ ...result, credit_units: '1' }, quote)
  )
  assert.equal(nativeCheckoutFinished(result), false)
  assert.equal(nativeCheckoutFinished({ ...result, state: 'credited' }), false)
  assert.equal(
    nativeCheckoutFinished({
      ...result,
      state: 'credited',
      core_credit_confirmed: true,
    }),
    true
  )
  assert.equal(nativeCheckoutFinished({ ...result, state: 'review' }), false)
})
test('only supported channels and HTTPS checkout links are accepted', () => {
  assert.equal(
    nativeChannelSchema.safeParse({ ...pending.channel, provider: 'fastpay' })
      .success,
    false
  )
  assert.equal(
    nativeChannelSchema.safeParse({ ...pending.channel, provider: 'creem' })
      .success,
    false
  )
  assert.equal(
    nativeChannelSchema.safeParse({
      ...pending.channel,
      provider: 'creem',
      currency: 'EUR',
      product_id: 'prod_1',
      fixed_amount_minor: '100',
    }).success,
    true
  )
  for (const url of [
    'javascript:alert(1)',
    'http://pay.example',
    'https://user:secret@pay.example',
  ]) {
    assert.equal(nativeCheckoutURL(url), null)
  }
  assert.equal(
    nativeCheckoutURL('https://pay.example/checkout'),
    'https://pay.example/checkout'
  )
})
test('persisted financial intent stays scoped to one login and keeps its original key', () => {
  const values = new Map<string, string>()
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value)
    },
    removeItem: (key: string) => {
      values.delete(key)
    },
  }
  saveNativeCheckout(
    '[1,"sid-a"]',
    { ...pending, quote, submitted: true },
    storage
  )
  assert.equal(
    readNativeCheckout('[1,"sid-a"]', storage)?.request.request_key,
    'immutable-payment'
  )
  assert.equal(readNativeCheckout('[2,"sid-b"]', storage), null)
  assert.equal(readNativeCheckout('[1,"sid-new"]', storage), null)
  saveNativeCheckout('[1,"sid-a"]', null, storage)
  assert.equal(readNativeCheckout('[1,"sid-a"]', storage), null)
})
