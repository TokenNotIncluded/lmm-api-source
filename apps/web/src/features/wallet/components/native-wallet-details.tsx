/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import { ChevronDown } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import {
  nativeCreditDisplay,
  walletProjection,
  type WalletProjectionUser,
} from '../lib/wallet-projection'

/** Ledger details stay distinct from the current-rate reference display. */
export function NativeWalletDetails({
  user,
}: {
  user: WalletProjectionUser | null | undefined
}) {
  const { t } = useTranslation()
  const projection = walletProjection(user)
  return (
    <details
      className='group relative z-10 mt-4 text-xs'
      data-native-wallet-details
    >
      <summary className='text-muted-foreground hover:text-foreground focus-visible:outline-ring flex min-h-8 w-fit cursor-pointer list-none items-center gap-2 rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2'>
        {t('Balance details')}
        <ChevronDown
          className='size-3.5 transition-transform group-open:rotate-180'
          aria-hidden='true'
        />
      </summary>
      <dl className='mt-3 grid grid-cols-1 gap-4 sm:grid-cols-3'>
        {(
          [
            [
              'wallet_spendable_quota',
              t('Spendable credits'),
              projection.spendable,
            ],
            [
              'wallet_reserved_quota',
              t('Reserved credits'),
              projection.reserved,
            ],
            ['wallet_debt', t('Credit debt'), projection.debt],
          ] as const
        ).map(([field, label, value]) => (
          <div key={field} data-wallet-bucket={field}>
            <dt className='text-muted-foreground break-all'>{label}</dt>
            <dd className='mt-2 font-medium break-words tabular-nums'>
              {nativeCreditDisplay(value, t('Credits'), t('Unknown'))}
            </dd>
          </div>
        ))}
      </dl>
      <p className='text-muted-foreground mt-4 max-w-prose leading-relaxed'>
        {t(
          'Reference values use the current pricing rate. Your balance is recorded in credits.'
        )}
      </p>
    </details>
  )
}
