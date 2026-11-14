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
import { Activity, BarChart3, ShieldCheck, WalletCards } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { IconBadge, type IconBadgeTone } from '@/components/ui/icon-badge'
import { Skeleton } from '@/components/ui/skeleton'
import { formatCumulativeUserUsage } from '@/lib/cumulative-user-usage'
import { cn } from '@/lib/utils'

import { nativeCreditDisplay, walletProjection } from '../lib/wallet-projection'
import type { UserWalletData } from '../types'
import { NativeWalletDetails } from './native-wallet-details'

function formatDiscount(percent: number) {
  return `${Math.round(percent)}%`
}

interface WalletStatsCardProps {
  user: UserWalletData | null
  loading?: boolean
  balanceAction?: ReactNode
}

export function WalletStatsCard(props: WalletStatsCardProps) {
  const { t, i18n } = useTranslation()
  const projection = walletProjection(props.user)
  const financialAvailable = projection.available !== null
  if (props.loading) {
    return (
      <div className='bg-muted/40 grid grid-cols-3 overflow-hidden rounded-2xl'>
        {['balance', 'usage', 'requests', 'trust'].map((key, index) => (
          <div
            key={key}
            className={cn(
              'min-w-0 p-4 sm:p-6',
              index === 0 && 'col-span-3 min-h-48 p-6 sm:p-8'
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
    tone: IconBadgeTone
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
      tone: 'primary',
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
      tone: 'info',
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
      tone: 'neutral',
    },
    {
      label: t('Trust level'),
      value: `L${props.user?.trust_level_info?.level ?? 0}`,
      description: `${formatDiscount(props.user?.trust_level_info?.discount_percent ?? 0)} ${t('discount')}`,
      icon: ShieldCheck,
      tone: 'neutral',
    },
  ]

  return (
    <div
      id='wallet-balance'
      tabIndex={-1}
      className='bg-muted/40 grid grid-cols-3 overflow-hidden rounded-2xl'
    >
      {stats.map((item, index) => (
        <div
          key={item.label}
          className={cn(
            'min-w-0 p-4 sm:p-6',
            index === 0
              ? 'relative isolate col-span-3 overflow-hidden p-6 sm:p-8'
              : ''
          )}
        >
          <div
            className={cn(
              'pointer-events-none relative z-10 flex gap-2',
              index === 0 ? 'items-center' : 'flex-col items-start'
            )}
          >
            <IconBadge tone={item.tone} size='stat'>
              <item.icon />
            </IconBadge>
            <div className='text-muted-foreground min-h-8 text-xs leading-4 font-medium sm:min-h-0'>
              {item.label}
            </div>
            {index === 0 && (
              <span
                className='ml-auto text-xs'
                aria-label={t('Balance display currency')}
              >
                {t('Credits')}
              </span>
            )}
          </div>

          <div
            className={cn(
              'text-foreground pointer-events-none relative z-10 mt-4 font-medium tracking-tight break-words tabular-nums',
              index === 0
                ? 'text-3xl leading-tight sm:text-4xl [overflow-wrap:anywhere]'
                : 'text-sm sm:text-xl'
            )}
          >
            {item.value}
          </div>
          {index === 0 ? (
            <div className='relative z-10 mt-3 flex items-end justify-between gap-4'>
              <div className='text-muted-foreground pointer-events-none text-xs'>
                {item.description}
              </div>
            </div>
          ) : (
            <div className='text-muted-foreground pointer-events-none relative z-10 mt-2 hidden text-xs leading-5 sm:block'>
              {item.description}
            </div>
          )}
          {index === 0 && <NativeWalletDetails user={props.user} />}
          {index === 0 && props.balanceAction && (
            <div className='mt-6 flex justify-end'>{props.balanceAction}</div>
          )}
        </div>
      ))}
    </div>
  )
}
