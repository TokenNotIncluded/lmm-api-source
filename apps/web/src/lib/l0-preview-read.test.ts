/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { l0PreviewRead } from './l0-preview-read'

test('preview account data is explicit and unknown endpoints fail closed', () => {
  assert.partialDeepStrictEqual(
    l0PreviewRead('/api/user/self', new URLSearchParams()),
    { id: -1, role: 1, developer_access_granted: false }
  )
  assert.equal(l0PreviewRead('/api/users', new URLSearchParams()), undefined)
  assert.partialDeepStrictEqual(
    l0PreviewRead('/api/todos', new URLSearchParams('category=all&p=2')),
    { items: [], page: 2, total: 0 }
  )
  assert.partialDeepStrictEqual(
    l0PreviewRead('/api/assistant/status', new URLSearchParams()),
    { is_admin: false, is_root: false }
  )
})
