/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api, type ApiRequestConfig } from '@/lib/api'

const navigationValue = z.object({
  value: z.string(),
  revision: z.number().int().positive().safe(),
})

export const navigationResponseSchema = z.object({
  success: z.literal(true),
  message: z.string(),
  data: navigationValue.extend({ sidebar: navigationValue.optional() }),
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
  config?: ApiRequestConfig,
  expectedValue?: string
) {
  const response = await api.put<unknown>(
    '/api/site/navigation',
    { value, expected_value: expectedValue },
    config
  )
  return navigationResponseSchema.parse(response.data)
}

export async function saveSiteSidebar(
  value: string,
  expectedValue: string,
  config?: ApiRequestConfig
) {
  const response = await api.put<unknown>(
    '/api/site/navigation',
    { key: 'SidebarModulesAdmin', value, expected_value: expectedValue },
    config
  )
  return navigationResponseSchema.parse(response.data)
}
