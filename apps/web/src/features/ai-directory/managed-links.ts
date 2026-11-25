/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'

import {
  AI_DIRECTORY_CATEGORIES,
  DEFAULT_AI_DIRECTORY_LINKS,
  safeDirectoryUrl,
  type AIDirectoryLink,
} from './api'

const link = z.strictObject({
  id: z.string().regex(/^[a-zA-Z0-9_-]{1,64}$/),
  name: z.string().refine((s) => Boolean(s.trim()) && [...s].length <= 80),
  url: z
    .string()
    .refine(
      (s) =>
        !/[ \t\r\n]/.test(s) &&
        new TextEncoder().encode(s).length <= 2048 &&
        Boolean(safeDirectoryUrl(s))
    ),
  category: z.enum(AI_DIRECTORY_CATEGORIES),
  summary: z.string().refine((s) => [...s].length <= 180),
  description: z.string().refine((s) => [...s].length <= 1200),
  enabled: z.boolean(),
})
const links = z
  .array(link)
  .max(60)
  .refine((rows) => new Set(rows.map((r) => r.id)).size === rows.length)
const envelope = z.object({
  success: z.literal(true),
  data: z.object({ links: links.nullable() }),
})

function current(scope: NativeUsersScope) {
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
}

export async function readManagedDirectory(
  scope: NativeUsersScope,
  signal?: AbortSignal
) {
  current(scope)
  const { data } = await api.get<unknown>(
    '/api/ai-directory',
    nativeUsersOptions(scope, signal)
  )
  current(scope)
  const saved = envelope.parse(data).data.links
  return saved ?? links.parse(DEFAULT_AI_DIRECTORY_LINKS)
}

export async function saveManagedDirectory(
  scope: NativeUsersScope,
  before: AIDirectoryLink[],
  draft: AIDirectoryLink[]
) {
  const next = links.parse(draft)
  const snapshot = JSON.stringify(next)
  const latest = JSON.stringify(await readManagedDirectory(scope))
  if (latest === snapshot) return next
  if (latest !== JSON.stringify(links.parse(before))) {
    throw new Error('Directory changed')
  }
  current(scope)
  const { data } = await api.put<unknown>(
    '/api/ai-directory/links',
    { links: next },
    nativeUsersOptions(scope)
  )
  current(scope)
  if (JSON.stringify(envelope.parse(data).data.links) !== snapshot) {
    throw new Error('Save not confirmed')
  }
  const confirmed = await readManagedDirectory(scope)
  if (JSON.stringify(confirmed) !== snapshot) {
    throw new Error('Directory changed')
  }
  return confirmed
}
