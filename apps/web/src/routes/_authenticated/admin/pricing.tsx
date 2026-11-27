/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createFileRoute, redirect, Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import { NativePricingSection } from '@/features/system-settings/billing/native-pricing-section'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

type Workspace = 'models' | 'groups' | 'router'
export const Route = createFileRoute('/_authenticated/admin/pricing')({
  validateSearch: (search): { workspace?: Workspace } => ({
    workspace:
      search.workspace === 'groups' || search.workspace === 'router'
        ? search.workspace
        : undefined,
  }),
  beforeLoad: () => {
    const { auth } = useAuthStore.getState()
    if (
      !auth.session ||
      !auth.accessToken ||
      auth.user?.role !== ROLE.SUPER_ADMIN
    )
      throw redirect({ to: '/403' })
  },
  component: PricingPage,
})
function PricingPage() {
  const { t } = useTranslation()
  const { workspace = 'models' } = Route.useSearch()
  const choices = {
    models: t('Model Pricing'),
    groups: t('Group Pricing'),
    router: t('Router pricing'),
  }
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>
        {t('Pricing management')}
      </SectionPageLayout.Title>
      <SectionPageLayout.Content>
        <nav
          className='mb-5 flex flex-wrap gap-2'
          aria-label={t('Pricing management')}
        >
          {(Object.keys(choices) as Workspace[]).map((value) => (
            <Button
              key={value}
              variant={workspace === value ? 'secondary' : 'ghost'}
              render={
                <Link
                  to='/admin/pricing'
                  search={{ workspace: value }}
                  aria-current={workspace === value ? 'page' : undefined}
                />
              }
            >
              {choices[value]}
            </Button>
          ))}
        </nav>
        <NativePricingSection
          groupWorkspace={workspace === 'groups'}
          initialMode={workspace === 'router' ? 'upstream_cost' : undefined}
        />
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
