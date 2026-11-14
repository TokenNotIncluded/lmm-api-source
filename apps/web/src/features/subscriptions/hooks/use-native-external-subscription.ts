/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { z } from 'zod'

import {
  currentCommerceScope,
  readPending,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'

import type { NativePlan } from '../native-api'

type PendingWithOrderId = {
  order_id?: string | number | null
}

export function useNativeExternalSubscription<
  TPending extends PendingWithOrderId,
  TPrice,
>({
  scope,
  area,
  pendingSchema,
  plans,
  onBlockedChange,
  listPrices,
  onRecover,
}: {
  scope: CommerceScope
  area: string
  pendingSchema: z.ZodType<TPending>
  plans: NativePlan[]
  onBlockedChange: (blocked: boolean) => void
  listPrices: (
    scope: CommerceScope,
    planId: NativePlan['id']
  ) => Promise<TPrice[]>
  onRecover: (saved: TPending) => Promise<void>
}) {
  const { t } = useTranslation()
  const [prices, setPrices] = useState<TPrice[]>([])
  const [pending, setPending] = useState<TPending | null>(null)
  const [ready, setReady] = useState(false)
  const [storageBlocked, setStorageBlocked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const mounted = useRef(false)
  const running = useRef(false)
  const current = () => mounted.current && currentCommerceScope(scope)

  function remember(value: TPending | null) {
    if (!current()) return false
    try {
      savePending(scope, area, value)
      setPending(value)
      return true
    } catch {
      setStorageBlocked(true)
      return false
    }
  }

  async function run(action: () => Promise<void>) {
    if (running.current || !current() || storageBlocked) return
    running.current = true
    setBusy(true)
    setNotice('')
    try {
      await action()
    } catch {
      if (current()) {
        setNotice(
          t('Result unknown. Retry the saved request before starting another.')
        )
      }
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }

  useEffect(() => {
    mounted.current = true
    try {
      const saved = readPending(scope, area, pendingSchema)
      setPending(saved)
      // Recovery is read-only: never prepare or confirm automatically.
      if (saved?.order_id) {
        void run(async () => onRecover(saved))
      }
    } catch {
      setStorageBlocked(true)
    }
    setReady(true)
    return () => {
      mounted.current = false
    }
    // The parent remounts this subtree for each account/session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    onBlockedChange(!ready || !!pending || storageBlocked)
  }, [ready, pending, storageBlocked, onBlockedChange])

  useEffect(() => {
    let active = true
    setPrices([])
    void Promise.all(plans.map((plan) => listPrices(scope, plan.id)))
      .then((catalog) => {
        if (active && current()) setPrices(catalog.flat())
      })
      .catch(() => {
        if (active && current()) setNotice(t('Request failed'))
      })
    return () => {
      active = false
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plans, scope.userId, scope.sessionId])

  return {
    prices,
    pending,
    ready,
    storageBlocked,
    busy,
    notice,
    setNotice,
    current,
    remember,
    run,
  }
}
