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
import {
  useQueryClient,
  useIsFetching,
  useQuery,
  useInfiniteQuery,
} from '@tanstack/react-query'
import {
  defaultStringifySearch,
  useNavigate,
  getRouteApi,
} from '@tanstack/react-router'
import { Copy, Download, Eye, EyeOff } from 'lucide-react'
import {
  useState,
  useCallback,
  useMemo,
  useRef,
  useEffect,
  type ReactNode,
} from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Combobox } from '@/components/ui/combobox'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import {
  NativeUsersFailure,
  readNativeUsersScope,
} from '@/features/native-users/access'
import { readNativeUserGroups } from '@/features/native-users/api'
import { getUserGroups } from '@/lib/api'
import { copyToClipboard } from '@/lib/copy-to-clipboard'
import type { RowData, Table } from '@/lib/table'
import { useAuthStore } from '@/stores/auth-store'

import { exportHistoricalUsage, exportNativeUsage } from '../api'
import {
  LOG_TYPE_ALL_VALUE,
  LOG_TYPE_FILTERS,
  NATIVE_LOG_TYPE_FILTERS,
} from '../constants'
import { buildSearchParams, validateCommonLogFilters } from '../lib/filter'
import { readUsageStatisticsScope } from '../lib/statistics'
import { buildApiParams, getDefaultTimeRange } from '../lib/utils'
import type { CommonLogFilters } from '../types'
import { CommonLogsStats } from './common-logs-stats'
import { CompactDateTimeRangePicker } from './compact-date-time-range-picker'
import {
  LogsFilterField,
  LogsFilterInput,
  LogsFilterToolbar,
} from './logs-filter-toolbar'
import { useLogsViewScope, useUsageLogsContext } from './usage-logs-provider'

const route = getRouteApi('/_authenticated/usage-logs/$section')

type LogTypeValue = (typeof LOG_TYPE_FILTERS)[number]['value']
const logTypeValueSet = new Set<string>(
  LOG_TYPE_FILTERS.map((type) => type.value)
)

type CommonLogDraft = {
  sourceKey: string
  filters: CommonLogFilters
  logType: LogTypeValue
}

function isLogTypeValue(value: string): value is LogTypeValue {
  return logTypeValueSet.has(value)
}

function getLogTypeValue(value: unknown): LogTypeValue {
  return Array.isArray(value) &&
    value.length === 1 &&
    typeof value[0] === 'string' &&
    isLogTypeValue(value[0])
    ? value[0]
    : LOG_TYPE_ALL_VALUE
}

function buildSearchSourceKey(values: {
  startTime?: unknown
  endTime?: unknown
  channel?: unknown
  model?: unknown
  token?: unknown
  group?: unknown
  username?: unknown
  requestId?: unknown
  upstreamRequestId?: unknown
  type?: unknown
  status?: unknown
  requestResult?: unknown
  settlementStatus?: unknown
  minDurationMs?: unknown
  maxDurationMs?: unknown
  sortOrder?: unknown
}) {
  return [
    values.startTime,
    values.endTime,
    values.channel,
    values.model,
    values.token,
    values.group,
    values.username,
    values.requestId,
    values.upstreamRequestId,
    values.status,
    values.requestResult,
    values.settlementStatus,
    values.minDurationMs,
    values.maxDurationMs,
    values.sortOrder,
    Array.isArray(values.type) ? values.type.join(',') : values.type,
  ]
    .map((value) => String(value ?? ''))
    .join('\u001f')
}

interface CommonLogsFilterBarProps<TData extends RowData> {
  table: Table<TData>
  refreshControls?: ReactNode
  refreshInterval?: number | false
}

