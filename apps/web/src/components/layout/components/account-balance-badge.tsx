/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/

import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import {
  nativeCreditDisplay,
  walletProjection,
} from '@/features/wallet/lib/wallet-projection'
import { walletReferenceValue } from '@/features/wallet/lib/wallet-reference'
import { useWalletCurrency } from '@/hooks/use-wallet-currency'
import { toIntlLocale } from '@/i18n/languages'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

/** The balance and top-up link stay visible at compact viewport widths. */
export function AccountBalanceBadge({
  className,
  compactMobile = false,
}: {
  className?: string
  compactMobile?: boolean
}) {
  const { t, i18n } = useTranslation()
  const { formatQuota, config, currency } = useWalletCurrency()
  const user = useAuthStore((state) => state.auth.user)
  const projection = walletProjection(user)
  const balance = projection.native
    ? nativeCreditDisplay(projection.available, t('Credits'), t('Unknown'))
    : user?.financial_projection_available === false
      ? t('Unavailable')
      : formatQuota(user?.quota ?? 0)
  const walletAction = projection.native ? t('Wallet') : t('Top up')
  const reference = walletReferenceValue(
    projection.available,
    currency === 'CNY' ? 'CNY' : 'USD',
    config,
    toIntlLocale(i18n.resolvedLanguage || i18n.language)
  )
  const visibleBalance = reference ? `≈ ${reference}` : balance
  const balanceLabel = reference ? t('Reference value') : t('Balance')
  const description = reference
    ? `${t('Reference value')}: ${reference} · ${t('Balance')}: ${balance}`
    : `${t('Balance')}: ${balance}`

  return (
    <>
      {compactMobile && (
        <Link
          to='/wallet'
          className='border-border/70 bg-muted/40 hover:bg-accent focus-visible:ring-ring/50 inline-flex h-auto min-h-11 max-w-32 min-w-11 shrink-0 items-center justify-center gap-1 rounded-lg border px-2 py-1 text-sm font-medium tabular-nums outline-none focus-visible:ring-[3px] md:hidden'
          aria-label={`${description} · ${walletAction}`}
          title={`${description} · ${walletAction}`}
          data-testid='mobile-account-balance'
        >
          <span className='min-w-0 text-center leading-4 wrap-anywhere whitespace-normal'>
            {reference && (
              <span className='block text-[10px] font-normal'>
                {t('Reference value')}
              </span>
            )}
            {visibleBalance}
          </span>
          {!projection.native && (
            <span aria-hidden='true' className='text-muted-foreground shrink-0'>
              +
            </span>
          )}
        </Link>
      )}
      <div
        className={cn(
          'border-border/70 bg-muted/40 inline-flex h-11 min-w-0 shrink-0 items-center rounded-lg border text-xs font-medium md:h-8',
          compactMobile && 'hidden md:inline-flex',
          className
        )}
        data-testid='account-balance-badge'
      >
        <span className='text-muted-foreground ps-2 md:ps-2.5'>
          {balanceLabel}
        </span>
        <Tooltip>
          <TooltipTrigger
            render={
              <button
                type='button'
                className='text-muted-foreground hover:text-foreground focus-visible:ring-ring/50 -mt-1 inline-flex size-5 items-center justify-center rounded-full text-[10px] font-semibold outline-none focus-visible:ring-[3px]'
                aria-label={t('Wallet balance')}
              />
            }
          >
            <sup aria-hidden='true'>?</sup>
          </TooltipTrigger>
          <TooltipContent side='bottom' className='max-w-72 leading-5'>
            {projection.native ? (
              reference ? (
                <>
                  {t('Balance')}: {balance}
                  <br />
                  {t(
                    'Reference values use the current pricing rate. Your balance is recorded in credits.'
                  )}
                </>
              ) : (
                t('Reference value unavailable')
              )
            ) : (
              t(
                'Your balance is stored in credits. Display currency does not change the amount charged at checkout.'
              )
            )}
          </TooltipContent>
        </Tooltip>
        <Link
          to='/wallet'
          className='hover:text-accent-foreground focus-visible:ring-ring/50 max-w-44 truncate px-2 tabular-nums outline-none focus-visible:ring-[3px]'
          aria-label={description}
        >
          {visibleBalance}
        </Link>
        <Link
          to='/wallet'
          className='border-border hover:bg-accent hover:text-accent-foreground focus-visible:ring-ring/50 inline-flex h-full shrink-0 items-center border-s px-2 font-semibold outline-none focus-visible:ring-[3px] md:px-2.5'
          aria-label={walletAction}
        >
          {walletAction}
        </Link>
      </div>
    </>
  )
}
