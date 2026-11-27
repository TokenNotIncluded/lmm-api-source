/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import * as z from 'zod'

import type { PluginInfo } from '@/features/plugins/api'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { api } from '@/lib/api'
import { CAPABILITY_REFRESH_MS } from '@/lib/runtime-capabilities'
import { useAuthStore } from '@/stores/auth-store'

const capabilityResponse = z.object({
  success: z.literal(true),
  data: z.object({ purchases_available: z.boolean() }),
})

export async function getHeroSmsPurchaseCapabilities() {
  const response = await api.get('/api/hero-sms/capabilities', {
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return capabilityResponse.parse(response.data).data
}

/** A selected settings module does not imply that purchasing is implemented. */
export function useHeroSmsPurchaseCapability(enabled = true) {
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const signedIn = Boolean(userId && sessionId)
  const active = enabled && signedIn
  const query = useQuery({
    queryKey: ['hero-sms-capabilities', userId, sessionId],
    queryFn: getHeroSmsPurchaseCapabilities,
    enabled: active,
    retry: false,
    staleTime: CAPABILITY_REFRESH_MS,
    refetchInterval: active ? CAPABILITY_REFRESH_MS : false,
  })
  const phase = useCapabilityFreshness(query)
  const confirmed =
    signedIn && phase === 'fresh' ? query.data?.purchases_available : undefined
  return {
    available: active && confirmed === true,
    pending: active && query.isPending,
    confirmed,
    signedIn,
  }
}

type NavigationPlugin = Pick<
  PluginInfo,
  'name' | 'available' | 'installed' | 'enabled' | 'navigation_paths'
>

/** The sidebar observes the page's session-scoped result without probing Extensions. */
export function useHeroSmsNavigation(
  enabled: boolean,
  plugins: readonly NavigationPlugin[] | undefined
) {
  const { confirmed, signedIn } = useHeroSmsPurchaseCapability(false)
  if (!enabled || !signedIn || confirmed === false) return false
  // Public Core status does not describe this Extensions route. A user without
  // fresh inventory can enter the page, where the live purchase check runs.
  if (!plugins) return true
  return plugins.some(
    (plugin) =>
      plugin.name === 'hero-sms' &&
      plugin.available &&
      plugin.installed &&
      plugin.enabled &&
      plugin.navigation_paths?.includes('/temporary-activations')
  )
}
