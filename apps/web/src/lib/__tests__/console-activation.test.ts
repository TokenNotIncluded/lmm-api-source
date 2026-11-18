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

import type { AuthUser } from '@/stores/auth-store'

import {
  getAuthenticatedLandingRoute,
  getOnboardingState,
  isConsoleActivated,
} from '../console-activation'

const user = (overrides: Partial<AuthUser> = {}): AuthUser => ({
  id: 7,
  username: 'member',
  role: 1,
  ...overrides,
})

describe('signed-in console access', () => {
  test('opens the console for a regular account without an access grant', () => {
    for (const account of [
      user(),
      user(JSON.parse(JSON.stringify({ developer_access_granted: false }))),
    ]) {
      assert.equal(isConsoleActivated(account), true)
      assert.equal(getAuthenticatedLandingRoute(account), '/dashboard')
    }
    assert.equal(isConsoleActivated(null), false)
    assert.equal(getAuthenticatedLandingRoute(null), '/sign-in')
  })

  test('keeps setup guidance separate from access', () => {
    const account = user(
      JSON.parse(
        JSON.stringify({
          developer_access_granted: false,
          onboarding: {
            activation_complete: false,
            credential_complete: false,
            first_request_complete: false,
            stage: 'activate',
          },
        })
      )
    )
    assert.equal(getOnboardingState(account).stage, 'credential')
    assert.equal(getAuthenticatedLandingRoute(account), '/dashboard')
  })

  test('preserves sidebar preferences and administrator route boundaries', () => {
    const withRoute = (route: string, role = 1) =>
      user({
        role,
        sidebar_modules: JSON.stringify({
          modules: {},
          preferences: { default_route: route },
        }),
      })
    assert.equal(
      getAuthenticatedLandingRoute(withRoute('/profile'), {}),
      '/profile'
    )
    assert.equal(
      getAuthenticatedLandingRoute(withRoute('/users'), {}),
      '/dashboard'
    )
    assert.equal(
      getAuthenticatedLandingRoute(withRoute('/users', 10), {}),
      '/users'
    )
    assert.equal(
      getAuthenticatedLandingRoute(withRoute('/remote-control'), {}),
      '/dashboard'
    )
    assert.equal(
      getAuthenticatedLandingRoute(withRoute('/dashboard/overview'), {
        console: { enabled: true, detail: false },
      }),
      '/dashboard'
    )
  })
})
