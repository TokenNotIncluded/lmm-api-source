/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { storeRequestKey } from './utils'

export async function prepareStockImport(input: {
  userId: number
  productId: string
  variantId?: string
  items: string[]
}) {
  // Keep only a digest and operation key across dialog close/reload. Private
  // inventory content stays in the form and the authenticated request.
  const bytes = new TextEncoder().encode(
    JSON.stringify([
      input.userId,
      input.productId,
      input.variantId ?? null,
      input.items,
    ])
  )
  const digest = await crypto.subtle.digest('SHA-256', bytes)
  const hash = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, '0')
  ).join('')
  const storageKey = `lmm:store-stock-import:v1:${hash}`
  const stored = window.sessionStorage.getItem(storageKey)
  const requestKey =
    stored === null ? storeRequestKey() : z.string().uuid().parse(stored)
  window.sessionStorage.setItem(storageKey, requestKey)
  return { storageKey, requestKey }
}

export function confirmStockImport(
  intent: Awaited<ReturnType<typeof prepareStockImport>>
) {
  if (window.sessionStorage.getItem(intent.storageKey) === intent.requestKey) {
    window.sessionStorage.removeItem(intent.storageKey)
  }
}
