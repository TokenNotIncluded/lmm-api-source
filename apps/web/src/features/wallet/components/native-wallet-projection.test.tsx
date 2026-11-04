/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'

import type { UserWalletData } from '../types'
import { AffiliateRewardsCard } from './affiliate-rewards-card'
import { WalletStatsCard } from './wallet-stats-card'

const user = {
  id: 911,
  username: 'synthetic-native',
  financial_projection_available: true,
  usage_projection_available: false,
  wallet_unit: 'core_credit',
  wallet_money_basis: 'unknown',
  wallet_money_available: false,
  quota: 199671,
  wallet_spendable_quota: 199671,
  wallet_reserved_quota: 280,
  wallet_debt: 0,
} as UserWalletData
function stats(value: UserWalletData) {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <WalletStatsCard
        user={value}
        balanceAction={<button type='button'>unsafe-action</button>}
      />
    </QueryClientProvider>
  )
}
test('actual stats component renders native amounts, unknown history and no legacy money tooltip', () => {
  const html = stats(user)
  assert.match(html, /199,671 Credits/)
  assert.match(html, /280 Credits/)
  assert.match(html, /wallet_debt/)
  assert.match(html, /Credit conversion unavailable/)
  assert.match(html, /USD \/ CNY/)
  assert.match(html, /Unavailable/)
  assert.doesNotMatch(html, /unsafe-action|wallet-token-cloud|\$0\.40|1 USD/)
})
test('negative economic position is not clamped into normal zero balance', () => {
  const html = stats({
    ...user,
    quota: -70,
    wallet_spendable_quota: 0,
    wallet_debt: 70,
  })
  assert.match(html, /-70 Credits/)
  assert.match(html, /70 Credits/)
  assert.match(html, /0 Credits/)
})
test('unknown native amount never falls back to a zero', () => {
  const html = stats({
    ...user,
    quota: undefined,
    wallet_spendable_quota: undefined,
    wallet_reserved_quota: undefined,
    wallet_debt: undefined,
  } as unknown as UserWalletData)
  assert.match(html, /Unknown/)
  assert.doesNotMatch(html, /0 Credits/)
})
function rewards(value: UserWalletData, complianceConfirmed = true) {
  return renderToStaticMarkup(
    <AffiliateRewardsCard
      user={value}
      affiliateLink=''
      complianceConfirmed={complianceConfirmed}
      onTransfer={() => {
        throw new Error('Native navigation must not use the legacy transfer')
      }}
    />
  )
}

test('native rewards link navigates to the same-origin authoritative preview without submitting money', () => {
  const html = rewards({
    ...user,
    aff_quota: 900000,
    aff_history_quota: 123456789,
  })
  assert.match(html, /900,000 Credits/)
  assert.match(
    html,
    /<a[^>]*href="\/core\/v1\/affiliate\/claim"[^>]*>Preview reward transfer<\/a>/
  )
  assert.match(html, /Unavailable/)
  assert.doesNotMatch(
    html,
    /123,456,789|amount_credits|aff_transfer|target=|<form/
  )
  assert.doesNotMatch(html, /<button[^>]*>Transfer to Balance<\/button>/)
})

test('missing or unavailable affiliate balance still offers preview without inventing an amount', () => {
  for (const value of [
    user,
    { ...user, financial_projection_available: false, aff_quota: 900000 },
    { ...user, aff_quota: Number.MAX_SAFE_INTEGER + 1 },
    { ...user, wallet_unit: 'unknown', aff_quota: 900000 },
  ]) {
    const html = rewards(value)
    assert.match(html, /href="\/core\/v1\/affiliate\/claim"/)
    assert.doesNotMatch(html, /900,000|0 Credits|amount_credits/)
  }
})

test('native referral debt uses credits and preserves access to authoritative preview', () => {
  const html = rewards({ ...user, aff_quota: 0, aff_debt: 70 })
  assert.match(html, /Reward debt/)
  assert.match(html, /70 Credits/)
  assert.match(html, /href="\/core\/v1\/affiliate\/claim"/)
})

test('native preview preserves the compliance gate', () => {
  const html = rewards({ ...user, aff_quota: 900000 }, false)
  assert.doesNotMatch(html, /href="\/core\/v1\/affiliate\/claim"/)
  assert.match(html, /disabled=""/)
  assert.match(html, /administrator confirms compliance terms/)
})
