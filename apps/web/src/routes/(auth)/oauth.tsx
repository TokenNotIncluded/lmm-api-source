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
import { createFileRoute, useNavigate, useSearch } from '@tanstack/react-router'
import i18next from 'i18next'
import { useEffect } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { sanitizeAuthRedirect } from '@/features/auth/lib/auth-redirect'
import { oauthCallbackSearchSchema } from '@/features/auth/lib/external-oauth'
import { readFederatedCeremony } from '@/features/auth/lib/federated-oauth'

function OAuthComponent() {
  const navigate = useNavigate()
  const search = useSearch({ from: '/(auth)/oauth' })

  useEffect(() => {
    if (search.provider === 'wechat' && search.code && search.state) {
      try {
        const ceremony = readFederatedCeremony(
          window.sessionStorage,
          search.state,
          'wechat',
          'login'
        )
        const query = new URLSearchParams({
          code: search.code,
          state: search.state,
        })
        const redirect = sanitizeAuthRedirect(
          search.redirect,
          window.location.origin
        )
        if (redirect) query.set('redirect', redirect)
        void navigate({
          href: `/oauth/${ceremony.slug}?${query}`,
          replace: true,
        })
        return
      } catch {
        /* Unknown or expired legacy entry points cannot start a login. */
      }
    }
    toast.error(i18next.t('OAuth failed'))
    void navigate({ to: '/sign-in', replace: true })
  }, [navigate, search])

  return null
}

export const Route = createFileRoute('/(auth)/oauth')({
  validateSearch: oauthCallbackSearchSchema.extend({
    provider: z
      .enum(['github', 'discord', 'oidc', 'linuxdo', 'telegram', 'wechat'])
      .optional()
      .catch(undefined),
  }),
  component: OAuthComponent,
})
