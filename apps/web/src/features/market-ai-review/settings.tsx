/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { readNativeUsersScope } from '@/features/native-users/access'
import { getSettingsErrorMessage } from '@/features/system-settings/utils/settings-error-message'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { marketAIReviewAPI } from './api'
import { MARKET_AI_REVIEW_COPY as copy } from './copy'
import {
  MarketAIReviewModeFields,
  type MarketAIReviewModes,
} from './mode-fields'
import { MarketAIReviewSettingsForm } from './settings-form'

export function MarketAIReviewSettingsSection({
  market,
}: {
  market?: keyof MarketAIReviewModes
} = {}) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  const queryKey = [
    'market-ai-review-settings',
    scope?.userId,
    scope?.sessionId,
    scope?.role,
  ]
  const query = useQuery({
    queryKey,
    queryFn: ({ signal }) => marketAIReviewAPI.settings(scope, signal),
    enabled: Boolean(scope),
    retry: false,
  })
  if (!scope) return null
  if (query.isPending) return <p role='status'>{t('Loading...')}</p>
  if (query.isError) {
    return (
      <div role='alert' className='space-y-3'>
        <p>
          {getSettingsErrorMessage(query.error, t('Failed to load settings'))}
        </p>
        <Button variant='outline' onClick={() => void query.refetch()}>
          {t('Retry')}
        </Button>
      </div>
    )
  }
  const settings = query.data
  const modes = { tool: settings.tool_mode, product: settings.store_mode }
  return (
    <div className='space-y-5'>
      {settings.review_group && settings.review_model ? (
        <p className='text-muted-foreground text-sm'>
          {t('Group')}: {settings.review_group} · {t('Model')}:{' '}
          {settings.review_model}
        </p>
      ) : (
        <p role='status'>{t(copy.routeError)}</p>
      )}
      {scope.role === ROLE.SUPER_ADMIN ? (
        <MarketAIReviewSettingsForm
          market={market}
          defaultValues={modes}
          onSave={async (updates) => {
            const saved = await marketAIReviewAPI.updateModes(
              {
                ...(updates.tool !== undefined && { tool_mode: updates.tool }),
                ...(updates.product !== undefined && {
                  store_mode: updates.product,
                }),
              },
              scope
            )
            client.setQueryData(queryKey, saved)
            return { tool: saved.tool_mode, product: saved.store_mode }
          }}
        />
      ) : (
        <MarketAIReviewModeFields
          market={market}
          value={modes}
          onChange={() => {}}
          disabled
        />
      )}
      {scope.role === ROLE.SUPER_ADMIN && (
        <a
          className='text-primary text-sm underline'
          href='/system-settings/security/moderation'
        >
          {t(copy.routing)}
        </a>
      )}
    </div>
  )
}
