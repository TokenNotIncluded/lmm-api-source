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
import { useCallback, useEffect, useRef, useState } from 'react'

import { useAuthStore } from '@/stores/auth-store'

import { getDeploymentSettings, testDeploymentConnection } from '../api'

interface ConnectionState {
  loading: boolean
  ok: boolean | null
  error: string | null
}

// Connection cache (5 minutes TTL)
const CONNECTION_CACHE_TTL = 5 * 60 * 1000
let connectionCache: {
  ok: boolean
  scope: string
  timestamp: number
} | null = null

function getCachedConnection(scope: string): boolean | null {
  if (!connectionCache || connectionCache.scope !== scope) return null
  if (Date.now() - connectionCache.timestamp > CONNECTION_CACHE_TTL) {
    connectionCache = null
    return null
  }
  return connectionCache.ok
}

function setCachedConnection(ok: boolean, scope: string) {
  connectionCache = { ok, scope, timestamp: Date.now() }
}

export function clearConnectionCache() {
  connectionCache = null
}

type LoadingPhase = 'idle' | 'settings' | 'connection' | 'done'
type DeploymentState = {
  owner: string
  loading: boolean
  loadingPhase: LoadingPhase
  settings: Record<string, unknown>
  connectionState: ConnectionState
}
function initialState(owner: string): DeploymentState {
  return {
    owner,
    loading: true,
    loadingPhase: 'settings',
    settings: { 'model_deployment.ionet.enabled': false },
    connectionState: { loading: false, ok: null, error: null },
  }
}

export function useModelDeploymentSettings() {
  const owner = useAuthStore(
    (s) => `${s.auth.user?.id}:${s.auth.user?.role}:${s.auth.session?.sid}`
  )
  const [snapshot, setSnapshot] = useState(() => initialState(owner))
  // An account transition must not commit the previous account's settings,
  // even before the new account's passive loading effect runs.
  if (snapshot.owner !== owner) setSnapshot(initialState(owner))
  const state = snapshot.owner === owner ? snapshot : initialState(owner)
  const fetchGenerationRef = useRef({ value: 0 })

  const fetchAll = useCallback(
    async (useCache = true) => {
      const generation = ++fetchGenerationRef.current.value
      const isCurrent = () =>
        fetchGenerationRef.current.value === generation &&
        owner ===
          `${useAuthStore.getState().auth.user?.id}:${useAuthStore.getState().auth.user?.role}:${useAuthStore.getState().auth.session?.sid}`
      const update = (patch: Partial<Omit<DeploymentState, 'owner'>>) => {
        setSnapshot((previous) =>
          previous.owner === owner ? { ...previous, ...patch } : previous
        )
      }
      try {
        const response = await getDeploymentSettings()
        if (!isCurrent()) return
        const isEnabled = response?.success && response?.data?.enabled === true
        update({ settings: { 'model_deployment.ionet.enabled': isEnabled } })
        if (!isEnabled) {
          update({ connectionState: { loading: false, ok: null, error: null } })
          return
        }
        const scope = `${owner}:${response.data?.revision}`
        if (useCache) {
          const cached = getCachedConnection(scope)
          if (cached !== null) {
            update({
              connectionState: { loading: false, ok: cached, error: null },
            })
            return
          }
        }
        update({
          loadingPhase: 'connection',
          connectionState: { loading: true, ok: null, error: null },
        })
        try {
          const connection = await testDeploymentConnection()
          if (!isCurrent()) return
          const ok = connection?.success === true
          setCachedConnection(ok, scope)
          update({
            connectionState: {
              loading: false,
              ok,
              error: ok ? null : connection?.message || 'Connection failed',
            },
          })
        } catch (error: unknown) {
          if (!isCurrent()) return
          setCachedConnection(false, scope)
          update({
            connectionState: {
              loading: false,
              ok: false,
              error:
                error instanceof Error ? error.message : 'Connection failed',
            },
          })
        }
      } catch {
        if (!isCurrent()) return
        update({ connectionState: { loading: false, ok: null, error: null } })
      } finally {
        if (isCurrent()) update({ loadingPhase: 'done', loading: false })
      }
    },
    [owner]
  )
  useEffect(() => {
    const pending = fetchGenerationRef.current
    clearConnectionCache()
    void fetchAll(true)
    return () => {
      ++pending.value
    }
  }, [fetchAll])

  const reload = useCallback(
    (useCache: boolean) => {
      setSnapshot((previous) =>
        previous.owner === owner
          ? { ...previous, loading: true, loadingPhase: 'settings' }
          : initialState(owner)
      )
      return fetchAll(useCache)
    },
    [fetchAll, owner]
  )
  const reloadWithoutCache = useCallback(() => {
    clearConnectionCache()
    return reload(false)
  }, [reload])
  useEffect(() => {
    const handler = () => {
      void reload(true)
    }
    window.addEventListener('focus', handler)
    return () => window.removeEventListener('focus', handler)
  }, [reload])

  return {
    loading: state.loading,
    loadingPhase: state.loadingPhase,
    settings: state.settings,
    isIoNetEnabled: Boolean(state.settings['model_deployment.ionet.enabled']),
    refresh: reloadWithoutCache,
    connectionLoading: state.connectionState.loading,
    connectionOk: state.connectionState.ok,
    connectionError: state.connectionState.error,
    testConnection: reloadWithoutCache,
  }
}
