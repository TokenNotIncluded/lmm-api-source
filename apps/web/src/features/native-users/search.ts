/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { defaultParseSearch } from '@tanstack/react-router'
import { z } from 'zod'

import {
  DEFAULT_NATIVE_USERS_QUERY,
  NATIVE_USERS_CAPABILITIES,
  nativeUsersQuery,
  type NativeUsersQuery,
} from './contract'

export const nativeUsersSearchSchema = z
  .object({
    keyword: z.string().default(DEFAULT_NATIVE_USERS_QUERY.keyword),
    group: z.string().default(DEFAULT_NATIVE_USERS_QUERY.group),
    role: z
      .enum(['', 'user', 'admin', 'superadmin'])
      .default(DEFAULT_NATIVE_USERS_QUERY.role),
    status: z
      .enum(['', 'active', 'disabled'])
      .default(DEFAULT_NATIVE_USERS_QUERY.status),
    // TanStack decodes an unquoted URL level as a JSON number.
    service_level: z.preprocess(
      (value) => (typeof value === 'number' ? String(value) : value),
      z
        .enum(['', '0', '1', '2', '3', '4'])
        .default(DEFAULT_NATIVE_USERS_QUERY.service_level)
    ),
    page: z.number().default(DEFAULT_NATIVE_USERS_QUERY.page),
    page_size: z.number().default(DEFAULT_NATIVE_USERS_QUERY.page_size),
    sort_by: z
      .enum(NATIVE_USERS_CAPABILITIES.sorts)
      .default(DEFAULT_NATIVE_USERS_QUERY.sort_by),
    sort_order: z
      .enum(['asc', 'desc'])
      .default(DEFAULT_NATIVE_USERS_QUERY.sort_order),
  })
  .strict()
  .transform((query, context) => {
    try {
      return nativeUsersQuery(query)
    } catch {
      context.addIssue({ code: 'custom', message: 'invalid_query' })
      return z.NEVER
    }
  })

export function nativeQueryFromSearch(
  search: string
): NativeUsersQuery | undefined {
  const seen = new Set<string>()
  for (const key of new URLSearchParams(search).keys()) {
    if (seen.has(key)) return undefined
    seen.add(key)
  }
  const parsed = nativeUsersSearchSchema.safeParse(defaultParseSearch(search))
  return parsed.success ? parsed.data : undefined
}
