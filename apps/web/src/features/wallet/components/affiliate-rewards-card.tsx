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
import { Share2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { IconBadge } from '@/components/ui/icon-badge'
import { Skeleton } from '@/components/ui/skeleton'

import { exactCredit, nativeCreditDisplay } from '../lib/wallet-projection'
import type { UserWalletData } from '../types'

interface AffiliateRewardsCardProps {
  user: UserWalletData | null
  complianceConfirmed?: boolean
  loading?: boolean
}

export function AffiliateRewardsCard({
  user,
  complianceConfirmed = true,
  loading,
}: AffiliateRewardsCardProps) {
  const { t } = useTranslation()
  if (loading) {
    return (
      <Card
        data-card-hover='false'
        className='bg-muted/30 border-0 py-0 shadow-none'
      >
        <CardContent className='wallet-affiliate-content grid gap-8 p-6 sm:p-8'>
          <div>
            <Skeleton className='h-5 w-32' />
            <Skeleton className='mt-2 h-4 w-48' />
          </div>
          <Skeleton className='h-14 rounded-lg' />
          <Skeleton className='col-span-full h-10 rounded-lg' />
        </CardContent>
      </Card>
    )
  }

  // Only the current native affiliate bucket is authoritative.
  const nativeRewards =
    user?.wallet_unit === 'core_credit' &&
    user.financial_projection_available === true
      ? exactCredit(user.aff_quota)
      : null
  const nativeDebt = nativeRewards !== null ? exactCredit(user?.aff_debt) : null
  const rewardDebt = (nativeDebt ?? 0) > 0
  const nativeRewardDisplay = nativeCreditDisplay(
    rewardDebt ? nativeDebt : nativeRewards,
    t('Credits'),
    t('Unavailable')
  )

  return (
    <Card
      data-card-hover='false'
      className='bg-muted/30 border-0 py-0 shadow-none'
    >
      <CardContent className='wallet-affiliate-content grid gap-8 p-6 sm:p-8'>
        <div className='flex min-w-0 items-start gap-4'>
          <IconBadge tone='chart-3'>
            <Share2 />
          </IconBadge>
          <div className='min-w-0'>
            <h3 className='text-base font-semibold tracking-tight'>
              {t('Referral Program')}
            </h3>
            <p className='text-muted-foreground mt-2 max-w-prose text-sm leading-6'>
              {t(
                'Registration advance and first-payment tail are separate rewards. Available rewards can be transferred to balance. Lifetime earnings are not the current balance.'
              )}
            </p>
          </div>
        </div>

        <div className='wallet-affiliate-stats grid gap-5'>
          {[
            [
              rewardDebt ? t('Reward debt') : t('Available Rewards'),
              nativeRewardDisplay,
            ],
            [t('Total Earned'), t('Unavailable')],
            [t('Invites'), t('Unavailable')],
          ].map(([label, value]) => (
            <div key={label}>
              <div className='text-muted-foreground text-xs font-medium'>
                {label}
              </div>
              <div className='mt-3 text-base font-medium [overflow-wrap:anywhere] tabular-nums'>
                {value}
              </div>
            </div>
          ))}
        </div>

        <div className='col-span-full flex flex-wrap items-center justify-end gap-3'>
          {complianceConfirmed ? (
            <Button
              render={<a href='/core/v1/affiliate/claim' />}
              className='min-h-11 shrink-0 px-4'
              size='sm'
              variant='outline'
            >
              {t('Preview reward transfer')}
            </Button>
          ) : (
            <Button
              disabled
              className='min-h-11 shrink-0 px-4'
              size='sm'
              variant='outline'
            >
              {t('Transfer to Balance')}
            </Button>
          )}
        </div>
        {!complianceConfirmed ? (
          <p className='text-muted-foreground col-span-full text-xs leading-6'>
            {t(
              'Referral reward transfer is disabled until the administrator confirms compliance terms.'
            )}
          </p>
        ) : null}
      </CardContent>
    </Card>
  )
}
