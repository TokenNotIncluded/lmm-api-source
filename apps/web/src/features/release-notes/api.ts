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
import { api } from '@/lib/api'

import type { ReleaseNote, ReleaseNoteResponse } from './types'

export async function getLatestUnreadReleaseNote(): Promise<ReleaseNote | null> {
  const response = await api.get('/api/release-notes/latest', {
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  const result = response.data as ReleaseNoteResponse<ReleaseNote | null>
  if (!result?.success) {
    throw new Error(result?.message || 'Failed to load release notes')
  }
  return result.data ?? null
}

export async function markReleaseNoteRead(id: number): Promise<void> {
  const response = await api.post(`/api/release-notes/${id}/read`)
  const result = response.data as ReleaseNoteResponse<null>
  if (!result?.success) {
    throw new Error(result?.message || 'Failed to acknowledge release note')
  }
}

export async function listReleaseNotes(): Promise<ReleaseNote[]> {
  const response = await api.get('/api/release-notes/admin?limit=50')
  const result = response.data as ReleaseNoteResponse<ReleaseNote[] | null>
  if (!result?.success) {
    throw new Error(result?.message || 'Failed to load release history')
  }
  return result.data ?? []
}

export async function publishReleaseNote(input: {
  version: string
  content: string
}): Promise<ReleaseNote> {
  const response = await api.post('/api/release-notes/admin', input)
  const result = response.data as ReleaseNoteResponse<ReleaseNote>
  if (!result?.success || !result.data) {
    throw new Error(result?.message || 'Failed to publish release note')
  }
  return result.data
}
