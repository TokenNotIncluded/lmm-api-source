/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { requestAssistantOpen } from '@/features/assistant/assistant-events'

const settingsReads = {
  identity: ['/api/option/'],
  billing: ['/api/option/'],
  console: ['/api/option/'],
  routing: [
    '/core/v1/admin/request-limits',
    '/core/v1/admin/routing/auto-groups',
  ],
  honeypot: ['/core/v1/admin/abuse/policies'],
  gemini: ['/core/v1/admin/routing/configuration'],
  'hero-sms': ['/api/option/hero-sms'],
} as const

export function ConfigurationAssistant({
  domain,
  revision,
}: {
  domain: keyof typeof settingsReads
  revision?: number | string
}) {
  const { t } = useTranslation()
  function openDraft() {
    const context = JSON.stringify({
      domain,
      reads: settingsReads[domain],
      revision,
    })
    requestAssistantOpen(
      'service',
      t(
        'Read the active Core configuration and its source. Prepare a Git configuration diff for my review. Leave credentials out of the proposal. Context: {{context}}',
        { context }
      )
    )
  }
  return (
    <Button type='button' variant='outline' size='sm' onClick={openDraft}>
      {t('Ask AI to edit')}
    </Button>
  )
}
