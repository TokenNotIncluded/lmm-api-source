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
import { useQuery } from '@tanstack/react-query'
import { useParams } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { getAssistantStatus } from '@/features/assistant/api'
import { requestAssistantOpen } from '@/features/assistant/assistant-events'
import { useAuthStore } from '@/stores/auth-store'

import { SettingsPage, SettingsPageFrame } from '../components/settings-page'
import {
  normalizeAssistantSearchProvider,
  type ContentSettings,
  type SystemOption,
} from '../types'
import { DEFAULT_ASSISTANT_TOOL_POLICY } from './assistant-tool-policy'
import { ConversationStarterSettings } from './conversation-starter-settings'
import {
  CONTENT_DEFAULT_SECTION,
  getContentSectionContent,
  getContentSectionMeta,
} from './section-registry.tsx'

const defaultContentSettings: ContentSettings = {
  'console_setting.api_info': '[]',
  'console_setting.announcements': '[]',
  'console_setting.faq': '[]',
  'console_setting.uptime_kuma_groups': '[]',
  'console_setting.api_info_enabled': true,
  'console_setting.announcements_enabled': true,
  'console_setting.faq_enabled': true,
  'console_setting.uptime_kuma_enabled': false,
  DataExportEnabled: false,
  DataExportDefaultTime: 'hour',
  DataExportInterval: 5,
  Chats: '[]',
  AssistantEnabled: true,
  AssistantGroup: 'default',
  AssistantModel: 'deepseek-v4-flash',
  AssistantModerationEnabled: false,
  AssistantModerationGroup: 'default',
  AssistantModerationModel: 'omni-moderation-latest',
  AssistantReasoningEffort: 'auto',
  AssistantStreamEnabled: true,
  AssistantTemperature: 0.2,
  AssistantMaxTokens: 900,
  AssistantNewUserGiftMaxCredits: 0,
  AssistantAgentLoopEnabled: true,
  AssistantMaxSteps: 6,
  AssistantTimeoutSeconds: 45,
  AssistantCacheEnabled: true,
  AssistantCacheTTLMinutes: 1440,
  AssistantPersona: '',
  AssistantSystemPrompt: '',
  AssistantPreConversationPresets: '',
  AssistantSearchProvider: 'none',
  AssistantSearchURL: '',
  AssistantSearchAPIKey: '',
  AssistantSearchMCPTool: '',
  AssistantToolPolicy: DEFAULT_ASSISTANT_TOOL_POLICY,
  AssistantSkills: '',
  AssistantSkillFiles: '[]',
  AssistantRegistrationAutoSuspendEnabled: true,
  AssistantRegistrationDailySuspendCap: 5,
  AssistantRetentionEnabled: true,
  AssistantActiveRetentionDays: 90,
  AssistantArchivedRetentionDays: 30,
  AssistantSecurityRetentionDays: 180,
  AssistantRetentionIntervalHours: 24,
  DrawingEnabled: false,
  MjNotifyEnabled: false,
  MjAccountFilterEnabled: false,
  MjForwardUrlEnabled: false,
  MjModeClearEnabled: false,
  MjActionCheckSuccessEnabled: false,
}

function resolveContentSettings(
  settings: ContentSettings,
  raw: SystemOption[] | undefined
): ContentSettings {
  if (!raw || raw.length === 0) return settings

  const optionMap = new Map(raw.map((item) => [item.key, item.value]))
  const next = { ...settings }

  next.AssistantSearchProvider = normalizeAssistantSearchProvider(
    optionMap.get('AssistantSearchProvider'),
    next.AssistantSearchURL
  )

  const legacyMap = [
    { current: 'console_setting.announcements', legacy: 'Announcements' },
    { current: 'console_setting.api_info', legacy: 'ApiInfo' },
    { current: 'console_setting.faq', legacy: 'FAQ' },
  ] as const

  for (const { current, legacy } of legacyMap) {
    if (!optionMap.has(current)) {
      const legacyValue = optionMap.get(legacy)
      if (legacyValue !== undefined) {
        next[current] = legacyValue
      }
    }
  }

  if (!optionMap.has('console_setting.uptime_kuma_groups')) {
    const legacyUrl = optionMap.get('UptimeKumaUrl')
    const legacySlug = optionMap.get('UptimeKumaSlug')
    if (legacyUrl && legacySlug) {
      next['console_setting.uptime_kuma_groups'] = JSON.stringify([
        { id: 1, categoryName: 'Legacy', url: legacyUrl, slug: legacySlug },
      ])
    }
  }

  return next
}

export function ContentSettings() {
  const { section } = useParams({
    from: '/_authenticated/system-settings/content/$section',
  })
  if (section === 'assistant') {
    return <AssistantPluginSettings />
  }
  return (
    <SettingsPage
      routePath='/_authenticated/system-settings/content/$section'
      defaultSettings={defaultContentSettings}
      defaultSection={CONTENT_DEFAULT_SECTION}
      getSectionContent={getContentSectionContent}
      getSectionMeta={getContentSectionMeta}
      loadingMessage='Loading content settings...'
      resolveSettings={resolveContentSettings}
    />
  )
}

function AssistantPluginSettings() {
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
    <SettingsPageFrame title={t('AI assistant settings')}>
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
      <ConversationStarterSettings />
    </SettingsPageFrame>
  )
}
