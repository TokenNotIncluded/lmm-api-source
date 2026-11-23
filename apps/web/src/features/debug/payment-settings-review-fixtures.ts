/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { AxiosError, type AxiosAdapter } from 'axios'

import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

// Read-only visual fixtures. Production never imports this debug entry;
// every identifier is synthetic and no payment request reaches a backend.
export function withPaymentSettingsReviewFixtures(
  next: AxiosAdapter
): AxiosAdapter {
  if (
    window.location.protocol !== 'http:' ||
    window.location.hostname !== '127.0.0.1' ||
    new URLSearchParams(window.location.search).get('payment_review') !== '1'
  ) {
    return next
  }

  return async (config) => {
    const url = new URL(config.url ?? '', window.location.origin)
    const method = (config.method ?? 'get').toUpperCase()
    const paymentPath =
      url.pathname.startsWith('/api/option/waffo-pancake/') ||
      url.pathname.startsWith('/api/option/epay/')
    if (
      url.origin !== window.location.origin ||
      !paymentPath ||
      useAuthStore.getState().auth.user?.role !== ROLE.SUPER_ADMIN
    ) {
      return next(config)
    }
    const respond = (data: unknown) => ({
      data: { success: true, data: structuredClone(data) },
      status: 200,
      statusText: 'OK',
      headers: {},
      config,
    })

    if (
      method === 'GET' &&
      url.pathname === '/api/option/waffo-pancake/config'
    ) {
      return respond({
        contract: 'extensions.pancake-configuration.v1',
        callbacks_enabled: false,
        bindings: [
          {
            channel_id: 51,
            merchant_id: 'MER_synthetic_preview',
            store_id: 'STO_synthetic_preview',
            product_id: 'PROD_synthetic_preview',
            return_url: 'https://native.example.invalid/wallet',
            environment: 'live',
            private_key_configured: true,
            channels: [
              {
                id: 51,
                currency: 'USD',
                enabled: false,
                subscription_enabled: false,
              },
              {
                id: 53,
                currency: 'CNY',
                enabled: false,
                subscription_enabled: false,
              },
            ],
          },
        ],
      })
    }
    if (method === 'GET' && url.pathname === '/api/option/epay/config') {
      return respond({
        contract: 'core.epay-configuration.v1',
        revision: 'a'.repeat(64),
        bindings: [
          {
            channel_id: 55,
            merchant_id: '123',
            environment: 'live',
            currency: 'LDC',
            payment_type: 'epay',
            endpoint: 'https://credit.linux.do/epay',
            notify_url: 'https://native.example.invalid/callbacks/55',
            return_url: 'https://native.example.invalid/wallet',
            key_configured: true,
            enabled: false,
            editable: true,
            credits_per_minor: '500',
            max_amount_minor: '500000000',
            min_amount_minor: '10000',
            adapter_configured: true,
            adapter_matches: true,
            wallet_enabled: false,
            subscription_enabled: false,
          },
        ],
      })
    }
    throw new AxiosError(
      'Payment review is read-only; this operation was not sent.',
      'ERR_BAD_REQUEST',
      config,
      undefined,
      {
        data: { success: false, code: 'SYNTHETIC_READ_ONLY' },
        status: 405,
        statusText: 'Method Not Allowed',
        headers: {},
        config,
      }
    )
  }
}
