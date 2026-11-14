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
import { X } from 'lucide-react'
import type { ComponentType, ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import type { RowData, Table } from '@/lib/table'

import { getFilterValues, removeFilterValue } from './filter-values'

export type FilterDef = {
  columnId: string
  title: string
  options: {
    label: string
    value: string
    icon?: ComponentType<{ className?: string }>
    iconNode?: ReactNode
    count?: number
  }[]
  singleSelect?: boolean
}

export function DataTableFilterSummary<TData extends RowData>({
  table,
  filters,
  onRemove,
}: {
  table: Table<TData>
  filters: FilterDef[]
  onRemove?: () => void
}) {
  const { t } = useTranslation()
  const active = filters.flatMap((filter) => {
    const column = table.getColumn(filter.columnId)
    if (!column) return []
    return [...new Set(getFilterValues(column.getFilterValue()))].map(
      (value) => ({
        filter,
        column,
        value,
        label:
          filter.options.find((option) => option.value === String(value))
            ?.label ?? String(value),
      })
    )
  })
  if (!active.length) return null

  return (
    <div
      role='group'
      aria-label={t('Filters active')}
      className='console-active-filters flex min-w-0 flex-wrap items-center gap-2'
    >
      {active.map(({ filter, column, value, label }) => (
        <Button
          key={`${column.id}:${typeof value}:${String(value)}`}
          type='button'
          variant='secondary'
          size='sm'
          className='h-auto min-h-9 max-w-full items-start gap-2 rounded-lg px-3 py-1.5 text-start whitespace-normal'
          aria-label={`${t('Clear filters')}: ${t(filter.title)}: ${t(label)}`}
          title={`${t(filter.title)}: ${t(label)}`}
          onClick={() => {
            column.setFilterValue((current: unknown) =>
              removeFilterValue(current, value)
            )
            onRemove?.()
          }}
        >
          <span className='min-w-0 [overflow-wrap:anywhere]'>
            <span className='text-muted-foreground'>{t(filter.title)}: </span>
            {t(label)}
          </span>
          <X className='mt-0.5 size-3.5 shrink-0' aria-hidden='true' />
        </Button>
      ))}
    </div>
  )
}
