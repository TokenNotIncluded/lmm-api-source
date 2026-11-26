/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ErrorState } from '@/components/error-state'
import {
  isCurrentNativeUsersScope,
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { ROLE } from '@/lib/roles'
import {
  getSiteNavigation,
  saveSiteNavigation,
  siteNavigationQueryKey,
} from '@/lib/site-navigation'
import { useAuthStore } from '@/stores/auth-store'

import { validateSystemOptions } from '../api'
import { parseHeaderNavModules, serializeHeaderNavModules } from './config'
import { HeaderNavigationSection } from './header-navigation-section'

export function HeaderNavigationPluginSettings() {
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  if (!scope || scope.role !== ROLE.SUPER_ADMIN) return null
  return <NavigationEditor key={JSON.stringify(scope)} scope={scope} />
}

function NavigationEditor({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const queryKey = [
    ...siteNavigationQueryKey,
    scope.userId,
    scope.sessionId,
    scope.role,
  ]
  const request = {
    authScope: { userId: scope.userId, sessionId: scope.sessionId },
    skipAuthRefresh: true,
  }
  const query = useQuery({
    queryKey,
    queryFn: ({ signal }) => getSiteNavigation({ ...request, signal }),
    retry: false,
  })
  const value = query.data?.value
  const config = useMemo(() => parseHeaderNavModules(value), [value])
  const save = useMutation({
    retry: false,
    mutationFn: async (value: string) => {
      if (!isCurrentNativeUsersScope(scope)) {
        throw new Error(t('Access Forbidden'))
      }
      const validation = await validateSystemOptions(
        { HeaderNavModules: value },
        request
      )
      if (!validation.success) {
        throw new Error(validation.message || t('Save failed'))
      }
      if (!isCurrentNativeUsersScope(scope)) {
        throw new Error(t('Access Forbidden'))
      }
      return saveSiteNavigation(value, request)
    },
    onSuccess: (saved) => {
      if (!isCurrentNativeUsersScope(scope)) return
      client.setQueryData(queryKey, saved.data)
      void client.invalidateQueries({ queryKey: siteNavigationQueryKey })
      void client.invalidateQueries({
        queryKey: ['system-options', 'navigation'],
      })
      toast.success(t('Settings saved'))
    },
    onError: () => toast.error(t('Save failed')),
  })
  if (query.isPending) return <p role='status'>{t('Loading settings...')}</p>
  if (query.isError || value === undefined) {
    return (
      <ErrorState
        title={t('Unable to load settings')}
        onRetry={() => void query.refetch()}
      />
    )
  }
  return (
    <fieldset disabled={save.isPending} className='min-w-0 border-0 p-0'>
      {save.isError && <p role='alert'>{t('Save failed')}</p>}
      <HeaderNavigationSection
        config={config}
        initialSerialized={serializeHeaderNavModules(config)}
        onSave={save.mutate}
        saving={save.isPending}
      />
    </fieldset>
  )
}
