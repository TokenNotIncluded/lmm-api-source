/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { createFileRoute, Link, redirect } from '@tanstack/react-router'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout/components/section-page-layout'
import { RSSReader } from '@/features/rss'
import { SettingsPageProvider } from '@/features/system-settings/components/settings-page-context'
import { RSSFeedsSection } from '@/features/system-settings/site/rss-feeds-section'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

export const Route = createFileRoute('/_authenticated/rss/')({
  validateSearch: (search: Record<string, unknown>): { manage?: 'feeds' } => ({
    manage: search.manage === 'feeds' ? 'feeds' : undefined,
  }),
  beforeLoad: ({ search }) => {
    const { auth } = useAuthStore.getState()
    if (
      search.manage &&
      (!auth.session ||
        !auth.accessToken ||
        auth.user?.role !== ROLE.SUPER_ADMIN)
    ) {
      throw redirect({ to: '/403' })
    }
  },
  component: Page,
})

function Page() {
  const { manage } = Route.useSearch()
  const isOwner = useAuthStore(({ auth }) =>
    Boolean(
      auth.session && auth.accessToken && auth.user?.role === ROLE.SUPER_ADMIN
    )
  )
  const { t } = useTranslation()
  const [actions, setActions] = useState<HTMLDivElement | null>(null)
  if (!manage || !isOwner) return <RSSReader />
  return (
    <SectionPageLayout>
      <SectionPageLayout.Breadcrumb>
        <Link
          to='/rss'
          search={{}}
          className='text-primary text-sm underline underline-offset-4'
        >
          {t('Back')}
        </Link>
      </SectionPageLayout.Breadcrumb>
      <SectionPageLayout.Title>{t('Manage feeds')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <div ref={setActions} />
      </SectionPageLayout.Actions>
      <SectionPageLayout.Content>
        <SettingsPageProvider actionsContainer={actions}>
          <RSSFeedsSection />
        </SettingsPageProvider>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
