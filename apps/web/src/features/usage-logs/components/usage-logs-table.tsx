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
import { useIsFetching, useQuery, useQueryClient } from '@tanstack/react-query'
import { getRouteApi, Link } from '@tanstack/react-router'
import { useState } from 'react'

import {
  DataTablePage,
  DataTableRow,
  useDataTable,
} from '@/components/data-table'
import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import { useMediaQuery } from '@/hooks'
import { useTableUrlState } from '@/hooks/use-table-url-state'
import type { ColumnDef } from '@/lib/table'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

import { LOG_TYPE_ALL_VALUE, LOG_TYPE_ENUM } from '../constants'
import { getAsyncLogRefreshInterval } from '../lib/async-task-logs'
import { useColumnsByCategory } from '../lib/columns'
import { parseLogOther } from '../lib/format'
import { UsageHistoryUnavailableError } from '../lib/history-response'
import { readUsageStatisticsScope } from '../lib/statistics'
import { fetchLogsByCategory } from '../lib/utils'
import { useTaskLogsTranslation } from '../task-logs-i18n'
import type { LogCategory } from '../types'
import { CommonLogsFilterBar } from './common-logs-filter-bar'
import { CommonLogsStatus } from './common-logs-status'
import { HistoricalLogsTable } from './historical-logs-table'
import { TaskLogsFilterBar } from './task-logs-filter-bar'
import { TaskLogsStatus } from './task-logs-status'
import { UsageLogsMobileList } from './usage-logs-mobile-card'
import { useLogsViewScope } from './usage-logs-provider'

const route = getRouteApi('/_authenticated/usage-logs/$section')

const logTypeRowTint: Record<number, string> = {
  [LOG_TYPE_ENUM.ERROR]: 'console-log-error-row',
  [LOG_TYPE_ENUM.REFUND]: 'console-log-refund-row',
}

// Warning tint for logs where a quota conversion saturated (admin-only marker).
// Takes precedence over the per-type tint since it flags a billing anomaly.
const quotaSaturationRowTint = 'console-log-saturation-row'

function getColumnVisibilityStorageKey(
  logCategory: LogCategory,
  isAdmin: boolean
): string {
  return `usage-logs:${logCategory}:${isAdmin ? 'admin' : 'user'}:column-visibility`
}

function deserializeLogTypeFilter(value: unknown): unknown[] {
  const values = Array.isArray(value) ? value : value ? [value] : []
  return values.filter((item) => String(item) !== LOG_TYPE_ALL_VALUE)
}

interface UsageLogsTableProps {
  logCategory: LogCategory
}

