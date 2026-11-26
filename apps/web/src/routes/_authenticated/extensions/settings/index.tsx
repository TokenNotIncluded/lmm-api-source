/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createFileRoute, Link, redirect } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { getSystemSettingsNavGroups } from '@/components/layout/config/system-settings.config'
import { SettingsPageFrame } from '@/features/system-settings/components/settings-page'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

export const Route = createFileRoute('/_authenticated/extensions/settings/')({
  beforeLoad: () => {
    const { auth } = useAuthStore.getState()
    if (
      !auth.session ||
      !auth.accessToken ||
      auth.user?.role !== ROLE.SUPER_ADMIN
    ) {
      throw redirect({ to: '/403' })
    }
  },
  component: PluginSettings,
})

function PluginSettings() {
  const { t } = useTranslation()
  const groups = getSystemSettingsNavGroups(t, ROLE.SUPER_ADMIN, 'extensions')
  return (
    <SettingsPageFrame title={t('Plugin Settings')}>
      <Link
        to='/extensions/plugins'
        className='mb-4 inline-flex min-h-10 items-center text-sm underline'
      >
        {t('Plugin management')}
      </Link>
      <div className='grid gap-5 md:grid-cols-2'>
        {groups.flatMap((group) =>
          group.items.map((category) => (
            <section key={category.title} className='rounded-lg border p-4'>
              <h2 className='mb-2 font-medium'>{category.title}</h2>
              <ul>
                {category.items?.map((item) => (
                  <li key={item.url}>
                    <Link
                      to={item.url}
                      className='hover:bg-muted focus-visible:ring-ring flex min-h-10 items-center rounded px-2 text-sm focus-visible:ring-2'
                    >
                      {item.title}
                    </Link>
                  </li>
                ))}
              </ul>
            </section>
          ))
        )}
      </div>
    </SettingsPageFrame>
  )
}
