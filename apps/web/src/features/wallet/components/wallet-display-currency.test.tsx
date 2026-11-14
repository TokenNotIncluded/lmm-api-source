/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { renderToStaticMarkup } from 'react-dom/server'

import { useSystemConfigStore } from '@/stores/system-config-store'
import { useWalletCurrencyPreferenceStore } from '@/stores/wallet-currency-preference-store'

import type { UserWalletData } from '../types'
import { WalletStatsCard } from './wallet-stats-card'

test('wallet balance and known usage remain exact native credits across display preferences and FX rates', () => {
  const config = useSystemConfigStore.getState().config
  const preference = useWalletCurrencyPreferenceStore.getState().preference
  try {
    for (const currency of ['USD', 'CNY', 'CREDIT'] as const) {
      useWalletCurrencyPreferenceStore.getState().setPreference(currency)
      useSystemConfigStore.setState({
        config: {
          ...config,
          currency: { ...config.currency, quotaPerUnit: 300000, cnyPerUsd: 7 },
        },
      })
      const markup = renderToStaticMarkup(
        <WalletStatsCard
          user={
            {
              id: 1,
              username: 'native',
              quota: 500009,
              wallet_unit: 'core_credit',
              financial_projection_available: true,
              usage_projection_available: true,
              normalized_used_quota: 371426713,
              used_quota: 2491782362,
              request_count: 0,
              wallet_spendable_quota: 500009,
              wallet_reserved_quota: 0,
              wallet_debt: 0,
            } as UserWalletData
          }
        />
      )
      assert.match(markup, /500,009 Credits/)
      assert.match(markup, /371,426,713 Credits/)
      assert.doesNotMatch(markup, /2,491,782,362|[$¥￥]|(?:CNY|USD) [0-9]/)
      assert.doesNotMatch(markup, /role="combobox"|wallet-token-cloud/)
    }
  } finally {
    useSystemConfigStore.setState({ config })
    useWalletCurrencyPreferenceStore.getState().setPreference(preference)
  }
})
