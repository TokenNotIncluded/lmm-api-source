/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQueryClient } from '@tanstack/react-query'
import { useRouter } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useCredentialStore } from '@/stores/auth-store'

export function L0PreviewBanner() {
  const preview = useCredentialStore((state) => state.preview)
  const queryClient = useQueryClient()
  const router = useRouter()
  const { t } = useTranslation()
  if (!preview) return null
  return (
    <div
      role='status'
      className='bg-background text-foreground relative z-[100] flex shrink-0 flex-wrap items-center justify-between gap-2 border-b px-3 py-2'
    >
      <span className='text-sm'>
        {t('Preview L0 page')} ·{' '}
        {t(
          'Empty L0 account preview. Public content is live; account data is simulated.'
        )}{' '}
        {t('Read-only preview. Your account and permissions stay unchanged.')}
      </span>
      <Button
        onClick={async () => {
          await queryClient.cancelQueries()
          queryClient.clear()
          useCredentialStore.getState().stopPreview()
          await router.navigate({ to: preview.returnHref })
          await router.invalidate()
        }}
      >
        {t('Exit preview')}
      </Button>
    </div>
  )
}
