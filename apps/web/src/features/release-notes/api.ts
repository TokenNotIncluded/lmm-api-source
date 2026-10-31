/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import axios from 'axios'
import { z } from 'zod'

import { api, type ApiRequestConfig } from '@/lib/api'
import { withAuthenticatedCookieWrite } from '@/lib/auth-session'
import { ROLE } from '@/lib/roles'
import { useAuthStore, type AuthUser } from '@/stores/auth-store'

import type { ReleaseNote } from './types'

export type ReleaseNoteScope = {
  userId: number
  sessionId: string
  role: number
}

export function canManageReleaseNotes(user: AuthUser | null | undefined) {
  return user?.role === ROLE.ADMIN || user?.role === ROLE.SUPER_ADMIN
}

export function isCurrentReleaseNoteScope(scope: ReleaseNoteScope): boolean {
  const auth = useAuthStore.getState().auth
  return (
    auth.user?.id === scope.userId &&
    auth.session?.sid === scope.sessionId &&
    auth.user?.role === scope.role
  )
}

function assertScope(scope: ReleaseNoteScope, admin: boolean) {
  if (!isCurrentReleaseNoteScope(scope)) {
    throw new axios.CanceledError('Release note identity changed')
  }
  if (admin && !canManageReleaseNotes(useAuthStore.getState().auth.user)) {
    throw new Error('This operation is not available to this account.')
  }
}

export const RELEASE_NOTE_MAX_BYTES = 32768
export const releaseNoteInputSchema = z
  .object({
    version: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._+-]{0,63}$/),
    content: z
      .string()
      .refine(
        (value) =>
          value.trim().length > 0 &&
          !value.includes('\0') &&
          new TextEncoder().encode(value).byteLength <= RELEASE_NOTE_MAX_BYTES
      ),
  })
  .strict()

const idSchema = z.number().int().safe().positive()
const releaseNoteSchema = releaseNoteInputSchema.extend({
  id: idSchema,
  revision: z.number().int().min(1).max(10000),
  published_at: z.number().int().min(1).max(8640000000000),
  published_by: idSchema,
})

async function request<T>(
  scope: ReleaseNoteScope,
  method: 'get' | 'post',
  path: string,
  schema: z.ZodType<T>,
  options: { admin?: boolean; data?: unknown; signal?: AbortSignal } = {}
): Promise<T> {
  const admin = options.admin === true
  assertScope(scope, admin)
  const dispatch = async (config: ApiRequestConfig = {}) => {
    assertScope(scope, admin)
    const result = await api.request<unknown>({
      ...config,
      method,
      url: path,
      data: options.data,
      authScope: scope,
      signal: options.signal,
      skipBusinessError: true,
      skipErrorHandler: true,
    })
    assertScope(scope, admin)
    return z
      .object({ success: z.literal(true), data: schema })
      .parse(result.data).data
  }
  // Prepare authentication once; never replay a publication or acknowledgement.
  return method === 'post'
    ? withAuthenticatedCookieWrite(dispatch, false)
    : dispatch()
}

export function getLatestUnreadReleaseNote(
  scope: ReleaseNoteScope,
  signal?: AbortSignal
): Promise<ReleaseNote | null> {
  return request(
    scope,
    'get',
    '/api/release-notes/latest',
    releaseNoteSchema.nullable(),
    { signal }
  )
}

export async function markReleaseNoteRead(
  id: number,
  scope: ReleaseNoteScope
): Promise<void> {
  await request(
    scope,
    'post',
    `/api/release-notes/${idSchema.parse(id)}/read`,
    z.null()
  )
}

export function listReleaseNotes(
  scope: ReleaseNoteScope,
  signal?: AbortSignal
): Promise<ReleaseNote[]> {
  return request(
    scope,
    'get',
    '/api/release-notes/admin?limit=50',
    z.array(releaseNoteSchema).max(50),
    { admin: true, signal }
  )
}

export async function publishReleaseNote(
  input: { version: string; content: string },
  scope: ReleaseNoteScope
): Promise<ReleaseNote> {
  const data = releaseNoteInputSchema.parse(input)
  return request(
    scope,
    'post',
    '/api/release-notes/admin',
    releaseNoteSchema.extend({
      version: z.literal(data.version),
      content: z.literal(data.content),
    }),
    { admin: true, data }
  )
}
