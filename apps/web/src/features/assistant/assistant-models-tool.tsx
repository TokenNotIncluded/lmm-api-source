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
import { Alert02Icon, ReloadIcon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { CopyButton } from '@/components/copy-button'
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'

import { getAssistantAvailableModels } from './api'

export function AssistantModelsTool() {
  const { t } = useTranslation()
  const modelsQuery = useQuery({
    queryKey: ['assistant-available-models'],
    queryFn: getAssistantAvailableModels,
    staleTime: 60_000,
    retry: false,
  })
  const models = modelsQuery.data ?? []

  let content = (
    <div className='grid max-h-60 gap-1 overflow-y-auto rounded-lg border p-2'>
      {models.map((model) => (
        <div
          key={model}
          className='bg-muted/30 flex items-center gap-2 rounded-md px-2 py-1.5'
        >
          <code className='min-w-0 flex-1 text-xs break-all'>{model}</code>
          <CopyButton
            value={model}
            size='sm'
            aria-label={t('Copy model name')}
          />
        </div>
      ))}
    </div>
  )

  if (modelsQuery.isLoading) {
    content = (
      <div className='grid gap-2' aria-label={t('Loading current models...')}>
        <Skeleton className='h-9 w-full' />
        <Skeleton className='h-9 w-full' />
        <Skeleton className='h-9 w-4/5' />
      </div>
    )
  } else if (modelsQuery.isError) {
    content = (
      <Alert variant='destructive'>
        <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} aria-hidden='true' />
        <AlertTitle>{t('Failed to load enabled models')}</AlertTitle>
        <AlertDescription>
          {t('Retry before using account-specific assistant tools.')}
        </AlertDescription>
        <AlertAction>
          <Button
            type='button'
            variant='outline'
            size='sm'
            onClick={() => void modelsQuery.refetch()}
          >
            <HugeiconsIcon
              icon={ReloadIcon}
              strokeWidth={2}
              data-icon='inline-start'
              aria-hidden='true'
            />
            {t('Retry')}
          </Button>
        </AlertAction>
      </Alert>
    )
  } else if (models.length === 0) {
    content = (
      <p className='text-muted-foreground rounded-lg border border-dashed p-3 text-xs'>
        {t('No available models')}
      </p>
    )
  }

  return (
    <Card size='sm'>
      <CardHeader>
        <CardTitle>{t('View all currently available models')}</CardTitle>
        <CardDescription>
          {t(
            'Ask me for the current model IDs and routing groups. I will read the account-specific list instead of guessing from a public model name.'
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className='grid gap-3'>
        {content}
        {models.length > 0 ? (
          <CopyButton
            value={models.join(',')}
            variant='outline'
            size='sm'
            tooltip={t('Copy model names')}
            aria-label={t('Copy model names')}
          >
            {t('Copy model names')}
          </CopyButton>
        ) : null}
      </CardContent>
    </Card>
  )
}
