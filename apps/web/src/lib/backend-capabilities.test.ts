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
import { describe, test } from 'node:test'

import {
  getCapabilitySafeStatus,
  getBackendCapabilities,
  normalizeBackendCapabilities,
} from './backend-capabilities'

describe('backend capability negotiation', () => {
  test('maps a legacy status response without capabilities to conservative defaults', () => {
    assert.deepEqual(getBackendCapabilities({ version: 'legacy-go' }), {
      bounty_notifications: false,
      bounty_challenge_cancel: false,
      bounty_public_read: false,
      self_oauth_unbind: false,
      responses_websocket: false,
    })
  })

  test('only enables capabilities explicitly advertised by the backend', () => {
    assert.deepEqual(
      getBackendCapabilities({
        backend_capabilities: {
          bounty_notifications: true,
          bounty_challenge_cancel: true,
          bounty_public_read: true,
          self_oauth_unbind: true,
          responses_websocket: true,
        },
      }),
      {
        bounty_notifications: true,
        bounty_challenge_cancel: true,
        bounty_public_read: true,
        self_oauth_unbind: true,
        responses_websocket: true,
      }
    )
  })

  test('does not trust persisted capability data before a live status response', () => {
    const normalized = normalizeBackendCapabilities(
      {
        backend_capabilities: { bounty_notifications: true },
      },
      false
    )

    assert.equal(normalized.backend_capabilities?.bounty_notifications, false)
  })

  test('masks every cached capability until the current mount confirms live status', () => {
    const cachedStatus = {
      backend_capabilities: {
        bounty_notifications: true,
        bounty_challenge_cancel: true,
        bounty_public_read: true,
        self_oauth_unbind: true,
        responses_websocket: true,
      },
    }

    assert.deepEqual(
      getBackendCapabilities(getCapabilitySafeStatus(cachedStatus, false)),
      {
        bounty_notifications: false,
        bounty_challenge_cancel: false,
        bounty_public_read: false,
        self_oauth_unbind: false,
        responses_websocket: false,
      }
    )
    assert.deepEqual(
      getBackendCapabilities(getCapabilitySafeStatus(cachedStatus, true)),
      cachedStatus.backend_capabilities
    )
  })
})
