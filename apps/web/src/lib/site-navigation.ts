/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api, type ApiRequestConfig } from '@/lib/api'

export const navigationResponseSchema = z.object({
  success: z.literal(true),
  message: z.string(),
  data: z.object({
    value: z.string(),
    revision: z.number().int().positive().safe(),
  }),
})

export const siteNavigationQueryKey = ['site-navigation']

export async function getSiteNavigation(config?: ApiRequestConfig) {
  const response = await api.get<unknown>('/api/site/navigation', {
    timeout: 10_000,
    skipAuthRefresh: true,
    ...config,
  })
  return navigationResponseSchema.parse(response.data).data
}

export async function saveSiteNavigation(
  value: string,
  config?: ApiRequestConfig
) {
  const response = await api.put<unknown>(
    '/api/site/navigation',
    { value },
    config
  )
  return navigationResponseSchema.parse(response.data)
}