export function UsageLogsTable({ logCategory }: UsageLogsTableProps) {
  const { t } = useTaskLogsTranslation()
  const { isAdminView: isAdmin, historical } = useLogsViewScope()
  const auth = useAuthStore((state) => state.auth)
  const scope = readUsageStatisticsScope(auth)
  const isMobile = useMediaQuery('(max-width: 640px)')
  const searchParams = route.useSearch()
  const [autoRefresh, setAutoRefresh] = useState(true)
  const [commonAutoRefresh, setCommonAutoRefresh] = useState(false)
  const [readCancelled, setReadCancelled] = useState(false)
  const queryClient = useQueryClient()
  const statsFetching = useIsFetching({
    queryKey: [
      'usage-logs-stats',
      scope?.userId,
      scope?.sessionId,
      scope?.role,
    ],
  })
  const trendFetching = useIsFetching({
    queryKey: [
      'usage-logs-trend',
      scope?.userId,
      scope?.sessionId,
      scope?.role,
    ],
  })

  const {
    columnFilters,
    onColumnFiltersChange,
    pagination,
    onPaginationChange,
    ensurePageInRange,
  } = useTableUrlState({
    search: route.useSearch(),
    navigate: route.useNavigate(),
    pagination: { defaultPage: 1, defaultPageSize: isMobile ? 20 : 100 },
    globalFilter: { enabled: false },
    columnFilters: [
      {
        columnId: 'created_at',
        searchKey: 'type',
        type: 'array' as const,
        deserialize: deserializeLogTypeFilter,
      },
      { columnId: 'model_name', searchKey: 'model', type: 'string' as const },
      { columnId: 'token_name', searchKey: 'token', type: 'string' as const },
      { columnId: 'group', searchKey: 'group', type: 'string' as const },
      ...(isAdmin
        ? [
            {
              columnId: 'channel',
              searchKey: 'channel',
              type: 'string' as const,
            },
            {
              columnId: 'username',
              searchKey: 'username',
              type: 'string' as const,
            },
          ]
        : []),
    ],
  })

  const commonRefreshInterval =
    commonAutoRefresh && !historical && pagination.pageIndex === 0
      ? 5000
      : false

  const {
    data,
    dataUpdatedAt,
    isLoading,
    isFetching,
    isError,
    error,
    refetch,
  } = useQuery({
    queryKey: [
      'logs',
      logCategory,
      isAdmin,
      scope?.userId,
      scope?.sessionId,
      scope?.role,
      pagination.pageIndex + 1,
      pagination.pageSize,
      columnFilters,
      searchParams,
      t,
      historical,
    ],
    queryFn: async ({ signal }) => {
      if (!scope) throw new Error('Usage history session unavailable')
      const result = await fetchLogsByCategory({
        logCategory,
        isAdmin,
        historical,
        page: pagination.pageIndex + 1,
        pageSize: pagination.pageSize,
        searchParams,
        columnFilters,
        signal,
        scope,
      })

      if (!result.success || !result.data) {
        throw new Error(result?.message || t('Failed to load logs'))
      }

      return result.data
    },
    enabled: !!scope,
    retry: false,
    refetchInterval: (query) =>
      logCategory === 'common'
        ? query.state.status === 'error'
          ? false
          : commonRefreshInterval
        : getAsyncLogRefreshInterval(
            logCategory,
            query.state.data?.items || [],
            autoRefresh,
            query.state.status === 'error'
          ),
    refetchIntervalInBackground: false,
  })

  const logs = data?.kind === 'historical' ? [] : data?.items || []
  const columns = useColumnsByCategory(logCategory, isAdmin)
  const isLoadingData = isLoading || (isFetching && !data)

  const { table } = useDataTable({
    data: logs as Record<string, unknown>[],
    columns: columns as ColumnDef<Record<string, unknown>>[],
    columnFilters,
    columnVisibilityStorageKey: getColumnVisibilityStorageKey(
      logCategory,
      isAdmin
    ),
    pagination,
    enableRowSelection: false,
    onPaginationChange,
    onColumnFiltersChange,
    manualPagination: true,
    manualFiltering: true,
    totalCount: data?.total || 0,
    ensurePageInRange,
  })

  const isCommon = logCategory === 'common'
  const emptyDescription = isCommon
    ? t(
        'No usage logs available. Logs will appear here once API calls are made.'
      )
    : t(
        logCategory === 'drawing'
          ? 'Drawing history covers Midjourney / MjProxy image and video tasks. Other image calls appear in Common Logs.'
          : 'Task history covers asynchronous audio and video jobs. Other API calls appear in Common Logs.'
      )
  const emptyAction = isCommon ? (
    <Button variant='outline' render={<Link to='/playground' />}>
      {t('Open the playground')}
    </Button>
  ) : (
    <Button
      variant='outline'
      render={<Link to='/usage-logs/$section' params={{ section: 'common' }} />}
    >
      {t('Common Logs')}
    </Button>
  )

  const commonToolbar = (
    <CommonLogsFilterBar
      table={table}
      refreshInterval={isError ? false : commonRefreshInterval}
      refreshControls={
        <CommonLogsStatus
          total={isError ? undefined : data?.total}
          updatedAt={dataUpdatedAt}
          refreshing={isFetching || statsFetching > 0 || trendFetching > 0}
          automatic={!!commonRefreshInterval}
          canAutoRefresh={!historical && pagination.pageIndex === 0}
          onAutomaticChange={setCommonAutoRefresh}
          onRefresh={() => {
            setReadCancelled(false)
            void refetch()
            void queryClient.invalidateQueries({
              queryKey: [
                'usage-logs-stats',
                scope?.userId,
                scope?.sessionId,
                scope?.role,
              ],
            })
            void queryClient.invalidateQueries({
              queryKey: [
                'usage-logs-trend',
                scope?.userId,
                scope?.sessionId,
                scope?.role,
              ],
            })
          }}
          onCancel={() => {
            setCommonAutoRefresh(false)
            setReadCancelled(true)
            void queryClient.cancelQueries({
              queryKey: [
                'logs',
                logCategory,
                isAdmin,
                scope?.userId,
                scope?.sessionId,
                scope?.role,
              ],
            })
            void queryClient.cancelQueries({
              queryKey: [
                'usage-logs-stats',
                scope?.userId,
                scope?.sessionId,
                scope?.role,
              ],
            })
            void queryClient.cancelQueries({
              queryKey: [
                'usage-logs-trend',
                scope?.userId,
                scope?.sessionId,
                scope?.role,
              ],
            })
          }}
        />
      }
    />
  )

  if (isCommon && readCancelled && !data && !isFetching && !isError) {
    return (
      <div className='space-y-5'>
        {commonToolbar}
        <ErrorState
          title={t('Log loading cancelled')}
          description={t('Refresh to load the selected requests.')}
          onRetry={() => {
            setReadCancelled(false)
            void refetch()
          }}
        />
      </div>
    )
  }

  if (isError) {
    if (error instanceof UsageHistoryUnavailableError) {
      return (
        <ErrorState
          title={t('History unavailable')}
          description={t(
            error.category === 'drawing'
              ? 'Drawing history is not available on this server. Image API calls appear in Common Logs.'
              : 'Task history is not available on this server. Other API calls appear in Common Logs.'
          )}
          onRetry={() => void refetch()}
          action={emptyAction}
        />
      )
    }
    return (
      <div className='space-y-5'>
        {isCommon ? (
          commonToolbar
        ) : (
          <TaskLogsFilterBar table={table} logCategory={logCategory} />
        )}
        <ErrorState
          title={t('Failed to load logs')}
          description={t(
            'Check your connection and retry without losing filters.'
          )}
          onRetry={() => void refetch()}
        />
      </div>
    )
  }

  if (data?.kind === 'historical') {
    return (
      <HistoricalLogsTable
        items={data.items}
        total={data.total}
        pagination={pagination}
        onPaginationChange={onPaginationChange}
        columnFilters={columnFilters}
        onColumnFiltersChange={onColumnFiltersChange}
        ensurePageInRange={ensurePageInRange}
        isFetching={isFetching}
      />
    )
  }

  return (
    <DataTablePage
      table={table}
      columns={columns as ColumnDef<Record<string, unknown>>[]}
      isLoading={isLoadingData}
      isFetching={isFetching}
      fixedHeight={!isMobile}
      paginationInFooter={!isMobile}
      emptyTitle={t('No Logs Found')}
      emptyDescription={emptyDescription}
      emptyAction={emptyAction}
      skeletonKeyPrefix='usage-log-skeleton'
      applyHeaderSize
      tableClassName={cn(
        '[&_[data-slot=table]]:text-[13px] [&_[data-slot=table]_td]:text-[13px] [&_[data-slot=table]_td_*]:text-[13px] [&_[data-slot=table]_th]:text-[13px] [&_[data-slot=table]_th_*]:text-[13px]'
      )}
      mobile={
        <UsageLogsMobileList
          table={table}
          isLoading={isLoadingData}
          logCategory={logCategory}
          emptyDescription={emptyDescription}
          emptyAction={emptyAction}
        />
      }
      toolbar={
        isCommon ? (
          commonToolbar
        ) : (
          <TaskLogsFilterBar
            table={table}
            logCategory={logCategory}
            stats={
              <TaskLogsStatus
                rows={logs}
                page={pagination.pageIndex + 1}
                total={data?.total || 0}
                autoRefresh={autoRefresh}
                onAutoRefreshChange={setAutoRefresh}
                onRefresh={() => void refetch()}
                isFetching={isFetching}
                updatedAt={dataUpdatedAt}
              />
            }
          />
        )
      }
      renderRow={(row) => {
        const logType = (row.original as Record<string, unknown>).type as
          | number
          | undefined
        let tintClass =
          isCommon && logType != null ? (logTypeRowTint[logType] ?? '') : ''
        if (isCommon && isAdmin) {
          const other = parseLogOther(
            ((row.original as Record<string, unknown>).other as string) ?? ''
          )
          if (other?.admin_info?.quota_saturation) {
            tintClass = quotaSaturationRowTint
          }
        }

        return (
          <DataTableRow
            key={row.id}
            row={row}
            className={cn('transition-colors', tintClass)}
            getColumnClassName={() => (isCommon ? 'py-3' : 'py-3.5')}
          />
        )
      }}
    />
  )
}
