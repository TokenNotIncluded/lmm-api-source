/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { lazy, Suspense, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { SettingsPageProvider } from '@/features/system-settings/components/settings-page-context'
import { hasPermission } from '@/lib/admin-permissions'
import { useAuthStore } from '@/stores/auth-store'

const PromotionLinks = lazy(() =>
  import('@/features/acquisition/promotion-links').then((module) => ({
    default: module.PromotionLinks,
  }))
)
const ConversationStarterSettings = lazy(() =>
  import('@/features/system-settings/content/conversation-starter-settings').then(
    (module) => ({ default: module.ConversationStarterSettings })
  )
)
const CheckinSettingsSection = lazy(() =>
  import('@/features/system-settings/general/checkin-settings-section').then(
    (module) => ({ default: module.CheckinSettingsSection })
  )
)
const HeroSmsSettingsSection = lazy(() =>
  import('@/features/system-settings/operations/hero-sms-settings-section').then(
    (module) => ({ default: module.HeroSmsSettingsSection })
  )
)
const ScriptsAdminSection = lazy(() =>
  import('@/features/scripts/scripts-panel').then((module) => ({
    default: module.ScriptsAdminSection,
  }))
)

export function PluginConfiguration({
  name,
  writable,
}: {
  name: string
  writable: boolean
}) {
  const { t } = useTranslation()
  const [actions, setActions] = useState<HTMLDivElement | null>(null)
  const canWriteAcquisition = useAuthStore((state) =>
    hasPermission(state.auth.user, 'acquisition', 'write')
  )
  const content = {
    acquisition: <PromotionLinks canWrite={canWriteAcquisition && writable} />,
    assistant: <ConversationStarterSettings embedded />,
    checkins: <CheckinSettingsSection />,
    'hero-sms': <HeroSmsSettingsSection />,
    operations: <ScriptsAdminSection />,
  }
  if (!Object.hasOwn(content, name)) return null

  return (
    <fieldset
      disabled={!writable}
      inert={!writable}
      className='min-w-0 border-0 p-0'
    >
      <div
        ref={setActions}
        className='mb-4 flex flex-wrap justify-end gap-2 empty:hidden'
      />
      <SettingsPageProvider
        actionsContainer={actions}
        suppressSectionHeader={false}
      >
        <Suspense fallback={<p role='status'>{t('Loading settings...')}</p>}>
          {content[name as keyof typeof content]}
        </Suspense>
      </SettingsPageProvider>
    </fieldset>
  )
}
