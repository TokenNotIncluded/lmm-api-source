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
  ChevronLeft as ChevronLeftIcon,
  ChevronRight as ChevronRightIcon,
  ChevronsLeft as DoubleArrowLeftIcon,
  ChevronsRight as DoubleArrowRightIcon,
} from 'lucide-react'
import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { RowData, Table } from '@/lib/table'
import { cn, getPageNumbers } from '@/lib/utils'

import {
  PAGE_SIZE_OPTIONS,
  parsePageSize,
  resetInlinePaginationScroll,
} from './pagination-state'

type DataTablePaginationProps<TData extends RowData> = {
  table: Table<TData>
}

const PAGE_SIZE_SELECT_ITEMS = PAGE_SIZE_OPTIONS.map((pageSize) => ({
  value: `${pageSize}`,
  label: pageSize,
}))

export function DataTablePagination<TData extends RowData>({
  table,
}: DataTablePaginationProps<TData>) {
  const { t } = useTranslation()
  const pagination = table.state.pagination
  const containerRef = useRef<HTMLDivElement>(null)
  const previousPageRef = useRef({ ...pagination })

  useEffect(() => {
    const nextPage = {
      pageIndex: pagination.pageIndex,
      pageSize: pagination.pageSize,
    }
    resetInlinePaginationScroll(
      containerRef.current,
      previousPageRef.current,
      nextPage
    )
    previousPageRef.current = nextPage
  }, [pagination.pageIndex, pagination.pageSize])
  const currentPage = pagination.pageIndex + 1
  const pageSize = pagination.pageSize
  const totalPages = table.getPageCount()
  const totalRows = table.getRowCount()
  const pageNumbers = getPageNumbers(currentPage, totalPages)

  return (
    <div
      ref={containerRef}
      className='@container/pagination w-full min-w-0'
      role='navigation'
      aria-label={t('Pagination')}
    >
      <div className='flex w-full min-w-0 flex-col gap-3 @3xl/pagination:flex-row @3xl/pagination:flex-wrap @3xl/pagination:items-center @3xl/pagination:justify-between @3xl/pagination:gap-4'>
        <div className='flex min-w-0 flex-wrap items-center justify-between gap-x-4 gap-y-2 @3xl/pagination:justify-start'>
          <div className='flex min-w-0 flex-wrap items-baseline gap-x-1.5 gap-y-1 text-xs font-medium sm:text-sm'>
            <span className='text-muted-foreground/80'>{t('Total:')}</span>
            <span className='text-foreground min-w-0 [overflow-wrap:anywhere] tabular-nums'>
              {totalRows.toLocaleString()}
            </span>
          </div>

          <div className='flex min-w-0 items-center gap-2'>
            <p className='text-muted-foreground/80 hidden text-sm font-medium @4xl/pagination:block'>
              {t('Rows per page')}
            </p>
            <Select
              items={PAGE_SIZE_SELECT_ITEMS}
              value={`${pageSize}`}
              onValueChange={(value) => {
                const size = parsePageSize(value)
                if (size !== null) table.setPageSize(size)
              }}
            >
              <SelectTrigger
                aria-label={t('Rows per page')}
                className='text-foreground h-11 w-22 shrink-0 font-medium tabular-nums sm:w-20 @lg/pagination:h-8'
              >
                <SelectValue placeholder={pageSize} />
              </SelectTrigger>
              <SelectContent side='top' alignItemWithTrigger={false}>
                <SelectGroup>
                  {PAGE_SIZE_OPTIONS.map((pageSize) => (
                    <SelectItem key={pageSize} value={`${pageSize}`}>
                      {pageSize}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>
        </div>
        <div className='flex w-full min-w-0 flex-wrap items-center justify-between gap-2 @3xl/pagination:ms-auto @3xl/pagination:w-auto @3xl/pagination:justify-end @3xl/pagination:gap-1.5'>
          <Button
            variant='ghost'
            className='text-muted-foreground hover:text-foreground disabled:text-muted-foreground/50 size-11 p-0 @max-3xl/pagination:hidden @lg/pagination:size-8'
            onClick={() => table.setPageIndex(0)}
            disabled={!table.getCanPreviousPage()}
          >
            <span className='sr-only'>{t('Go to first page')}</span>
            <DoubleArrowLeftIcon className='h-4 w-4' />
          </Button>
          <Button
            variant='ghost'
            className='text-muted-foreground hover:text-foreground disabled:text-muted-foreground/50 size-11 p-0 @lg/pagination:size-8'
            onClick={() => table.previousPage()}
            disabled={!table.getCanPreviousPage()}
          >
            <span className='sr-only'>{t('Go to previous page')}</span>
            <ChevronLeftIcon className='h-4 w-4' />
          </Button>

          <span
            aria-current='page'
            aria-label={t('Go to page {{page}}', { page: currentPage })}
            className='min-w-0 flex-1 text-center text-sm font-medium tabular-nums @3xl/pagination:hidden'
          >
            {currentPage} / {Math.max(1, totalPages)}
          </span>

          {pageNumbers.map((pageNumber, index) => (
            <div
              key={getPageItemKey(pageNumbers, pageNumber, index)}
              className='hidden items-center @3xl/pagination:flex'
            >
              {pageNumber === '...' ? (
                <>
                  <span
                    aria-hidden='true'
                    className='text-muted-foreground/60 px-0.5 text-sm @lg/pagination:px-1'
                  >
                    ...
                  </span>
                  <span className='sr-only'>{t('More pages')}</span>
                </>
              ) : (
                <Button
                  variant={currentPage === pageNumber ? 'default' : 'ghost'}
                  className={cn(
                    'h-11 min-w-11 px-2 tabular-nums @lg/pagination:h-8 @lg/pagination:min-w-8',
                    currentPage === pageNumber
                      ? 'font-semibold'
                      : 'text-muted-foreground hover:text-foreground'
                  )}
                  onClick={() => table.setPageIndex((pageNumber as number) - 1)}
                  aria-current={currentPage === pageNumber ? 'page' : undefined}
                >
                  <span className='sr-only'>
                    {t('Go to page {{page}}', { page: pageNumber })}
                  </span>
                  {pageNumber}
                </Button>
              )}
            </div>
          ))}

          <Button
            variant='ghost'
            className='text-muted-foreground hover:text-foreground disabled:text-muted-foreground/50 size-11 p-0 @lg/pagination:size-8'
            onClick={() => table.nextPage()}
            disabled={!table.getCanNextPage()}
          >
            <span className='sr-only'>{t('Go to next page')}</span>
            <ChevronRightIcon className='h-4 w-4' />
          </Button>
          <Button
            variant='ghost'
            className='text-muted-foreground hover:text-foreground disabled:text-muted-foreground/50 size-11 p-0 @max-3xl/pagination:hidden @lg/pagination:size-8'
            onClick={() => table.setPageIndex(table.getPageCount() - 1)}
            disabled={!table.getCanNextPage()}
          >
            <span className='sr-only'>{t('Go to last page')}</span>
            <DoubleArrowRightIcon className='h-4 w-4' />
          </Button>
        </div>
      </div>
    </div>
  )
}

function getPageItemKey(
  pageNumbers: readonly (number | string)[],
  pageNumber: number | string,
  pageIndex: number
) {
  if (pageNumber !== '...') {
    return `page-${pageNumber}`
  }

  const previousPage = pageNumbers[pageIndex - 1] ?? 'start'
  const nextPage = pageNumbers[pageIndex + 1] ?? 'end'
  return `ellipsis-${previousPage}-${nextPage}`
}
