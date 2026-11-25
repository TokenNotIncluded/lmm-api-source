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
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { PageTransition } from '@/components/page-transition'
import { Skeleton } from '@/components/ui/skeleton'
import { EcosystemRouteShell } from '@/features/forge/ecosystem-route-shell'

import {
  MarketShareSection,
  ModelsSection,
  PulseSection,
  RankingsHero,
  UserUsageLeaderboard,
} from './components'
import { RankingLoadError } from './components/ranking-load-error'
import { useRankings } from './hooks/use-rankings'
import { useUserUsageRankings } from './hooks/use-user-usage-rankings'
import type { RankingPeriod } from './types'

const VALID_PERIODS = new Set<RankingPeriod>(['today', 'week', 'month', 'year'])

export function Rankings() {
  const { t } = useTranslation()
  const search = useSearch({ from: '/rankings/' })
  const navigate = useNavigate()

  const period: RankingPeriod = VALID_PERIODS.has(
    search.period as RankingPeriod
  )
    ? (search.period as RankingPeriod)
    : 'week'

  const handlePeriodChange = (next: RankingPeriod) => {
    navigate({
      to: '/rankings',
      search: (prev) => ({ ...prev, period: next }),
    })
  }

  return (
    <EcosystemRouteShell
      console={
        <SectionPageLayout>
          <SectionPageLayout.Title>{t('Rankings')}</SectionPageLayout.Title>
          <SectionPageLayout.Content>
            <div className='mx-auto w-full max-w-6xl'>
              <RankingsContent
                compact
                period={period}
                onPeriodChange={handlePeriodChange}
              />
            </div>
          </SectionPageLayout.Content>
        </SectionPageLayout>
      }
      public={
        <main className='mx-auto w-full max-w-7xl px-5 pt-12 pb-20 md:px-10 md:pt-16'>
          <RankingsContent
            compact={false}
            period={period}
            onPeriodChange={handlePeriodChange}
          />
        </main>
      }
    />
  )
}

export function RankingsContent({
  compact,
  period,
  onPeriodChange,
}: {
  compact: boolean
  period: RankingPeriod
  onPeriodChange: (period: RankingPeriod) => void
}) {
  const { t } = useTranslation()
  const [userLeaderboardOpen, setUserLeaderboardOpen] = useState(false)
  const rankingsQuery = useRankings(period)
  const userUsageRankingsQuery = useUserUsageRankings(
    period,
    userLeaderboardOpen
  )
  const snapshot = rankingsQuery.data?.success
    ? rankingsQuery.data.data
    : undefined

  return (
    <PageTransition className={compact ? 'space-y-6' : 'space-y-10'}>
      <RankingsHero
        period={period}
        onPeriodChange={onPeriodChange}
        compact={compact}
      />

      <UserUsageLeaderboard
        data={
          userUsageRankingsQuery.data?.success
            ? userUsageRankingsQuery.data.data
            : undefined
        }
        isLoading={userUsageRankingsQuery.isLoading}
        error={
          userUsageRankingsQuery.error ??
          (userUsageRankingsQuery.data?.success === false
            ? userUsageRankingsQuery.data
            : undefined)
        }
        onRetry={() => void userUsageRankingsQuery.refetch()}
        retrying={userUsageRankingsQuery.isFetching}
        open={userLeaderboardOpen}
        onOpenChange={setUserLeaderboardOpen}
      />

      {rankingsQuery.isLoading ? (
        <RankingsLoading />
      ) : rankingsQuery.isError || !snapshot ? (
        <RankingLoadError
          title={t('Unable to load rankings')}
          error={rankingsQuery.error ?? rankingsQuery.data}
          retrying={rankingsQuery.isFetching}
          onRetry={() => void rankingsQuery.refetch()}
        />
      ) : (
        <>
          <ModelsSection
            history={snapshot.models_history}
            rows={snapshot.models}
            period={period}
          />

          <MarketShareSection
            history={snapshot.vendor_share_history}
            rows={snapshot.vendors}
            period={period}
          />

          <PulseSection
            movers={snapshot.top_movers}
            droppers={snapshot.top_droppers}
          />
        </>
      )}
    </PageTransition>
  )
}

function RankingsLoading() {
  return (
    <div className='space-y-6'>
      <Skeleton className='h-[420px] w-full rounded-xl' />
      <Skeleton className='h-[360px] w-full rounded-xl' />
      <Skeleton className='h-[180px] w-full rounded-xl' />
    </div>
  )
}
