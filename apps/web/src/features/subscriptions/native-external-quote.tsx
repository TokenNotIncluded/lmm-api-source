/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { formatCredits } from '@/features/commerce/native-client'
import { nativeAmountDisplay } from '@/features/wallet/lib/native-checkout'

import type { NativePlan } from './native-api'

/** Values come from the frozen server order, never the live price catalog. */
export function NativeExternalQuote({
  plan,
  orderId,
  currency,
  amountMinor,
  expiresAt,
}: {
  plan: NativePlan
  orderId: string
  currency: string
  amountMinor: string
  expiresAt: string
}) {
  const { t, i18n } = useTranslation()
  return (
    <>
      <p>
        {plan.terms.title} · #{plan.revision}
      </p>
      <p className='break-all'>
        {t('Order ID')}: {orderId}
      </p>
      <p>
        {t('Amount')}: {currency} {nativeAmountDisplay(amountMinor)}
      </p>
      <p>
        {t('Quota')}: {formatCredits(plan.terms.limit_units, i18n.language)}{' '}
        {t('Core credits')} ·{' '}
        {(plan.terms.duration_months ?? 0) > 0
          ? `${plan.terms.duration_months} ${t('months')}`
          : `${t('Duration (seconds)')}: ${plan.terms.duration_seconds}`}
      </p>
      <p>
        {t('Quota reset period')}:{' '}
        {t(
          {
            grant: 'Never',
            day: 'Daily',
            week: 'Weekly',
            month: 'Monthly',
            anniversary_month: 'Anniversary month',
            custom: 'Custom',
          }[plan.terms.reset_period]
        )}
        {plan.terms.reset_period === 'custom'
          ? ` (${plan.terms.reset_seconds} s)`
          : ''}
      </p>
      <p className='break-words'>
        {t('Models')}: {plan.terms.models.join(', ')} · {t('Groups')}:{' '}
        {plan.terms.groups.join(', ')}
      </p>
      <p>
        {t('Expires')}: {new Date(Number(expiresAt) * 1000).toLocaleString()}
      </p>
    </>
  )
}
