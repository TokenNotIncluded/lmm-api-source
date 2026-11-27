/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Link } from '@tanstack/react-router'
import { ArrowUpRight, ChevronRight } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { getSystemSettingsNavGroups } from '@/components/layout/config/system-settings.config'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { ROLE } from '@/lib/roles'
import { getSettingsBoundary } from '@/lib/system-settings-access'
import { useAuthStore } from '@/stores/auth-store'

import { SettingsSearch } from './components/settings-search'
import { useSystemOptions } from './hooks/use-system-options'
import { settingsDestination } from './utils/settings-search-index'

export function SettingsOverview() {
  const { t } = useTranslation()
  const role = useAuthStore((state) => state.auth.user?.role ?? ROLE.GUEST)
  const isRoot = role === ROLE.SUPER_ADMIN
  const options = useSystemOptions(isRoot, false, 'session')
  const phase = useCapabilityFreshness(options)
  const groups = getSystemSettingsNavGroups(
    t,
    role,
    'core',
    false,
    isRoot && phase === 'fresh' ? options.data : undefined
  ).flatMap((group) => group.items)

  const adminShortcuts =
    role === ROLE.ADMIN
      ? [
          { panel: 'assistant', title: t('AI assistant settings') },
          { panel: 'toolmarket', title: t('Market AI first review') },
        ]
      : []

  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Core Settings')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <SettingsSearch />
      </SectionPageLayout.Actions>
      <SectionPageLayout.Content>
        <div className='mx-auto w-full max-w-5xl space-y-8 py-2'>
          {groups.length > 0 && (
            <div className='grid gap-x-12 gap-y-8 md:grid-cols-2'>
              {groups.map((group) => {
                const Icon = group.icon
                return (
                  <section key={group.title} className='min-w-0 border-t pt-4'>
                    <h2 className='mb-2 flex items-center gap-2 text-base font-semibold'>
                      {Icon && (
                        <Icon
                          aria-hidden='true'
                          className='text-muted-foreground size-4'
                        />
                      )}
                      {group.title}
                    </h2>
                    <ul>
                      {group.items?.map((item) => {
                        if (!item.url) return null
                        const url =
                          getSettingsBoundary(item.url)?.canonical ?? item.url
                        return (
                          <li key={item.url}>
                            <Link
                              {...settingsDestination(url)}
                              className='hover:bg-muted focus-visible:ring-ring flex min-h-11 items-center justify-between gap-3 rounded-md px-2 py-2 text-sm focus-visible:ring-2'
                            >
                              <span>{item.title}</span>
                              <ChevronRight
                                aria-hidden='true'
                                className='text-muted-foreground size-4 shrink-0'
                              />
                            </Link>
                          </li>
                        )
                      })}
                    </ul>
                  </section>
                )
              })}
            </div>
          )}
          <div className='flex flex-wrap gap-x-6 border-t pt-3'>
            <Link
              to='/extensions/plugins'
              className='text-primary inline-flex min-h-11 items-center gap-2 text-sm font-medium hover:underline'
            >
              {t('Plugin management')}
              <ArrowUpRight aria-hidden='true' className='size-4' />
            </Link>
            {adminShortcuts.map(({ panel, title }) => (
              <Link
                key={panel}
                to='/extensions/plugins'
                search={{ configure: panel }}
                className='text-primary inline-flex min-h-11 items-center gap-2 text-sm font-medium hover:underline'
              >
                {title}
                <ArrowUpRight aria-hidden='true' className='size-4' />
              </Link>
            ))}
          </div>
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
