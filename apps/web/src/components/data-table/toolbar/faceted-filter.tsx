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
import { Check as CheckIcon, PlusCircle as PlusCircledIcon } from 'lucide-react'
import * as React from 'react'
import { useTranslation } from 'react-i18next'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from '@/components/ui/command'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import type { RowData, Column } from '@/lib/table'
import { cn } from '@/lib/utils'

import { getFilterValues } from './filter-values'

type DataTableFacetedFilterProps<TData extends RowData, TValue> = {
  column?: Column<TData, TValue>
  title?: string
  options: {
    label: string
    value: string
    icon?: React.ComponentType<{ className?: string }>
    iconNode?: React.ReactNode
    count?: number
  }[]
  /** Enable single select mode (only one option can be selected at a time) */
  singleSelect?: boolean
}

function DataTableFacetedFilterInner<TData extends RowData, TValue>({
  column,
  title,
  options,
  singleSelect = false,
}: DataTableFacetedFilterProps<TData, TValue>) {
  const { t } = useTranslation()
  const facets = column?.getFacetedUniqueValues()
  const selectedValues = new Set(
    getFilterValues(column?.getFilterValue()).map(String)
  )

  const handleOptionSelect = (optionValue: string) => {
    const nextSelectedValues = getNextSelectedValues(
      selectedValues,
      optionValue,
      singleSelect
    )

    column?.setFilterValue(
      nextSelectedValues.length ? nextSelectedValues : undefined
    )
  }

  return (
    <Popover>
      <PopoverTrigger
        render={
          <Button
            variant='outline'
            size='sm'
            className='h-auto min-h-9 max-w-full min-w-0 flex-wrap justify-start gap-2 py-1.5 text-start whitespace-normal'
            aria-label={title ?? t('Filter')}
          />
        }
      >
        <PlusCircledIcon className='size-4' />
        <span className='min-w-0 [overflow-wrap:anywhere]'>{title}</span>
        {selectedValues?.size > 0 && (
          <>
            <Badge
              variant='secondary'
              className='rounded-md px-1.5 font-normal lg:hidden'
            >
              {selectedValues.size}
            </Badge>
            <div className='hidden max-w-full min-w-0 flex-wrap gap-1 lg:flex'>
              {selectedValues.size > 2 ? (
                <Badge
                  variant='secondary'
                  className='h-auto max-w-full rounded-md px-1.5 font-normal [overflow-wrap:anywhere] whitespace-normal'
                >
                  {selectedValues.size} {t('selected')}
                </Badge>
              ) : (
                options
                  .filter((option) => selectedValues.has(option.value))
                  .map((option) => (
                    <Badge
                      variant='secondary'
                      key={option.value}
                      className='h-auto max-w-full rounded-md px-1.5 font-normal [overflow-wrap:anywhere] whitespace-normal'
                    >
                      {t(option.label)}
                    </Badge>
                  ))
              )}
            </div>
          </>
        )}
      </PopoverTrigger>
      <PopoverContent className='w-80 p-0' align='start'>
        <Command>
          <CommandInput aria-label={title ?? t('Filter')} placeholder={title} />
          <CommandList>
            <CommandEmpty>{t('No results found.')}</CommandEmpty>
            <CommandGroup>
              {options.map((option) => {
                const isSelected = selectedValues.has(option.value)
                const icon = renderOptionIcon(option)
                const facetCount = facets?.get(option.value)
                const hasExplicitCount = typeof option.count === 'number'
                const optionCount = hasExplicitCount ? option.count : facetCount
                const shouldShowCount = hasExplicitCount || Boolean(facetCount)
                const countClassName = hasExplicitCount
                  ? 'text-muted-foreground ms-auto flex min-w-4 shrink-0 items-center justify-center font-mono text-xs'
                  : 'ms-auto flex min-w-4 shrink-0 items-center justify-center font-mono text-xs'

                return (
                  <CommandItem
                    key={option.value}
                    onSelect={() => handleOptionSelect(option.value)}
                  >
                    <div
                      className={cn(
                        'border-primary flex size-4 shrink-0 items-center justify-center rounded-sm border',
                        isSelected
                          ? 'bg-primary text-primary-foreground'
                          : 'opacity-50 [&_svg]:invisible'
                      )}
                    >
                      <CheckIcon className={cn('text-background h-4 w-4')} />
                    </div>
                    {icon}
                    <span
                      className='min-w-0 flex-1 [overflow-wrap:anywhere] whitespace-normal'
                      title={t(option.label)}
                    >
                      {t(option.label)}
                    </span>
                    {shouldShowCount && (
                      <span className={countClassName}>{optionCount}</span>
                    )}
                  </CommandItem>
                )
              })}
            </CommandGroup>
            {selectedValues.size > 0 && (
              <>
                <CommandSeparator />
                <CommandGroup>
                  <CommandItem
                    onSelect={() => column?.setFilterValue(undefined)}
                    className='justify-center text-center'
                  >
                    {t('Clear filters')}
                  </CommandItem>
                </CommandGroup>
              </>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

// Column identity stays stable when Reset or URL navigation changes its value.
export const DataTableFacetedFilter = DataTableFacetedFilterInner

function renderOptionIcon(option: {
  icon?: React.ComponentType<{ className?: string }>
  iconNode?: React.ReactNode
}) {
  if (option.iconNode) {
    return (
      <span className='text-muted-foreground flex size-4 shrink-0 items-center justify-center'>
        {option.iconNode}
      </span>
    )
  }

  if (option.icon) {
    const Icon = option.icon
    return <Icon className='text-muted-foreground size-4 shrink-0' />
  }

  return null
}

function getNextSelectedValues(
  selectedValues: Set<string>,
  optionValue: string,
  singleSelect: boolean
): string[] {
  if (singleSelect) {
    return selectedValues.has(optionValue) ? [] : [optionValue]
  }

  const nextSelectedValues = new Set(selectedValues)
  if (nextSelectedValues.has(optionValue)) {
    nextSelectedValues.delete(optionValue)
  } else {
    nextSelectedValues.add(optionValue)
  }

  return [...nextSelectedValues]
}
