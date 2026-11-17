/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  defaultParseSearch,
  defaultStringifySearch,
} from '@tanstack/react-router'
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
    team_id: z.preprocess(
      (value) =>
        typeof value === 'number' && Number.isSafeInteger(value)
          ? String(value)
          : value,
      z.string().default(DEFAULT_NATIVE_USERS_QUERY.team_id)
    ),
    role: z
      .enum(['', 'user', 'admin', 'superadmin'])
      .default(DEFAULT_NATIVE_USERS_QUERY.role),
    status: z
      .enum(['', 'active', 'disabled'])
      .default(DEFAULT_NATIVE_USERS_QUERY.status),
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

export function nativeUsersSearch(query: NativeUsersQuery): string {
  const validated = nativeUsersQuery(query)
  // TanStack quotes numeric-looking strings; an ordinary URLSearchParams round
  // trip would coerce a long user ID or group name into a rounded number.
  const changed = Object.fromEntries(
    Object.entries(validated).filter(
      ([key, value]) =>
        value !== DEFAULT_NATIVE_USERS_QUERY[key as keyof NativeUsersQuery]
    )
  )
  return defaultStringifySearch(changed)
}
