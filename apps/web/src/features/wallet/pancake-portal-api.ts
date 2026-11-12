/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  currentCommerceScope,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { api } from '@/lib/api'

const portalSchema = z
  .object({
    portal_url: z.string().url(),
    expires_at: z.iso.datetime({ offset: true }),
  })
  .strict()
export type PancakePortalLink = z.infer<typeof portalSchema>

export async function createPancakePortal(
  scope: CommerceScope
): Promise<PancakePortalLink> {
  if (!currentCommerceScope(scope)) {
    throw new Error('Payment portal session changed')
  }
  const response = await api.post(
    '/api/user/payments/pancake/portal',
    undefined,
    {
      authScope: scope,
      disableDuplicate: true,
      skipAuthRefresh: true,
      skipBusinessError: true,
      skipErrorHandler: true,
      timeout: 15_000,
    }
  )
  if (!currentCommerceScope(scope)) {
    throw new Error('Payment portal session changed')
  }
  const link = z
    .object({ success: z.literal(true), data: portalSchema })
    .parse(response.data).data
  const url = new URL(link.portal_url)
  if (
    url.origin !== 'https://pancake.waffo.ai' ||
    url.username ||
    url.password ||
    url.search ||
    !/^\/consumer\/portal\/store\/[A-Za-z0-9_.-]{1,128}$/.test(url.pathname) ||
    !url.hash.startsWith('#token=') ||
    url.hash.length <= 7 ||
    Date.parse(link.expires_at) <= Date.now()
  ) {
    throw new Error('Invalid payment portal link')
  }
  return link
}
