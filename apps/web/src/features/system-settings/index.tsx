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
import { Outlet } from '@tanstack/react-router'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout/components/section-page-layout'
import { Button } from '@/components/ui/button'
import { useStatus } from '@/hooks/use-status'

import { SettingsSearch } from './components/settings-search'
import {
  CoreConfigurationSection,
  type CoreConfigurationDomain,
} from './core-configuration-section'
import { buildSettingsSearchIndex } from './utils/settings-search-index'

export function SystemSettings() {
  return <Outlet />
}

export function SystemSettingsOverview({
  domain,
}: {
  domain?: CoreConfigurationDomain
}) {
  const { t } = useTranslation()
  const { status, loading, error, capabilitiesReady, refetch } = useStatus()
  const unknown = t('Unavailable')
  const catalog = useMemo(() => buildSettingsSearchIndex(t), [t])
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('System Settings')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <SettingsSearch />
      </SectionPageLayout.Actions>
      <SectionPageLayout.Content>
        {domain ? (
          <CoreConfigurationSection domain={domain} />
        ) : (
          <>
            <div className='max-w-2xl space-y-8'>
              <p className='text-muted-foreground text-sm leading-6'>
                {t(
                  'Edit Core settings in the configuration files on the server.'
                )}
              </p>
              <div>
                <p className='text-muted-foreground text-sm'>
                  {t('Default configuration file')}
                </p>
                <code className='mt-1 block break-all'>
                  /etc/lmm.conf.d/core.toml
                </code>
              </div>
              {loading ? (
                <p role='status'>{t('Loading...')}</p>
              ) : error || !capabilitiesReady ? (
                <div className='flex items-center gap-3'>
                  <span role='status'>{unknown}</span>
                  <Button
                    variant='outline'
                    size='sm'
                    onClick={() => void refetch()}
                  >
                    {t('Retry')}
                  </Button>
                </div>
              ) : (
                <dl className='grid gap-6 sm:grid-cols-2'>
                  <div>
                    <dt className='text-muted-foreground text-sm'>
                      {t('System Name')}
                    </dt>
                    <dd className='mt-1 font-medium break-words'>
                      {status?.system_name || unknown}
                    </dd>
                  </div>
                  <div>
                    <dt className='text-muted-foreground text-sm'>
                      {t('API version')}
                    </dt>
                    <dd className='mt-1 font-medium break-words'>
                      {status?.version || unknown}
                    </dd>
                  </div>
                  <div>
                    <dt className='text-muted-foreground text-sm'>
                      {t('Registration Enabled')}
                    </dt>
                    <dd className='mt-1 font-medium'>
                      {typeof status?.register_enabled === 'boolean'
                        ? status.register_enabled
                          ? t('Enabled')
                          : t('Disabled')
                        : unknown}
                    </dd>
                  </div>
                </dl>
              )}
            </div>
            <nav
              aria-label={t('System Settings')}
              className='mt-8 grid gap-x-10 gap-y-8 sm:grid-cols-2 xl:grid-cols-3'
            >
              {catalog.map((group) => (
                <section key={group.group}>
                  <h2 className='mb-2 text-sm font-semibold'>{group.group}</h2>
                  <ul className='space-y-1'>
                    {group.entries
                      .filter((entry) => entry.url !== '/system-settings/site')
                      .map((entry) => (
                        <li key={entry.url}>
                          <a
                            className='text-primary inline-block py-1 text-sm underline-offset-4 hover:underline focus-visible:underline'
                            href={entry.url}
                          >
                            {entry.section}
                          </a>
                        </li>
                      ))}
                  </ul>
                </section>
              ))}
            </nav>
          </>
        )}
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
