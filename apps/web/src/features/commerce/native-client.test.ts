/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import {
  claimCampaign,
  issueCodes,
  redeemCode,
  type NativeCampaign,
} from '@/features/promotions/native-api'
import {
  confirmNativeRefund,
  confirmNativeSubscription,
  quoteNativeSubscription,
  type NativeQuote,
} from '@/features/subscriptions/native-api'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { commerceScope, positiveUnits } from './native-client'

const original = useAuthStore.getState().auth
const adapter = api.defaults.adapter
function login(sid = 'synthetic-commerce-session') {
  useAuthStore.getState().auth.setBundle({
    user: { id: 51, role: 100, username: 'synthetic-commerce' },
    access_token: 'synthetic-commerce-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'test',
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
  const scope = commerceScope()
  assert.ok(scope)
  return scope
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: original })
})
const quote: NativeQuote = {
  id: 'a'.repeat(64),
  plan: {
    id: 3,
    revision: 2,
    archived: false,
    terms: {
      title: 'Synthetic plan',
      price_units: '999999999999',
      limit_units: '1000',
      duration_seconds: 3600,
      reset_period: 'grant',
      reset_seconds: 0,
      models: ['*'],
      groups: ['*'],
      max_purchases_per_user: 0,
      enabled: true,
    },
  },
  starts_at: 1700000000,
  ends_at: 1700003600,
  expires_at: 1700000300,
  renew_from: null,
}
const receipt = {
  quote_id: quote.id,
  status: 'posted',
  purchase_id: 8,
  subscription_id: 9,
  journal_id: 10,
  amount_units: quote.plan.terms.price_units,
  rejection: null,
}
const campaign: NativeCampaign = {
  id: 'b'.repeat(64),
  revision: 1,
  spec: {
    kind: 'checkin',
    title: 'Synthetic',
    award_units: '100',
    budget_units: '300',
    max_claims: 3,
    max_claims_per_user: 1,
    starts_at: 1,
    ends_at: 9999999999,
    enabled: true,
  },
  spent_units: '0',
  reserved_units: '0',
  claims: 0,
  reserved_claims: 0,
}

test('subscription API freezes exact quote and explicit commit uses only its original identity', async () => {
  const scope = login()
  const request = {
    operation_key: 'synthetic-quote-operation-0001',
    plan_id: 3,
    renew_from: null,
  }
  const sent: string[] = []
  api.defaults.adapter = async (config) => {
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(config.authScope, scope)
    sent.push(config.url!)
    if (config.url?.endsWith('/quotes')) {
      assert.deepEqual(JSON.parse(String(config.data)), request)
    }
    return {
      config,
      data: config.url?.endsWith('/quotes') ? quote : receipt,
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  assert.deepEqual(await quoteNativeSubscription(scope, request), quote)
  assert.equal(sent.length, 1, 'preparing a quote must not spend')
  assert.deepEqual(await confirmNativeSubscription(scope, quote), receipt)
  assert.equal(sent[1], `/core/v1/subscriptions/quotes/${quote.id}/confirm`)
  for (const value of ['0', '01', '+1', '1.5', '1e6', '1000000000001']) {
    assert.equal(positiveUnits.safeParse(value).success, false)
  }
})

test('wrong identities or amounts cannot become purchase and refund success', async () => {
  const scope = login()
  for (const data of [
    { ...receipt, quote_id: 'c'.repeat(64) },
    { ...receipt, amount_units: '1' },
    { ...receipt, journal_id: null },
    { ...receipt, status: 'refunded' },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      data,
      headers: {},
      status: 200,
      statusText: 'OK',
    })
    await assert.rejects(
      confirmNativeSubscription(scope, quote),
      /INVALID_RESPONSE/
    )
  }
  api.defaults.adapter = async (config) => ({
    config,
    data: { ...receipt, status: 'refunded', purchase_id: 999 },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await assert.rejects(
    confirmNativeRefund(scope, {
      id: quote.id,
      purchase_id: 8,
      amount_units: receipt.amount_units,
      expires_at: 1700000300,
    }),
    /INVALID_RESPONSE/
  )
})

test('ambiguous commits are not replayed automatically and a new session cannot reuse old scope', async () => {
  const scope = login()
  const paths: string[] = []
  api.defaults.adapter = async (config) => {
    paths.push(config.url!)
    if (paths.length === 1) throw new Error('commit response lost')
    return { config, data: receipt, headers: {}, status: 200, statusText: 'OK' }
  }
  await assert.rejects(
    confirmNativeSubscription(scope, quote),
    /RESULT_UNKNOWN/
  )
  assert.equal(paths.length, 1)
  await confirmNativeSubscription(scope, quote)
  assert.equal(paths[0], paths[1])
  login('replacement-commerce-session')
  await assert.rejects(
    confirmNativeSubscription(scope, quote),
    /SESSION_CHANGED/
  )
  assert.equal(paths.length, 2)
})

test('reward requests carry only campaign/code and stable key, and validate returned entitlement', async () => {
  const scope = login()
  const operation_key = 'synthetic-reward-operation-0001'
  const result = {
    id: 'd'.repeat(64),
    campaign_id: campaign.id,
    amount_units: '100',
    journal_id: 13,
    claimed_at: 1700000000,
  }
  const code = `lmmr_${'A'.repeat(43)}`
  api.defaults.adapter = async (config) => {
    assert.deepEqual(
      JSON.parse(String(config.data)),
      config.url?.endsWith('/redemptions')
        ? { operation_key, code }
        : { operation_key }
    )
    return { config, data: result, headers: {}, status: 200, statusText: 'OK' }
  }
  assert.deepEqual(await claimCampaign(scope, campaign, operation_key), result)
  assert.deepEqual(await redeemCode(scope, operation_key, code), result)
  api.defaults.adapter = async (config) => ({
    config,
    data: { ...result, amount_units: '200' },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await assert.rejects(
    claimCampaign(scope, campaign, operation_key),
    /INVALID_RESPONSE/
  )
  api.defaults.adapter = async (config) => ({
    config,
    data: {
      id: 'd'.repeat(64),
      campaign_id: campaign.id,
      codes: [{ id: 'e'.repeat(64), code }],
    },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await assert.rejects(
    issueCodes(
      scope,
      { ...campaign, spec: { ...campaign.spec, kind: 'redemption' } },
      { operation_key, quantity: 2 }
    ),
    /INVALID_RESPONSE/
  )
})