export function CommonLogsFilterBar<TData extends RowData>(
  props: CommonLogsFilterBarProps<TData>
) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const searchParams = route.useSearch()
  const { isAdminView: isAdmin, historical, usageSource } = useLogsViewScope()
  const { sensitiveVisible, setSensitiveVisible } = useUsageLogsContext()
  const fetchingLogs = useIsFetching({ queryKey: ['logs'] })
  const auth = useAuthStore((state) => state.auth)
  const scope = readUsageStatisticsScope(auth)
  const [exporting, setExporting] = useState(false)
  const [exportFailed, setExportFailed] = useState(false)
  const exportController = useRef<AbortController | null>(null)
  useEffect(
    () => () => {
      exportController.current?.abort()
    },
    [
      scope?.userId,
      scope?.sessionId,
      scope?.role,
      isAdmin,
      historical,
      searchParams,
    ]
  )
  const handleExport = async () => {
    if (!scope || exportController.current || (historical && !isAdmin)) return
    const controller = new AbortController()
    exportController.current = controller
    setExporting(true)
    setExportFailed(false)
    try {
      const params = buildApiParams({
        page: 1,
        pageSize: 1,
        searchParams,
        columnFilters: props.table.state.columnFilters,
        isAdmin,
      })
      const options = { params, scope, signal: controller.signal }
      const blob = historical
        ? await exportHistoricalUsage(options)
        : await exportNativeUsage({ ...options, isAdmin })
      if (controller.signal.aborted) return
      const url = URL.createObjectURL(blob)
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = historical
        ? 'lmm-historical-usage.csv'
        : 'lmm-native-usage.csv'
      anchor.click()
      URL.revokeObjectURL(url)
    } catch {
      if (!controller.signal.aborted) setExportFailed(true)
    } finally {
      if (exportController.current === controller) {
        exportController.current = null
        setExporting(false)
      }
    }
  }
  const adminScope = readNativeUsersScope(auth)
  const scopeKey = [scope?.userId, scope?.sessionId, scope?.role]

  const adminGroups = useInfiniteQuery({
    queryKey: ['history-admin-group-grants', ...scopeKey],
    initialPageParam: 1,
    queryFn: ({ signal, pageParam }) => {
      if (!adminScope) throw new NativeUsersFailure('session_changed')
      return readNativeUserGroups(adminScope, pageParam, signal)
    },
    getNextPageParam: (last) =>
      last.page * last.page_size < last.total ? last.page + 1 : undefined,
    enabled: !!adminScope && isAdmin,
    retry: false,
    staleTime: 60_000,
  })
  const { data: userGroups } = useQuery({
    queryKey: ['user-groups', ...scopeKey],
    queryFn: getUserGroups,
    enabled: !!scope && !isAdmin,
  })
  const groupOptions = useMemo(() => {
    const groups = isAdmin
      ? adminGroups.isError
        ? []
        : (adminGroups.data?.pages.flatMap((page) => page.items) ?? [])
      : Object.keys(userGroups?.data ?? {})
    return [...new Set(groups)]
      .filter((group) => group !== 'auto')
      .map((group) => ({ label: group, value: group }))
  }, [isAdmin, adminGroups.data, adminGroups.isError, userGroups])

  const searchState = useMemo<CommonLogDraft>(() => {
    const { start, end } = getDefaultTimeRange()
    const sourceValues = {
      startTime: searchParams.startTime,
      endTime: searchParams.endTime,
      channel: searchParams.channel,
      model: searchParams.model,
      token: searchParams.token,
      group: searchParams.group,
      username: searchParams.username,
      requestId: searchParams.requestId,
      upstreamRequestId: searchParams.upstreamRequestId,
      type: searchParams.type,
      status: searchParams.status,
      requestResult: searchParams.requestResult,
      settlementStatus: searchParams.settlementStatus,
      minDurationMs: searchParams.minDurationMs,
      maxDurationMs: searchParams.maxDurationMs,
      sortOrder: searchParams.sortOrder,
    }
    const filters: CommonLogFilters = {
      startTime: searchParams.startTime
        ? new Date(searchParams.startTime)
        : start,
      endTime: searchParams.endTime ? new Date(searchParams.endTime) : end,
      channel: searchParams.channel || undefined,
      model: searchParams.model || undefined,
      token: searchParams.token || undefined,
      group: searchParams.group || undefined,
      username: searchParams.username || undefined,
      requestId: searchParams.requestId || undefined,
      upstreamRequestId: searchParams.upstreamRequestId || undefined,
      requestResult:
        searchParams.requestResult ??
        (searchParams.status === 'success' || searchParams.status === 'error'
          ? searchParams.status
          : undefined),
      settlementStatus:
        searchParams.settlementStatus ??
        (searchParams.status === 'pending' || searchParams.status === 'refunded'
          ? searchParams.status
          : undefined),
      minDurationMs: searchParams.minDurationMs || undefined,
      maxDurationMs: searchParams.maxDurationMs || undefined,
      sortOrder: searchParams.sortOrder || 'desc',
    }
    return {
      sourceKey: buildSearchSourceKey(sourceValues),
      filters,
      logType: getLogTypeValue(searchParams.type),
    }
  }, [
    searchParams.startTime,
    searchParams.endTime,
    searchParams.channel,
    searchParams.model,
    searchParams.token,
    searchParams.group,
    searchParams.username,
    searchParams.requestId,
    searchParams.upstreamRequestId,
    searchParams.type,
    searchParams.status,
    searchParams.requestResult,
    searchParams.settlementStatus,
    searchParams.minDurationMs,
    searchParams.maxDurationMs,
    searchParams.sortOrder,
  ])
  const [draft, setDraft] = useState<CommonLogDraft>(() => searchState)
  const activeDraft =
    draft.sourceKey === searchState.sourceKey ? draft : searchState
  const filters = activeDraft.filters
  const typeFilters = historical ? LOG_TYPE_FILTERS : NATIVE_LOG_TYPE_FILTERS
  const logType = typeFilters.some(({ value }) => value === activeDraft.logType)
    ? activeDraft.logType
    : LOG_TYPE_ALL_VALUE
  const filterError = validateCommonLogFilters(filters, isAdmin, historical)

  const handleChange = useCallback(
    (field: keyof CommonLogFilters, value: Date | string | undefined) => {
      setDraft((current) => {
        const base =
          current.sourceKey === searchState.sourceKey ? current : searchState
        return {
          sourceKey: searchState.sourceKey,
          filters: { ...base.filters, [field]: value },
          logType: base.logType,
        }
      })
    },
    [searchState]
  )

  const handleApply = useCallback(() => {
    if (filterError) return
    const filterParams = buildSearchParams(
      {
        ...filters,
        channel: isAdmin ? filters.channel : undefined,
        username: isAdmin ? filters.username : undefined,
        upstreamRequestId:
          !historical || isAdmin ? filters.upstreamRequestId : undefined,
        requestResult: historical ? undefined : filters.requestResult,
        settlementStatus: historical ? undefined : filters.settlementStatus,
        minDurationMs: historical ? undefined : filters.minDurationMs,
        maxDurationMs: historical ? undefined : filters.maxDurationMs,
        sortOrder: historical ? undefined : filters.sortOrder,
      },
      'common'
    )
    navigate({
      to: '/usage-logs/$section',
      params: { section: 'common' },
      search: {
        ...filterParams,
        source: usageSource,
        scope: isAdmin ? 'all' : 'self',
        pageSize: searchParams.pageSize,
        type: [logType],
        page: 1,
      },
    })
    if (
      buildSearchSourceKey({ ...filterParams, type: [logType] }) ===
      searchState.sourceKey
    ) {
      void queryClient.invalidateQueries({ queryKey: ['logs'] })
      void queryClient.invalidateQueries({ queryKey: ['usage-logs-stats'] })
      void queryClient.invalidateQueries({ queryKey: ['usage-logs-trend'] })
    }
  }, [
    filters,
    logType,
    filterError,
    isAdmin,
    historical,
    navigate,
    queryClient,
    searchState.sourceKey,
    searchParams.pageSize,
    usageSource,
  ])

  const handleReset = useCallback(() => {
    const { start, end } = getDefaultTimeRange()
    const resetFilters: CommonLogFilters = { startTime: start, endTime: end }
    const resetSearch = {
      type: [LOG_TYPE_ALL_VALUE],
      startTime: start.getTime(),
      endTime: end.getTime(),
    }
    setDraft({
      sourceKey: buildSearchSourceKey(resetSearch),
      filters: resetFilters,
      logType: LOG_TYPE_ALL_VALUE,
    })

    navigate({
      to: '/usage-logs/$section',
      params: { section: 'common' },
      search: {
        page: 1,
        pageSize: searchParams.pageSize,
        source: usageSource,
        scope: isAdmin ? 'all' : 'self',
        ...resetSearch,
      },
    })
  }, [isAdmin, navigate, searchParams.pageSize, usageSource])

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') handleApply()
    },
    [handleApply]
  )

  const hasTypeFilter = logType !== LOG_TYPE_ALL_VALUE
  const expandedFilterCount = [
    filters.group,
    filters.token,
    hasTypeFilter,
    isAdmin ? filters.username : undefined,
    isAdmin ? filters.channel : undefined,
    !historical || isAdmin ? filters.upstreamRequestId : undefined,
    !historical ? filters.minDurationMs : undefined,
    !historical ? filters.maxDurationMs : undefined,
    !historical && filters.sortOrder === 'asc',
  ].filter(Boolean).length
  const hasExpandedFilters = expandedFilterCount > 0
  const hasAdditionalFilters =
    !!filters.model ||
    !!filters.requestId ||
    !!filters.requestResult ||
    !!filters.settlementStatus ||
    hasExpandedFilters
  const sensitiveType = sensitiveVisible ? 'text' : 'password'
  const logTypeItems = useMemo(
    () =>
      typeFilters.map((type) => ({
        value: type.value,
        label: t(type.label),
      })),
    [t, typeFilters]
  )
  const logTypeLabel =
    logTypeItems.find((type) => type.value === logType)?.label ?? t('All Types')

  const sensitiveToggle = (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant='ghost'
            size='icon'
            onClick={() => setSensitiveVisible(!sensitiveVisible)}
            aria-label={sensitiveVisible ? t('Hide') : t('Show')}
            className='text-muted-foreground hover:text-foreground size-9'
          />
        }
      >
        {sensitiveVisible ? <Eye /> : <EyeOff />}
      </TooltipTrigger>
      <TooltipContent>
        {sensitiveVisible ? t('Hide') : t('Show')}
      </TooltipContent>
    </Tooltip>
  )

  const dateRangeFilter = (
    <LogsFilterField wide label={t('Date Range')}>
      <CompactDateTimeRangePicker
        start={filters.startTime}
        end={filters.endTime}
        onChange={({ start, end }) => {
          handleChange('startTime', start)
          handleChange('endTime', end)
        }}
      />
    </LogsFilterField>
  )
  const modelFilter = (
    <LogsFilterField label={t('Model Name')}>
      <LogsFilterInput
        placeholder={t('Model Name')}
        value={filters.model || ''}
        onChange={(e) => handleChange('model', e.target.value)}
        onKeyDown={handleKeyDown}
      />
    </LogsFilterField>
  )
  const groupFilter = (
    <LogsFilterField label={t('Group')}>
      {sensitiveVisible ? (
        <Combobox
          options={groupOptions}
          allowCustomValue
          aria-label={t('Group')}
          emptyText={t('No group found.')}
          placeholder={t('Group')}
          className='h-9 min-w-0 text-sm leading-5'
          value={filters.group || ''}
          onValueChange={(value) => handleChange('group', value ?? '')}
          onKeyDown={handleKeyDown}
        />
      ) : (
        <LogsFilterInput
          aria-label={t('Group')}
          placeholder={t('Group')}
          type={sensitiveType}
          value={filters.group || ''}
          onChange={(e) => handleChange('group', e.target.value)}
          onKeyDown={handleKeyDown}
        />
      )}
      {isAdmin && sensitiveVisible && (
        <div
          className='text-muted-foreground mt-1 space-y-1 text-xs'
          data-group-source={adminGroups.data?.pages[0]?.source}
        >
          <p>{t('Explicit group memberships')}</p>
          {adminGroups.isLoading ? (
            <p role='status'>{t('Loading groups...')}</p>
          ) : null}
          {adminGroups.isError || !adminScope ? (
            <p role='status'>{t('Group directory is unavailable.')}</p>
          ) : null}
          {adminGroups.hasNextPage ? (
            <Button
              type='button'
              variant='ghost'
              size='sm'
              onClick={() => void adminGroups.fetchNextPage()}
              disabled={adminGroups.isFetchingNextPage}
            >
              {t('Load more')}
            </Button>
          ) : null}
        </div>
      )}
    </LogsFilterField>
  )
  const typeFilter = (
    <LogsFilterField label={t('Type')}>
      <Select
        items={logTypeItems}
        value={logType}
        onValueChange={(value) => {
          const nextLogType =
            value !== null && isLogTypeValue(value) ? value : LOG_TYPE_ALL_VALUE
          setDraft((current) => {
            const base =
              current.sourceKey === searchState.sourceKey
                ? current
                : searchState
            return {
              sourceKey: searchState.sourceKey,
              filters: base.filters,
              logType: nextLogType,
            }
          })
        }}
      >
        <SelectTrigger aria-label={t('Type')}>
          <SelectValue>{logTypeLabel}</SelectValue>
        </SelectTrigger>
        <SelectContent alignItemWithTrigger={false}>
          <SelectGroup>
            {typeFilters.map((type) => (
              <SelectItem key={type.value} value={type.value}>
                {t(type.label)}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
    </LogsFilterField>
  )
  const requestFilter = (
    <LogsFilterField label={t('Request ID')}>
      <LogsFilterInput
        placeholder={t('Request ID')}
        value={filters.requestId || ''}
        onChange={(event) => handleChange('requestId', event.target.value)}
        onKeyDown={handleKeyDown}
      />
    </LogsFilterField>
  )
  const stateFilter = (
    field: 'requestResult' | 'settlementStatus',
    label: string,
    values: { value: string; label: string }[]
  ) => {
    const items = [{ value: 'all', label: t('All') }, ...values]
    return (
      <LogsFilterField label={label}>
        <Select
          items={items}
          value={filters[field] || 'all'}
          onValueChange={(value) =>
            handleChange(
              field,
              value === 'all' || value === null ? undefined : value
            )
          }
        >
          <SelectTrigger aria-label={label}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {items.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </LogsFilterField>
    )
  }
  const statusFilter = (
    <>
      {stateFilter('requestResult', t('Request result'), [
        { value: 'success', label: t('Success') },
        { value: 'error', label: t('Error') },
        { value: 'disconnected', label: t('Disconnected or cancelled') },
      ])}
      {stateFilter('settlementStatus', t('Settlement status'), [
        { value: 'pending', label: t('Awaiting usage receipt') },
        { value: 'settled', label: t('Settled') },
        { value: 'released', label: t('Released') },
        { value: 'refunded', label: t('Refunded') },
      ])}
    </>
  )
  const sortItems = [
    { value: 'desc', label: t('Newest first') },
    { value: 'asc', label: t('Oldest first') },
  ]
  const advancedFilters = (
    <>
      {groupFilter}
      {typeFilter}
      <LogsFilterField label={t('Token Name')}>
        <LogsFilterInput
          placeholder={t('Token Name')}
          type={sensitiveType}
          value={filters.token || ''}
          onChange={(event) => handleChange('token', event.target.value)}
          onKeyDown={handleKeyDown}
        />
      </LogsFilterField>
      {isAdmin && (
        <>
          <LogsFilterField label={t('Username')}>
            <LogsFilterInput
              placeholder={t('Username')}
              type={sensitiveType}
              value={filters.username || ''}
              onChange={(event) => handleChange('username', event.target.value)}
              onKeyDown={handleKeyDown}
            />
          </LogsFilterField>
          <LogsFilterField label={t('Channel ID')}>
            <LogsFilterInput
              placeholder={t('Channel ID')}
              inputMode='numeric'
              value={filters.channel || ''}
              onChange={(event) => handleChange('channel', event.target.value)}
              onKeyDown={handleKeyDown}
            />
          </LogsFilterField>
        </>
      )}
      {(!historical || isAdmin) && (
        <LogsFilterField label={t('Upstream Request ID')}>
          <LogsFilterInput
            placeholder={t('Upstream Request ID')}
            value={filters.upstreamRequestId || ''}
            onChange={(event) =>
              handleChange('upstreamRequestId', event.target.value)
            }
            onKeyDown={handleKeyDown}
          />
        </LogsFilterField>
      )}
      {!historical && (
        <>
          <LogsFilterField label={t('Minimum duration (ms)')}>
            <LogsFilterInput
              aria-label={t('Minimum duration (ms)')}
              placeholder='0'
              inputMode='decimal'
              value={filters.minDurationMs || ''}
              onChange={(event) =>
                handleChange('minDurationMs', event.target.value)
              }
              onKeyDown={handleKeyDown}
            />
          </LogsFilterField>
          <LogsFilterField label={t('Maximum duration (ms)')}>
            <LogsFilterInput
              aria-label={t('Maximum duration (ms)')}
              placeholder={t('No limit')}
              inputMode='decimal'
              value={filters.maxDurationMs || ''}
              onChange={(event) =>
                handleChange('maxDurationMs', event.target.value)
              }
              onKeyDown={handleKeyDown}
            />
          </LogsFilterField>
          <LogsFilterField label={t('Sort order')}>
            <Select
              items={sortItems}
              value={filters.sortOrder || 'desc'}
              onValueChange={(value) =>
                handleChange('sortOrder', value || 'desc')
              }
            >
              <SelectTrigger aria-label={t('Sort order')}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {sortItems.map((item) => (
                  <SelectItem key={item.value} value={item.value}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </LogsFilterField>
        </>
      )}
    </>
  )

  return (
    <div className='min-w-0 space-y-5'>
      <LogsFilterToolbar
        table={props.table}
        stats={props.refreshControls}
        actionStart={
          <>
            {sensitiveToggle}
            <Button
              type='button'
              variant='ghost'
              size='sm'
              onClick={async () => {
                const search = defaultStringifySearch({
                  ...searchParams,
                  source: usageSource,
                  scope: isAdmin ? 'all' : 'self',
                  username: isAdmin ? searchParams.username : undefined,
                  channel: isAdmin ? searchParams.channel : undefined,
                })
                const copied = await copyToClipboard(
                  `${window.location.origin}/usage-logs/common${search}`
                )
                if (copied) toast.success(t('Filter link copied.'))
                else toast.error(t('Could not copy the filter link.'))
              }}
            >
              <Copy aria-hidden='true' />
              {t('Copy filter link')}
            </Button>
            {(!historical || isAdmin) && (
              <Button
                type='button'
                variant='ghost'
                size='sm'
                disabled={!scope || exporting}
                onClick={() => void handleExport()}
              >
                <Download aria-hidden='true' />
                {t(exporting ? 'Exporting usage...' : 'Export CSV')}
              </Button>
            )}
            {exporting && (
              <Button
                type='button'
                variant='ghost'
                size='sm'
                onClick={() => exportController.current?.abort()}
              >
                {t('Cancel export')}
              </Button>
            )}
            {exportFailed && (
              <span role='alert' className='text-destructive text-xs'>
                {t('Usage export failed. Narrow the time range or retry.')}
              </span>
            )}
          </>
        }
        primaryFilters={
          <>
            {dateRangeFilter}
            {requestFilter}
            {modelFilter}
            {!historical && statusFilter}
          </>
        }
        advancedFilters={advancedFilters}
        mobilePinnedFilters={dateRangeFilter}
        mobileFilters={
          <>
            {requestFilter}
            {modelFilter}
            {!historical && statusFilter}
            {advancedFilters}
          </>
        }
        mobileFilterCount={
          [
            filters.model,
            filters.requestId,
            filters.requestResult,
            filters.settlementStatus,
          ].filter(Boolean).length + expandedFilterCount
        }
        hasAdvancedActiveFilters={hasExpandedFilters}
        advancedFilterCount={expandedFilterCount}
        hasActiveFilters={
          hasAdditionalFilters || !!filters.startTime || !!filters.endTime
        }
        onSearch={handleApply}
        searchLoading={fetchingLogs > 0}
        searchDisabled={!!filterError}
        validationMessage={filterError ? t(filterError) : undefined}
        onReset={handleReset}
      />
      <CommonLogsStats refreshInterval={props.refreshInterval} />
    </div>
  )
}
