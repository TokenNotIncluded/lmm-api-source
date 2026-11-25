/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { createFileRoute, Link, redirect } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout/components/section-page-layout'
import { AIDirectory } from '@/features/ai-directory'
import { AIDirectoryAdsSection } from '@/features/system-settings/site/ai-directory-ads-section'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

export const Route = createFileRoute('/_authenticated/ai-directory/')({
  validateSearch: (search: Record<string, unknown>): { manage?: 'ads' } => ({
    manage: search.manage === 'ads' ? 'ads' : undefined,
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

  if (!manage || !isOwner) return <AIDirectory />
  return (
    <SectionPageLayout>
      <SectionPageLayout.Breadcrumb>
        <Link
          to='/ai-directory'
          search={{}}
          className='text-primary text-sm underline underline-offset-4'
        >
          {t('Back')}
        </Link>
      </SectionPageLayout.Breadcrumb>
      <SectionPageLayout.Title>{t('Manage ads')}</SectionPageLayout.Title>

      <SectionPageLayout.Content>
        <AIDirectoryAdsSection />
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
