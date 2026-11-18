/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { AuthUser } from '@/stores/auth-store'

import { canManageApiKeys } from './can-manage-api-keys'

test('every signed-in account can manage its own API keys', () => {
  const account: AuthUser = { id: 1, username: 'member', role: 1 }
  assert.equal(canManageApiKeys(account), true)
  assert.equal(
    canManageApiKeys(
      JSON.parse(
        JSON.stringify({
          ...account,
          developer_access_granted: false,
          permissions: { api_key_management: false },
        })
      )
    ),
    true
  )
  assert.equal(canManageApiKeys(null), false)
  assert.equal(canManageApiKeys(undefined), false)
})
