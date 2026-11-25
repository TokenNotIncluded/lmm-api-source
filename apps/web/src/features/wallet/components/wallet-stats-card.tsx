/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { Activity, BarChart3, WalletCards } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Skeleton } from '@/components/ui/skeleton'
import { useWalletCurrency } from '@/hooks/use-wallet-currency'
import { toIntlLocale } from '@/i18n/languages'
import { formatCumulativeUserUsage } from '@/lib/cumulative-user-usage'
import { cn } from '@/lib/utils'

import { nativeCreditDisplay, walletProjection } from '../lib/wallet-projection'
import { walletReferenceValue } from '../lib/wallet-reference'
import type { UserWalletData } from '../types'
import { NativeWalletDetails } from './native-wallet-details'

interface WalletStatsCardProps {
  user: UserWalletData | null
  loading?: boolean
  balanceAction?: ReactNode
}

export function WalletStatsCard(props: WalletStatsCardProps) {
  const { t, i18n } = useTranslation()
  const projection = walletProjection(props.user)
  const financialAvailable = projection.available !== null
  const walletCurrency = useWalletCurrency()
  const reference = walletReferenceValue(
    projection.available,
    walletCurrency.currency === 'CNY' ? 'CNY' : 'USD',
    walletCurrency.config,
    toIntlLocale(i18n.resolvedLanguage || i18n.language)
  )
  if (props.loading) {
    return (
      <div className='wallet-stats bg-card grid overflow-hidden rounded-2xl'>
        {['balance', 'usage', 'requests'].map((key, index) => (
          <div
            key={key}
            className={cn(
              'min-w-0 p-4 sm:p-6',
              index === 0 && 'col-span-full p-5 sm:p-6'
            )}
          >
            <Skeleton className='h-3.5 w-full' />
            <Skeleton className='mt-3 h-6 w-3/4' />
            <Skeleton className='mt-2 hidden h-3 w-24 md:block' />
          </div>
        ))}
      </div>
    )
  }

  const stats: {
    label: string
    value: string
    description: string
    icon: typeof WalletCards
    available?: boolean
  }[] = [
    {
      label: t('Current Balance'),
      value: nativeCreditDisplay(
        projection.available,
        t('Credits'),
        t('Unknown')
      ),
      description: t('Remaining quota'),
      icon: WalletCards,
      available: financialAvailable,
    },
    {
      label: t('Total Usage'),
      value: projection.history
        ? formatCumulativeUserUsage(
            props.user,
            (quota) => nativeCreditDisplay(quota, t('Credits'), t('Unknown')),
            t('Credits'),
            i18n.language
          )
        : t('Unavailable'),
      description: t('Total consumed quota'),
      icon: BarChart3,
      available: projection.history,
    },
    {
      label: t('API Requests'),
      value:
        projection.requests !== null
          ? projection.requests.toLocaleString()
          : t('Unavailable'),
      description: t('Total requests made'),
      icon: Activity,
    },
  ]

  return (
    <div
      id='wallet-balance'
      tabIndex={-1}
      className='wallet-stats bg-card grid overflow-hidden rounded-2xl'
    >
      {stats.map((item, index) => (
        <div
          key={item.label}
          className={cn(
            'min-w-0 p-4 sm:p-5',
            index === 0
              ? 'relative isolate col-span-full overflow-hidden p-5 sm:p-6'
              : 'wallet-secondary-stat border-border/60 border-t'
          )}
        >
          <div
            className='relative z-10 flex min-w-0 flex-wrap items-center gap-2'
          >
            <item.icon
              className='text-muted-foreground size-4 shrink-0'
              aria-hidden='true'
            />
            <div className='text-muted-foreground text-sm leading-5 font-medium'>
              {index === 0 && reference ? t('Reference value') : item.label}
            </div>
            {index === 0 && props.balanceAction && (
              <div className='ml-auto'>{props.balanceAction}</div>
            )}
          </div>

          <div
            className={cn(
              'text-foreground pointer-events-none relative z-10 mt-4 font-medium tracking-tight break-words tabular-nums',
              index === 0
                ? 'text-3xl leading-tight sm:text-4xl [overflow-wrap:anywhere]'
                : 'text-sm sm:text-xl [overflow-wrap:anywhere]'
            )}
          >
            {index === 0 && reference ? `≈ ${reference}` : item.value}
          </div>
          {index === 0 ? (
            <p
              className='text-muted-foreground relative z-10 mt-2 text-sm tabular-nums'
              data-wallet-credit-balance
            >
              {reference ? item.value : t('Reference value unavailable')}
            </p>
          ) : (
            <div className='text-muted-foreground pointer-events-none relative z-10 mt-2 hidden text-xs leading-5 sm:block'>
              {item.description}
            </div>
          )}
          {index === 0 && <NativeWalletDetails user={props.user} />}
        </div>
      ))}
    </div>
  )
}
