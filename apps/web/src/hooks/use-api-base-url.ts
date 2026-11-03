/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { resolveApiBaseUrl } from '@/lib/api-base-url'

import { useStatus } from './use-status'

/** Public request address, not an OAuth issuer or trust decision. */
export function useApiBaseUrl() {
  const { status, loading } = useStatus()
  const baseUrl = status
    ? resolveApiBaseUrl(
        status.server_address ?? status.data?.server_address,
        typeof window === 'undefined' ? '' : window.location.origin
      )
    : null
  return { baseUrl, rootUrl: baseUrl?.replace(/\/v1$/, '') ?? null, loading }
}
