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
import { CircleAlert, Search } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Button } from '@/components/ui/button'

export interface EmptyStateProps {
  searchQuery?: string
  hasActiveFilters: boolean
  onClearFilters: () => void
  emptyReason?: 'no_visible_models' | 'catalog_empty' | null
  onViewPublic?: () => void
  error?: unknown
  onRetry?: () => void | Promise<unknown>
}

export function EmptyState(props: EmptyStateProps) {
  const { t } = useTranslation()

  if (props.error) {
    return (
      <Alert variant='destructive' className='my-2'>
        <CircleAlert aria-hidden='true' />
        <AlertTitle>{t('Failed to load enabled models')}</AlertTitle>
        <AlertDescription>
          {t('Please retry or refresh the page.')}
        </AlertDescription>
        {props.onRetry ? (
          <AlertAction>
            <Button
              type='button'
              size='sm'
              variant='outline'
              onClick={() => void props.onRetry?.()}
            >
              {t('Retry')}
            </Button>
          </AlertAction>
        ) : null}
      </Alert>
    )
  }

  const hasSearch = Boolean(props.searchQuery?.trim())
  const noVisibleModels = props.emptyReason === 'no_visible_models'
  const emptyPublicCatalog = props.emptyReason === 'catalog_empty'
  const catalogEmpty = noVisibleModels || emptyPublicCatalog

  return (
    <div className='flex min-h-[320px] flex-col items-center justify-center px-6 py-12 text-center'>
      <Search className='text-muted-foreground/40 mb-3 size-10' />

      <h3 className='text-foreground mb-1 text-base font-semibold'>
        {noVisibleModels
          ? t('No models available to your account')
          : emptyPublicCatalog
            ? t('No public models listed')
            : t('No models found')}
      </h3>

      <p className='text-muted-foreground mb-5 max-w-sm text-sm'>
        {noVisibleModels
          ? t(
              'Your account currently has no available models. Contact an administrator to request access.'
            )
          : emptyPublicCatalog
            ? t(
                'No models are currently listed in the public catalog. Please check again later.'
              )
            : hasSearch
              ? t(
                  'No results for "{{query}}". Try adjusting your search or filters.',
                  { query: props.searchQuery }
                )
              : t('No models match your current filters.')}
      </p>

      <div className='flex flex-wrap justify-center gap-2'>
        {noVisibleModels && props.onViewPublic ? (
          <Button type='button' size='sm' onClick={props.onViewPublic}>
            {t('View public catalog')}
          </Button>
        ) : null}
        {catalogEmpty && props.onRetry ? (
          <Button
            type='button'
            variant='outline'
            size='sm'
            onClick={() => void props.onRetry?.()}
          >
            {t('Refresh')}
          </Button>
        ) : null}
        {!catalogEmpty && (props.hasActiveFilters || hasSearch) ? (
          <Button
            type='button'
            variant='outline'
            size='sm'
            onClick={props.onClearFilters}
          >
            {t('Clear all filters')}
          </Button>
        ) : null}
      </div>
    </div>
  )
}
