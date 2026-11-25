/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  skipToken,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { isAxiosError } from 'axios'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  readNativeUsersScope,
} from '@/features/native-users/access'
import {
  parseHeaderNavModules,
  readSupportedSidebarModules,
  serializeSidebarModulesAdmin,
  serializeHeaderNavModules,
} from '@/features/system-settings/maintenance/config'
import { HeaderNavigationForm } from '@/features/system-settings/maintenance/header-navigation-section'
import { SidebarModulesForm } from '@/features/system-settings/maintenance/sidebar-modules-section'
import { ROLE } from '@/lib/roles'
import {
  getSiteNavigation,
  saveSiteNavigation,
  saveSiteSidebar,
  siteNavigationQueryKey,
} from '@/lib/site-navigation'
import { useAuthStore } from '@/stores/auth-store'

export function CommunitySidebarConfiguration() {
  return <CommunityNavigationEditor section='sidebar' />
}

export function CommunityConfiguration() {
  return <CommunityNavigationEditor section='header' />
}

function CommunityNavigationEditor({
  section,
}: {
  section: 'header' | 'sidebar'
}) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const [reload, setReload] = useState(0)
  const scope = readNativeUsersScope(useAuthStore((state) => state.auth))
  const owner = scope?.role === ROLE.SUPER_ADMIN
  const queryKey = [
    'community-navigation-editor',
    scope?.userId,
    scope?.sessionId,
  ]
  const query = useQuery({
    queryKey,
    enabled: owner,
    queryFn: scope
      ? ({ signal }) => getSiteNavigation(nativeUsersOptions(scope, signal))
      : skipToken,
    retry: false,
    staleTime: Infinity,
    gcTime: 0,
  })
  const config = useMemo(
    () => parseHeaderNavModules(query.data?.value),
    [query.data?.value]
  )
  const sidebar = useMemo(
    () => readSupportedSidebarModules(query.data?.sidebar?.value),
    [query.data?.sidebar?.value]
  )
  const save = useMutation({
    retry: false,
    mutationFn: async (value: string) => {
      if (
        !scope ||
        !owner ||
        !isCurrentNativeUsersScope(scope) ||
        !query.data
      ) {
        throw new Error('Session changed')
      }
      if (section === 'sidebar') {
        if (!sidebar || !query.data.sidebar)
          throw new Error('Sidebar unavailable')
        return saveSiteSidebar(
          value,
          query.data.sidebar.value,
          nativeUsersOptions(scope)
        )
      }
      return saveSiteNavigation(
        value,
        nativeUsersOptions(scope),
        query.data.value
      )
    },
    onSuccess: (saved) => {
      if (!scope || !isCurrentNativeUsersScope(scope)) return
      // A PUT returns only the part that changed. Keep the other navigation
      // settings in both caches until the fresh public projection arrives.
      const merge = (old?: Awaited<ReturnType<typeof getSiteNavigation>>) => {
        if (!old) return old
        return section === 'sidebar'
          ? { ...old, sidebar: saved.data }
          : { ...old, value: saved.data.value, revision: saved.data.revision }
      }
      client.setQueryData(queryKey, merge)
      client.setQueryData(siteNavigationQueryKey, merge)
      void client.invalidateQueries({ queryKey: siteNavigationQueryKey })
      toast.success(t('Settings saved'))
    },
  })
  if (!owner) return <ErrorState title={t('Owner access required')} />
  if (query.isPending) return <p role='status'>{t('Loading settings...')}</p>
  if (query.isError) {
    return (
      <ErrorState
        title={t('Failed to load settings')}
        onRetry={() => void query.refetch()}
      />
    )
  }
  if (section === 'sidebar' && !sidebar) {
    return (
      <p role='status'>
        {t('This server does not expose controls for this section.')}
      </p>
    )
  }
  const conflict =
    isAxiosError(save.error) && save.error.response?.status === 409
  return (
    <div className='space-y-4'>
      {save.isError && (
        <div role='alert' className='space-y-2'>
          <p>
            {t(
              conflict
                ? 'Settings changed on the server. Reload before saving.'
                : 'Failed to save settings. Your changes are retained.'
            )}
          </p>
          <Button
            variant='outline'
            disabled={query.isFetching}
            onClick={async () => {
              const refreshed = await query.refetch()
              if (!refreshed.isError) {
                setReload((n) => n + 1)
                save.reset()
              }
            }}
          >
            {t('Discard changes and reload')}
          </Button>
        </div>
      )}
      <fieldset disabled={save.isPending || query.isFetching || conflict}>
        {section === 'sidebar' && sidebar ? (
          <SidebarModulesForm
            key={`sidebar:${query.data.sidebar?.revision}:${reload}`}
            config={sidebar}
            initialSerialized={serializeSidebarModulesAdmin(sidebar)}
            onSave={(value) => save.mutate(value)}
            saving={save.isPending}
            disabled={query.isFetching || conflict}
          />
        ) : (
          <HeaderNavigationForm
            key={`${query.data.revision}:${reload}`}
            config={config}
            initialSerialized={serializeHeaderNavModules(config)}
            onSave={(value) => save.mutate(value)}
            saving={save.isPending}
            disabled={query.isFetching || conflict}
          />
        )}
      </fieldset>
    </div>
  )
}
