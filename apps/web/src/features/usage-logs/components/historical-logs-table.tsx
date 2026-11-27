/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { getRouteApi } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { CopyButton } from '@/components/copy-button'
import { DataTablePage, useDataTable } from '@/components/data-table'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useMediaQuery } from '@/hooks'
import { useHistoricalQuotaMoney } from '@/hooks/use-historical-quota-money'
import { formatTimestampToDate } from '@/lib/format'
import type {
  ColumnDef,
  ColumnFiltersState,
  OnChangeFn,
  PaginationState,
} from '@/lib/table'

import type { HistoricalLog } from '../lib/history-response'
import { CommonLogsFilterBar } from './common-logs-filter-bar'
import { ModelBadge } from './model-badge'
import { useUsageLogsContext } from './usage-logs-provider'

const route = getRouteApi('/_authenticated/usage-logs/$section')

interface HistoricalLogsTableProps {
  importedRange?: [number, number] | null
  items: HistoricalLog[]
  total: number
  pagination: PaginationState
  onPaginationChange: OnChangeFn<PaginationState>
  columnFilters: ColumnFiltersState
  onColumnFiltersChange: OnChangeFn<ColumnFiltersState>
  ensurePageInRange: (pageCount: number) => void
  isFetching: boolean
}

/** Historical rows carry their original ID and nullable facts, not native receipts. */
export function HistoricalLogsTable(props: HistoricalLogsTableProps) {
  const { t, i18n } = useTranslation()
  const isMobile = useMediaQuery('(max-width: 640px)')
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const range = props.importedRange
  const outside =
    range &&
    ((search.startTime !== undefined &&
      Math.floor(search.startTime / 1000) > range[1]) ||
      (search.endTime !== undefined &&
        Math.floor(search.endTime / 1000) < range[0]))
  const jumpToLatestDay = () => {
    if (!range) return
    const latest = new Date(range[1] * 1000)
    const start = new Date(latest).setHours(0, 0, 0, 0)
    const end = new Date(latest).setHours(23, 59, 59, 999)
    void navigate({
      to: '/usage-logs/$section',
      params: { section: 'common' },
      search: {
        source: 'historical',
        scope: search.scope,
        pageSize: search.pageSize,
        page: 1,
        startTime: start,
        endTime: end,
      },
    })
  }
  const { sensitiveVisible } = useUsageLogsContext()
  const { formatHistoricalQuota } = useHistoricalQuotaMoney()
  const columns: ColumnDef<HistoricalLog>[] = [
    {
      accessorKey: 'created_at',
      header: t('Time'),
      size: 168,
      cell: ({ row }) => formatTimestampToDate(row.original.created_at),
    },
    {
      accessorKey: 'source_log_id',
      header: t('ID'),
      size: 84,
      cell: ({ row }) => String(row.original.source_log_id),
    },
    {
      accessorKey: 'username',
      header: t('Username'),
      cell: ({ row }) =>
        sensitiveVisible ? row.original.username || '—' : '••••',
    },
    {
      accessorKey: 'model_name',
      header: t('Model'),
      size: 192,
      cell: ({ row }) =>
        row.original.model_name ? (
          <ModelBadge modelName={row.original.model_name} />
        ) : (
          '—'
        ),
    },
    {
      accessorKey: 'quota',
      header: t('Usage'),
      cell: ({ row }) =>
        row.original.quota === null
          ? '—'
          : formatHistoricalQuota(row.original.quota, row.original),
    },
    {
      accessorKey: 'prompt_tokens',
      header: t('Input Tokens'),
      cell: ({ row }) => row.original.prompt_tokens?.toLocaleString() ?? '—',
    },
    {
      accessorKey: 'completion_tokens',
      header: t('Output Tokens'),
      cell: ({ row }) =>
        row.original.completion_tokens?.toLocaleString() ?? '—',
    },
    {
      accessorKey: 'token_name',
      header: t('Token Name'),
      cell: ({ row }) =>
        sensitiveVisible ? row.original.token_name || '—' : '••••',
    },
    {
      accessorKey: 'group',
      header: t('Group'),
      size: 116,
      cell: ({ row }) =>
        row.original.group ? (
          <Badge variant='outline' className='text-primary bg-primary/5'>
            {row.original.group}
          </Badge>
        ) : (
          '—'
        ),
    },
    {
      accessorKey: 'request_id',
      header: t('Request ID'),
      size: 240,
      cell: ({ row }) => {
        const id = row.original.request_id
        return id ? (
          <div className='flex min-w-0 items-center gap-2'>
            <span className='min-w-0 truncate font-mono' title={id}>
              {id}
            </span>
            <CopyButton value={id} className='size-7 shrink-0' />
          </div>
        ) : (
          '—'
        )
      },
    },
  ]
  const { table } = useDataTable({
    data: props.items,
    columns,
    pagination: props.pagination,
    onPaginationChange: props.onPaginationChange,
    columnFilters: props.columnFilters,
    onColumnFiltersChange: props.onColumnFiltersChange,
    totalCount: props.total,
    ensurePageInRange: props.ensurePageInRange,
    enableRowSelection: false,
    manualFiltering: true,
    manualPagination: true,
    enableSorting: false,
    columnVisibilityStorageKey: 'usage-logs:historical:column-visibility',
  })
  return (
    <DataTablePage
      table={table}
      columns={columns}
      isFetching={props.isFetching}
      applyHeaderSize
      tableClassName='[&_[data-slot=table]]:table-fixed!'
      tableHeaderClassName='text-muted-foreground'
      fixedHeight={!isMobile}
      paginationInFooter={!isMobile}
      emptyTitle={
        range === null
          ? t('No imported records for this scope')
          : outside
            ? t('The selected dates are outside the imported range')
            : range
              ? t('No imported records match these filters')
              : t('No Logs Found')
      }
      emptyAction={
        range ? (
          <Button variant='outline' onClick={jumpToLatestDay}>
            {t('Reset filters and show the latest day with records')}
          </Button>
        ) : undefined
      }
      toolbar={
        <div className='space-y-3'>
          <div role='note' className='text-muted-foreground text-sm'>
            <p className='text-foreground font-medium'>
              {t('Imported history')}
            </p>
            {range ? (
              <p>
                {t('Imported dates: {{first}} – {{last}}', {
                  first: new Date(range[0] * 1000).toLocaleDateString(
                    i18n.language
                  ),
                  last: new Date(range[1] * 1000).toLocaleDateString(
                    i18n.language
                  ),
                })}
              </p>
            ) : null}
          </div>
          <CommonLogsFilterBar table={table} />
        </div>
      }
    />
  )
}
