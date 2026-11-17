/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import * as z from 'zod'

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
  const active = enabled && Boolean(userId && sessionId)
  const query = useQuery({
    queryKey: ['hero-sms-capabilities', userId, sessionId],
    queryFn: getHeroSmsPurchaseCapabilities,
    enabled: active,
    retry: false,
    staleTime: CAPABILITY_REFRESH_MS,
    refetchInterval: CAPABILITY_REFRESH_MS,
  })
  const phase = useCapabilityFreshness(query)
  return {
    available:
      active && phase === 'fresh' && query.data?.purchases_available === true,
    pending: active && query.isPending,
  }
}
