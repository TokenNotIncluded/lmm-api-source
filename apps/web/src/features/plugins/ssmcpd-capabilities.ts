import { useQuery } from '@tanstack/react-query'
import { z } from 'zod'

import { api } from '@/lib/api'
import { ROLE } from '@/lib/roles'
import { CAPABILITY_REFRESH_MS } from '@/lib/runtime-capabilities'
import { useAuthStore } from '@/stores/auth-store'

const capabilitiesSchema = z.object({
  service: z.object({
    mode: z.enum(['standard', 'relay']),
    public_origin: z.string().url(),
    connect_command: z.string(),
    traffic_portal: z.string().url().optional(),
  }),
  account_usage_api_configured: z.boolean(),
  credit_grant_configured: z.boolean(),
})

export async function readSSMCPDSelf(
  path: string,
  scope: { userId: number; sessionId: string },
  signal: AbortSignal
) {
  const result = await api.get(path, {
    authScope: scope,
    signal,
    skipAuthRefresh: true,
    skipBusinessError: true,
    skipErrorHandler: true,
    disableDuplicate: true,
  })
  const { user, session } = useAuthStore.getState().auth
  if (session?.sid !== scope.sessionId || user?.id !== scope.userId)
    throw new Error('Session changed')
  return result.data.data
}

// Sidebar discovery and the page share a session-scoped read. Neither performs a quota lookup here.
export function useSSMCPDCapabilities(enabled = true) {
  const userId = useAuthStore((s) => s.auth.user?.id)
  const sessionId = useAuthStore((s) => s.auth.session?.sid)
  const admin = useAuthStore((s) => (s.auth.user?.role ?? 0) >= ROLE.ADMIN)
  return useQuery({
    queryKey: ['ssmcpd', userId, sessionId, admin, 'capabilities'],
    enabled: enabled && !!userId && !!sessionId,
    retry: false,
    gcTime: 0,
    staleTime: CAPABILITY_REFRESH_MS,
    refetchInterval: CAPABILITY_REFRESH_MS,
    queryFn: async ({ signal }) =>
      capabilitiesSchema.parse(
        await readSSMCPDSelf(
          '/api/plugins/ssmcpd/' + (admin ? 'admin/' : '') + 'capabilities',
          { userId: userId!, sessionId: sessionId! },
          signal
        )
      ),
  })
}
