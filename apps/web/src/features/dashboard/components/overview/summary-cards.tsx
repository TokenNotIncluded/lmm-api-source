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
import { NativeWalletDetails } from '@/features/wallet/components/native-wallet-details'
import { walletProjection } from '@/features/wallet/lib/wallet-projection'
import { useStatus } from '@/hooks/use-status'
import { toIntlLocale } from '@/i18n/languages'
import {
  formatCumulativeUserUsage,
  normalizedUserUsage,
} from '@/lib/cumulative-user-usage'
import { formatHistoricalRawQuota } from '@/lib/historical-quota-money'
import { computeTimeRange } from '@/lib/time'
import { useAuthStore } from '@/stores/auth-store'

export function SummaryCards() {
  const { t, i18n } = useTranslation()
  const user = useAuthStore((state) => state.auth.user)
  const { loading } = useStatus()
  const locale = toIntlLocale(i18n.resolvedLanguage || i18n.language)
  const number = (value: number) => new Intl.NumberFormat(locale).format(value)
  const summaryTimeRange = useMemo(() => computeTimeRange(1), [])
  const remainQuota = walletProjection(user).available
  const historyAvailable = user?.usage_projection_available === true
  const requestCount =
    historyAvailable &&
    typeof user?.request_count === 'number' &&
    Number.isSafeInteger(user.request_count) &&
    user.request_count >= 0
      ? user.request_count
      : null
  const normalizedUsage = normalizedUserUsage(user)

  const usageTrendQuery = useQuery({
    queryKey: [
      'dashboard',
      'overview',
      'summary-sparklines',
      user?.id,
      summaryTimeRange.start_timestamp,
      summaryTimeRange.end_timestamp,
    ],
    queryFn: ({ signal }) =>
      getUserQuotaDates(
        {
          start_timestamp: summaryTimeRange.start_timestamp,
          end_timestamp: summaryTimeRange.end_timestamp,
          default_time: 'hour',
        },
        false,
        signal
      ),
    staleTime: 60_000,
    enabled: Boolean(user?.id),
    retry: false,
  })
  const recentUsage = useMemo(() => {
    const result = usageTrendQuery.data
    if (
      !result ||
      result.coverage?.complete === false ||
      result.availability === 'partial'
    ) {
      return null
    }
    let total = 0
    for (const item of result.data) {
      if (typeof item.quota !== 'number' || !Number.isSafeInteger(item.quota)) {
        return null
      }
      total += item.quota
      if (!Number.isSafeInteger(total)) return null
    }
    return total
  }, [usageTrendQuery.data])
  const usageReady = usageTrendQuery.isSuccess && recentUsage !== null
  const depleted = remainQuota !== null && remainQuota <= 0
  const items = [
    {
      key: 'todayUsage',
      title: t('Last 24h usage'),
      value: usageReady
        ? formatHistoricalRawQuota(recentUsage, t('Raw credits'), locale)
        : '—',
      description: t('Consumed in the last 24 hours'),
    },
    {
      key: 'usage',
      title: t('Recorded consumption'),
      value: historyAvailable
        ? formatCumulativeUserUsage(
            user,
            (quota) => formatHistoricalRawQuota(quota, t('Credits'), locale),
            t('Raw credits'),
            locale
          )
        : '—',
      description:
        normalizedUsage !== null
          ? t('Current usage and imported history')
          : historyAvailable
            ? t('Original imported counters')
            : t('Total consumed'),
    },
    {
      key: 'requests',
      title: t('Request Count'),
      value: requestCount === null ? '—' : number(requestCount),
      description: t('Total requests made'),
    },
  ]

  return (
    <section className='overview-summary' aria-label={t('Usage at a glance')}>
      <div className='overview-balance'>
        <div className='overview-balance-heading'>
          <h3 className='text-sm font-medium'>{t('Credit remaining')}</h3>
          {depleted && (
            <span className='overview-health'>
              <span
                className='bg-destructive size-1.5 rounded-full'
                aria-hidden='true'
              />
              {t('Balance depleted')}
            </span>
          )}
        </div>
        <div
          className='overview-balance-value'
          aria-live='polite'
          aria-atomic='true'
        >
          {remainQuota === null ? (
            t('Unknown')
          ) : loading ? (
            <Skeleton className='h-10 w-48 max-w-full' />
          ) : (
            <span>
              {number(remainQuota)}{' '}
              <span className='overview-unit'>{t('Credits')}</span>
            </span>
          )}
        </div>
        <NativeWalletDetails user={user} />
        <div className='overview-balance-footer'>
          <p className='text-muted-foreground max-w-[34ch] text-xs leading-relaxed'>
            {remainQuota === null
              ? t('The current wallet balance could not be read.')
              : t(
                  'Current personal wallet. Usage totals are shown separately.'
                )}
          </p>
          <Button className='min-h-11 gap-3' render={<Link to='/wallet' />}>
            {t('Wallet')}
            <ArrowRight data-icon='inline-end' aria-hidden='true' />
          </Button>
        </div>
      </div>
      <div className='overview-usage'>
        <header className='mb-4 flex flex-wrap items-center justify-between gap-2'>
          <h3 className='text-sm font-semibold'>{t('Usage at a glance')}</h3>
          <p className='text-muted-foreground text-xs'>
            {t('Your account only')}
          </p>
        </header>
        <dl className='overview-metrics'>
          {items.map((item) => (
            <div
              className='overview-metric'
              data-metric={item.key}
              key={item.key}
            >
              <dt className='text-muted-foreground text-xs'>{item.title}</dt>
              <dd className='overview-metric-value'>
                {loading ||
                (item.key === 'todayUsage' && usageTrendQuery.isLoading) ? (
                  <Skeleton className='h-7 w-28 max-w-full' />
                ) : (
                  item.value
                )}
              </dd>
              <dd className='overview-metric-description text-muted-foreground text-xs'>
                {item.description}
              </dd>
            </div>
          ))}
        </dl>
        {!historyAvailable && (
          <p className='text-muted-foreground mt-4 text-xs'>
            {t('Cumulative usage has not been reported for this account.')}
          </p>
        )}
        {usageTrendQuery.isSuccess && recentUsage === null && (
          <p className='text-muted-foreground mt-4 text-xs' role='status'>
            {t('Usage totals are incomplete for this period.')}
          </p>
        )}
        {usageTrendQuery.isError && (
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
