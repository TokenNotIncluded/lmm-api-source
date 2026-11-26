/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { getAssistantStatus } from '@/features/assistant/api'
import { requestAssistantOpen } from '@/features/assistant/assistant-events'
import { useAuthStore } from '@/stores/auth-store'

import { ConversationStarterSettings } from './conversation-starter-settings'

export function AssistantPluginSettings() {
  const { t } = useTranslation()
  const { user, session } = useAuthStore((state) => state.auth)
  const { data } = useQuery({
    queryKey: ['assistant-status', user?.id, session?.sid],
    queryFn: getAssistantStatus,
    enabled: Boolean(user && session),
    staleTime: 30_000,
    retry: false,
  })
  const modelEnabled = data?.route_available
  return (
    <>
      <section className='mb-6 space-y-3 rounded-lg border p-4'>
        <div className='flex flex-wrap items-center justify-between gap-3'>
          <h2 className='text-sm font-medium'>{t('AI assistant')}</h2>
          <Button variant='outline' onClick={() => requestAssistantOpen()}>
            {t('Open AI assistant')}
          </Button>
        </div>
        <p className='text-muted-foreground text-sm' role='status'>
          {modelEnabled === false
            ? t(
                'Assistant model access is disabled on this host. Conversation starter settings are still available.'
              )
            : modelEnabled === true
              ? t('Enabled')
              : t('Unknown')}
        </p>
      </section>
      <ConversationStarterSettings embedded />
    </>
  )
}
