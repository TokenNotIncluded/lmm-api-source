/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { useEffect, useState } from 'react'

import { frontendEntry } from '@/lib/frontend-entry'

export function useFrontendUpdateAvailable(enabled = true): boolean {
  const [available, setAvailable] = useState(false)
  useEffect(() => {
    if (!enabled || import.meta.env.DEV) return
    const origin = window.location.origin
    const current = frontendEntry(document, origin)
    if (!current) return
    let lastCheck = 0
    let request: AbortController | null = null
    let disposed = false
    const check = async () => {
      if (
        disposed ||
        document.visibilityState !== 'visible' ||
        request ||
        Date.now() - lastCheck < 60_000
      ) {
        return
      }
      lastCheck = Date.now()
      request = new AbortController()
      const timeout = window.setTimeout(() => request?.abort(), 10_000)
      try {
        const response = await fetch('/index.html', {
          cache: 'no-store',
          signal: request.signal,
        })
        if (
          !response.ok ||
          !response.headers.get('content-type')?.includes('text/html')
        ) {
          return
        }
        const remote = frontendEntry(
          new DOMParser().parseFromString(await response.text(), 'text/html'),
          origin
        )
        if (!disposed && remote) setAvailable(remote !== current)
      } catch {
        // A transient outage must not interrupt a payment or replace the page.
      } finally {
        window.clearTimeout(timeout)
        request = null
      }
    }
    void check()
    const onResume = () => void check()
    window.addEventListener('focus', onResume)
    window.addEventListener('pageshow', onResume)
    document.addEventListener('visibilitychange', onResume)
    return () => {
      disposed = true
      request?.abort()
      window.removeEventListener('focus', onResume)
      window.removeEventListener('pageshow', onResume)
      document.removeEventListener('visibilitychange', onResume)
    }
  }, [enabled])
  return enabled && available
}
