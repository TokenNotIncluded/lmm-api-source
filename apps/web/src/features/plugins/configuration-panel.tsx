/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { lazy, Suspense, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { SettingsPageProvider } from '@/features/system-settings/components/settings-page-context'
import { hasPermission } from '@/lib/admin-permissions'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { configurationOwner } from './configuration'

const PromotionLinks = lazy(() =>
  import('@/features/acquisition/promotion-links').then((module) => ({
    default: module.PromotionLinks,
  }))
)
const AssistantPluginSettings = lazy(() =>
  import('@/features/system-settings/content/assistant-plugin-settings').then(
    (module) => ({ default: module.AssistantPluginSettings })
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

const AIDirectorySection = lazy(() =>
  import('@/features/system-settings/site/ai-directory-section').then(
    (module) => ({
      default: module.AIDirectorySection,
    })
  )
)
const HeaderNavigationPluginSettings = lazy(() =>
  import('@/features/system-settings/maintenance/header-navigation-plugin-settings').then(
    (module) => ({ default: module.HeaderNavigationPluginSettings })
  )
)
const IoNetDeploymentSettingsSection = lazy(() =>
  import('@/features/system-settings/integrations/ionet-deployment-settings-section').then(
    (module) => ({ default: module.IoNetDeploymentSettingsSection })
  )
)
const RSSFeedsSection = lazy(() =>
  import('@/features/system-settings/site/rss-feeds-section').then(
    (module) => ({
      default: module.RSSFeedsSection,
    })
  )
)
const AIDirectoryAdsSection = lazy(() =>
  import('@/features/system-settings/site/ai-directory-ads-section').then(
    (module) => ({
      default: module.AIDirectoryAdsSection,
    })
  )
)
const MerchantStoreSettingsSection = lazy(() =>
  import('@/features/system-settings/integrations/merchant-store-settings-section').then(
    (module) => ({ default: module.MerchantStoreSettingsSection })
  )
)

const MarketAIReviewSettingsSection = lazy(() =>
  import('@/features/market-ai-review/settings').then((module) => ({
    default: module.MarketAIReviewSettingsSection,
  }))
)

export function PluginConfiguration({
  name,
  writable,
  marketReviewEnabled,
  onSelect,
}: {
  name: string
  writable: boolean
  marketReviewEnabled: boolean
  onSelect: (name: string) => void
}) {
  const { t } = useTranslation()
  const [actions, setActions] = useState<HTMLDivElement | null>(null)
  const canWriteAcquisition = useAuthStore((state) =>
    hasPermission(state.auth.user, 'acquisition', 'write')
  )
  const root = useAuthStore(
    (state) => state.auth.user?.role === ROLE.SUPER_ADMIN
  )
  const marketReview = (market: 'product' | 'tool') =>
    marketReviewEnabled ? (
      <MarketAIReviewSettingsSection key={market} market={market} />
    ) : (
      <p role='status'>{t('AI first review is disabled')}</p>
    )
  const content = {
    acquisition: (
      <>
        <PromotionLinks canWrite={canWriteAcquisition && writable} />
        <p className='text-muted-foreground mt-4 text-sm'>
          {t(
            'Automatic registration and invitation rewards, referral thresholds and penalties have no online policy implementation in this deployment.'
          )}
        </p>
      </>
    ),
    assistant: <AssistantPluginSettings />,
    checkins: <CheckinSettingsSection />,
    'hero-sms': <HeroSmsSettingsSection />,
    operations: <ScriptsAdminSection />,
    rss: root && <RSSFeedsSection />,
    'model-deployment': root && <IoNetDeploymentSettingsSection />,
    community: root && <AIDirectorySection />,
    'directory-ads': root && <AIDirectoryAdsSection />,
    'header-navigation': root && <HeaderNavigationPluginSettings />,
    store: root && <MerchantStoreSettingsSection />,
    'store-review': marketReview('product'),
    toolmarket: marketReview('tool'),
  }
  if (!Object.hasOwn(content, name)) return null

  const owner = configurationOwner(name)
  const panels =
    owner === 'operations'
      ? [
          ['operations', t('Scripts')],
          ['rss', t('RSS')],
          ['model-deployment', t('Model Deployment')],
        ]
      : owner === 'community'
        ? [
            ['community', t('AI directory')],
            ['header-navigation', t('Header navigation')],
            ['directory-ads', t('Advertisement moderation')],
          ]
        : owner === 'store'
          ? [
              ['store', t('Merchant store settings')],
              ['store-review', t('Market AI first review')],
            ]
          : undefined
  return (
    <>
      {root && panels && (
        <div
          role='group'
          aria-label={t('Configuration')}
          className='mb-4 flex flex-wrap gap-2'
        >
          {panels.map(([panel, label]) => (
            <Button
              key={panel}
              type='button'
              variant={panel === name ? 'secondary' : 'ghost'}
              aria-pressed={panel === name}
              className='min-h-11'
              onClick={() => onSelect(panel)}
            >
              {label}
            </Button>
          ))}
        </div>
      )}
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
    </>
  )
}
