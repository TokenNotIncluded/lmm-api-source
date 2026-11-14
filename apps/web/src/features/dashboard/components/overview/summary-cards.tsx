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
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { getUserQuotaDates } from '@/features/dashboard/api'
import { useSummaryCardsConfig } from '@/features/dashboard/hooks/use-dashboard-config'
import { NativeWalletDetails } from '@/features/wallet/components/native-wallet-details'
import {
  nativeCreditDisplay,
  walletProjection,
} from '@/features/wallet/lib/wallet-projection'
import { useStatus } from '@/hooks/use-status'
import { toIntlLocale } from '@/i18n/languages'
import { formatCumulativeUserUsage } from '@/lib/cumulative-user-usage'
import { formatNumber } from '@/lib/format'
import { formatHistoricalRawQuota } from '@/lib/historical-quota-money'
import { computeTimeRange } from '@/lib/time'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

export function SummaryCards() {
  const { t, i18n } = useTranslation()
  const user = useAuthStore((state) => state.auth.user)
  const { loading } = useStatus()

  const summaryTimeRange = useMemo(() => computeTimeRange(1), [])
  const projection = walletProjection(user)
  const remainQuota = projection.available
  const requestCount = projection.requests

  const usageTrendQuery = useQuery({
    queryKey: [
      'dashboard',
      'overview',
      'summary-sparklines',
      user?.id,
      summaryTimeRange.start_timestamp,
      summaryTimeRange.end_timestamp,
    ],
    queryFn: async () =>
      getUserQuotaDates({
        start_timestamp: summaryTimeRange.start_timestamp,
        end_timestamp: summaryTimeRange.end_timestamp,
        default_time: 'hour',
      }),
    staleTime: 60 * 1000,
    enabled: Boolean(user?.id) && projection.history,
  })

  // Formatting follows the current language and currency configuration on every render.
  const summaryValues = {
    usedDisplay: projection.history
      ? formatCumulativeUserUsage(
          user,
          (quota) => nativeCreditDisplay(quota, t('Credits'), t('Unknown')),
          t('Credits'),
          i18n.language
        )
      : t('Unavailable'),
    requestCountDisplay:
      requestCount === null ? t('Unavailable') : formatNumber(requestCount),
  }

  const recentUsage = useMemo(
    () =>
      (usageTrendQuery.data?.data ?? []).reduce(
        (total, item) => total + (Number(item.quota) || 0),
        0
      ),
    [usageTrendQuery.data?.data]
  )

  const usageReady =
    projection.history &&
    usageTrendQuery.isSuccess &&
    usageTrendQuery.data.success === true
  const usageFailed = usageTrendQuery.isError
  // Unpriced history cannot predict the current native wallet's runway.
  const depleted = remainQuota !== null && remainQuota <= 0

  const todayUsageDisplay = usageReady
    ? formatHistoricalRawQuota(
        recentUsage,
        t('Raw credits'),
        toIntlLocale(i18n.resolvedLanguage || i18n.language)
      )
    : projection.history
      ? '—'
      : t('Unavailable')
  const runwayDisplay =
    remainQuota === null
      ? t('Unavailable')
      : usageReady && depleted
        ? t('Balance depleted')
        : t('Unknown')

  const items = useSummaryCardsConfig({
    ...summaryValues,
    todayUsageDisplay,
    currencyLabel: t('Credits'),
    todayCurrencyLabel: t('Raw credits'),
    usedCurrencyLabel: t('Credits'),
    currencyEnabled: true,
  })

  return (
    <section className='overview-summary' aria-label={t('Usage at a glance')}>
      <div className='overview-balance'>
        <div className='overview-balance-heading'>
          <h3 className='text-muted-foreground text-sm font-medium'>
            {t('Credit remaining')}
          </h3>
          {depleted ? (
            <span className='overview-health'>
              <span
                className='bg-destructive size-1.5 rounded-full'
                aria-hidden='true'
              />
              {t('Balance depleted')}
            </span>
          ) : null}
        </div>
        <div
          className='overview-balance-value'
          aria-live='polite'
          aria-atomic='true'
        >
          {remainQuota === null ? (
            t('Unknown')
          ) : loading ? (
            <Skeleton className='h-12 w-48 max-w-full' />
          ) : (
            <span>
              {nativeCreditDisplay(remainQuota, t('Credits'), t('Unknown'))}
            </span>
          )}
        </div>
        <NativeWalletDetails user={user} />
        <div className='overview-balance-footer'>
          <dl className='min-w-0 text-sm'>
            <dt className='text-muted-foreground text-xs'>{t('Runway')}</dt>
            <dd
              className={cn(
                'mt-1 font-medium tabular-nums',
                depleted && 'text-destructive'
              )}
            >
              {runwayDisplay}
            </dd>
          </dl>
          <Button
            className='min-h-11 gap-4 px-5'
            render={<Link to='/wallet' />}
          >
            {t('Wallet')}
            <ArrowRight data-icon='inline-end' aria-hidden='true' />
          </Button>
        </div>
      </div>
      <div className='overview-usage'>
        <h3 className='mb-5 text-sm font-semibold'>{t('Usage at a glance')}</h3>
        <dl className='overview-metrics'>
          {items.map((item) => (
            <div className='overview-metric' key={item.key}>
              <dt className='text-muted-foreground text-xs'>{item.title}</dt>
              <dd className='overview-metric-value'>
                {loading ||
                (item.key === 'todayUsage' && usageTrendQuery.isLoading) ? (
                  <Skeleton className='h-7 w-28 max-w-full' />
                ) : (
                  item.value
                )}
              </dd>
              <dd className='sr-only'>{item.description}</dd>
            </div>
          ))}
        </dl>
        {usageFailed && (
          <div
            role='status'
            className='text-muted-foreground mt-4 flex flex-wrap items-center gap-2 text-sm'
          >
            <span>{t('Failed to load data')}</span>
            <Button
              type='button'
              variant='ghost'
              className='min-h-11'
              disabled={usageTrendQuery.isFetching}
              onClick={() => void usageTrendQuery.refetch()}
            >
              {t('Retry')}
            </Button>
          </div>
        )}
      </div>
    </section>
  )
}
