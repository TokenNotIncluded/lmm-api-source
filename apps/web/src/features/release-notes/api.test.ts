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
import assert from 'node:assert/strict'
import { afterEach, describe, test } from 'node:test'

import { api } from '@/lib/api'

import {
  getLatestUnreadReleaseNote,
  listReleaseNotes,
  markReleaseNoteRead,
  publishReleaseNote,
} from './api'

const originalGet = api.get
const originalPost = api.post

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
})

describe('release note API', () => {
  test('uses the authenticated read and acknowledgement endpoints', async () => {
    const gets: string[] = []
    const posts: string[] = []
    api.get = (async (url) => {
      gets.push(url)
      return { data: { success: true, data: null } }
    }) as typeof api.get
    api.post = (async (url) => {
      posts.push(url)
      return { data: { success: true, data: null } }
    }) as typeof api.post

    assert.equal(await getLatestUnreadReleaseNote(), null)
    await markReleaseNoteRead(17)

    assert.deepEqual(gets, ['/api/release-notes/latest'])
    assert.deepEqual(posts, ['/api/release-notes/17/read'])
  })

  test('publishes a required version and changelog through the admin endpoint', async () => {
    const requests: Array<{ url: string; data: unknown }> = []
    const note = {
      id: 9,
      version: 'v1.2.3',
      revision: 1,
      content: '- Added release notes',
      published_at: 123,
      published_by: 1,
    }
    api.get = (async () => ({
      data: { success: true, data: [note] },
    })) as typeof api.get
    api.post = (async (url, data) => {
      requests.push({ url, data })
      return { data: { success: true, data: note } }
    }) as typeof api.post

    assert.deepEqual(await listReleaseNotes(), [note])
    assert.deepEqual(
      await publishReleaseNote({
        version: 'v1.2.3',
        content: '- Added release notes',
      }),
      note
    )
    assert.deepEqual(requests, [
      {
        url: '/api/release-notes/admin',
        data: { version: 'v1.2.3', content: '- Added release notes' },
      },
    ])
  })
})
