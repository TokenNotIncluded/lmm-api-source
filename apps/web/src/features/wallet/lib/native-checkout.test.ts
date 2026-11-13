/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  nativeAmountMinor,
  nativeRequestedCreditUnits,
  nativeDiscountCode,
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

test('Coinbase channels keep USD quote units while accepting test and live profiles', () => {
  for (const environment of ['test', 'live']) {
    assert.equal(
      nativeChannelSchema.safeParse({
        ...pending.channel,
        provider: 'coinbase',
        environment,
      }).success,
      true
    )
  }
  for (const currency of ['USDC', 'CNY', 'EUR', 'LDC']) {
    assert.equal(
      nativeChannelSchema.safeParse({
        ...pending.channel,
        provider: 'coinbase',
        currency,
      }).success,
      false
    )
  }
})

test('credit quantity preserves source USD units and trusts Core CNY pricing', () => {
  assert.equal(nativeRequestedCreditUnits('1'), '500000')
  assert.equal(nativeRequestedCreditUnits('3.5'), '1750000')
  assert.equal(nativeRequestedCreditUnits('100'), '50000000')
  assert.equal(nativeRequestedCreditUnits('0.000003'), '1')
  for (const value of ['0', '-1', '1e2', '1.0000001']) {
    assert.equal(nativeRequestedCreditUnits(value), null)
  }
  const saved: PendingNativeCheckout = {
    channel: {
      id: '5003',
      provider: 'waffo_pancake',
      environment: 'test',
      currency: 'CNY',
      credit_quantity: true,
    },
    request: {
      channel_id: '5003',
      amount_minor: '0',
      requested_credit_units: '1000000',
      currency: 'CNY',
      request_key: 'quantity_browser_fixture_01',
    },
    submitted: false,
  }
  const quote = nativeQuoteSchema.parse({
    ...saved.request,
    amount_minor: '1327',
    order_id: 'top_31',
    account_id: '42',
    provider: 'waffo_pancake',
    environment: 'test',
    credit_units: '1000000',
    credit_unit: 'credit_500k_usd',
  })
  assert.equal(validateNativeQuote(quote, saved).amount_minor, '1327')
  assert.throws(() =>
    validateNativeQuote({ ...quote, credit_units: '999999' }, saved)
  )
  const values = new Map<string, string>()
  const storage = {
    setItem: (key: string, value: string) => {
      values.set(key, value)
    },
    getItem: (key: string) => values.get(key) ?? null,
    removeItem: (key: string) => {
      values.delete(key)
    },
  }
  saveNativeCheckout('quantity-owner', { ...saved, quote }, storage)
  assert.deepEqual(readNativeCheckout('quantity-owner', storage), {
    ...saved,
    quote,
  })
})

test('discounted credit quote binds its code and frozen amount across restoration', () => {
  assert.equal(nativeDiscountCode(' save-10 '), 'SAVE-10')
  assert.equal(nativeDiscountCode(' '), '')
  for (const value of ['ßAVE', 'A', 'BAD CODE', 'X'.repeat(65)]) {
    assert.equal(nativeDiscountCode(value), null)
  }
  const discounted: PendingNativeCheckout = {
    ...pending,
    channel: { ...pending.channel, credit_quantity: true },
    request: {
      ...pending.request,
      amount_minor: '0',
      requested_credit_units: '5000000',
      discount_code: 'SAVE-10',
    },
  }
  const frozen = nativeQuoteSchema.parse({
    ...quote,
    ...discounted.request,
    amount_minor: '900',
    credit_units: '5000000',
  })
  assert.equal(validateNativeQuote(frozen, discounted).amount_minor, '900')
  assert.throws(() =>
    validateNativeQuote({ ...frozen, discount_code: 'OTHER' }, discounted)
  )
  assert.throws(() =>
    validateNativeQuote({ ...frozen, discount_code: undefined }, discounted)
  )
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
  const saved = { ...discounted, quote: frozen }
  saveNativeCheckout('discount-owner', saved, storage)
  assert.deepEqual(readNativeCheckout('discount-owner', storage), saved)
})
