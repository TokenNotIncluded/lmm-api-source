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
import { getRouteApi } from '@tanstack/react-router'
import { useCallback } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { CacheStatsDialog } from '@/features/system-settings/general/channel-affinity/cache-stats-dialog'
import { useMediaQuery } from '@/hooks'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

import { UserInfoDialog } from './components/dialogs/user-info-dialog'
import {
  type LogsViewScope,
  UsageLogsProvider,
  useLogsViewScope,
  useUsageLogsContext,
} from './components/usage-logs-provider'
import { UsageLogsTable } from './components/usage-logs-table'
import { NATIVE_LOG_TYPE_FILTERS } from './constants'

const route = getRouteApi('/_authenticated/usage-logs/$section')

function UsageLogsContent() {
  const { t } = useTranslation()
  const navigate = route.useNavigate()
  const isMobile = useMediaQuery('(max-width: 640px)')
  const {
    selectedUserId,
    userInfoDialogOpen,
    setUserInfoDialogOpen,
    affinityTarget,
    affinityDialogOpen,
    setAffinityDialogOpen,
  } = useUsageLogsContext()
  const { canManageScope, viewScope, usageSource } = useLogsViewScope()
  const handleViewScopeChange = useCallback(
    (scope: string) => {
      if (scope === 'all' || scope === 'self') {
        void navigate({
          to: '/usage-logs/$section',
          params: { section: 'common' },
          search: (previous) => ({
            ...previous,
            source: usageSource,
            scope: scope as LogsViewScope,
            page: 1,
            username: undefined,
            channel: undefined,
          }),
        })
      }
    },
    [usageSource, navigate]
  )

  return (
    <>
      <SectionPageLayout fixedContent={!isMobile}>
        <SectionPageLayout.Title>{t('Usage Logs')}</SectionPageLayout.Title>
        <SectionPageLayout.Actions>
          <Tabs
            value={usageSource}
            onValueChange={(value) => {
              if (value !== 'native' && value !== 'historical') return
              void navigate({
                to: '/usage-logs/$section',
                params: { section: 'common' },
                search: (previous) => ({
                  ...previous,
                  source: value,
                  scope: viewScope,
                  page: 1,
                  channel: undefined,
                  type:
                    value === 'historical' ||
                    NATIVE_LOG_TYPE_FILTERS.some(
                      ({ value }) => value === previous.type?.[0]
                    )
                      ? previous.type
                      : undefined,
                  upstreamRequestId: undefined,
                  status: undefined,
                  minDurationMs: undefined,
                  maxDurationMs: undefined,
                  sortOrder: undefined,
                }),
              })
            }}
          >
            <TabsList>
              <TabsTrigger value='native'>{t('Native usage')}</TabsTrigger>
              <TabsTrigger value='historical'>
                {t('Historical Usage')}
              </TabsTrigger>
            </TabsList>
          </Tabs>
          {canManageScope && (
            <Tabs value={viewScope} onValueChange={handleViewScopeChange}>
              <TabsList>
                <TabsTrigger value='all'>{t('All')}</TabsTrigger>
                <TabsTrigger value='self'>{t('Only Mine')}</TabsTrigger>
              </TabsList>
            </Tabs>
          )}
        </SectionPageLayout.Actions>
        <SectionPageLayout.Content>
          <div
            className={cn('flex min-h-0 flex-col gap-4', !isMobile && 'h-full')}
          >
            <div className={cn('min-h-0', !isMobile && 'flex-1')}>
              <UsageLogsTable logCategory='common' />
            </div>
          </div>
        </SectionPageLayout.Content>
      </SectionPageLayout>

      <UserInfoDialog
        userId={selectedUserId}
        open={userInfoDialogOpen}
        onOpenChange={setUserInfoDialogOpen}
      />

      <CacheStatsDialog
        open={affinityDialogOpen}
        onOpenChange={setAffinityDialogOpen}
        target={
          affinityTarget
            ? {
                rule_name: affinityTarget.rule_name || '',
                using_group:
                  affinityTarget.using_group ||
                  affinityTarget.selected_group ||
                  '',
                key_hint: affinityTarget.key_hint || '',
                key_fp: affinityTarget.key_fp || '',
              }
            : null
        }
      />
    </>
  )
}

export function UsageLogs() {
  const search = route.useSearch()
  const selectedType = search.type?.[0]
  const historicalType =
    selectedType !== undefined &&
    !NATIVE_LOG_TYPE_FILTERS.some(({ value }) => value === selectedType)
  const auth = useAuthStore((state) => state.auth)
  const accountKey = [auth.user?.id, auth.session?.sid, auth.user?.role].join(
    ':'
  )
  return (
    <UsageLogsProvider
      key={accountKey}
      source={historicalType ? 'historical' : search.source}
      scope={search.scope}
    >
      <UsageLogsContent />
    </UsageLogsProvider>
  )
}
