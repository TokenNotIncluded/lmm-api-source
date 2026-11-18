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

import { isRedirect } from '@tanstack/react-router'
import { AxiosHeaders } from 'axios'

import {
  bootstrapAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { useAuthStore, type AuthUser } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import { Route } from './route'

// Normal supported browser; install after this fixture replaces its globals.
installSupportedAuthBrowser()

function authenticate(user: AuthUser) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'route-test-token',
    token_type: 'Bearer',
    access_expires_at: 1_900_000_000,
    user,
    session: {
      sid: 'route-test-session',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'route-test',
      created_at: 1,
      last_active_at: 1,
      expires_at: 1_900_000_000,
    },
  })
}

async function runBeforeLoad(pathname: string) {
  let thrown: unknown
  try {
    await Route.options.beforeLoad?.({
      location: { href: pathname, pathname },
    } as never)
  } catch (error) {
    thrown = error
  }
  return thrown
}

afterEach(() => useAuthStore.getState().auth.reset('complete'))

describe('authenticated route access', () => {
  test('does not redirect a pending session check to sign in', async () => {
    useAuthStore.getState().auth.reset('checking')
    const error = await runBeforeLoad('/profile')
    assert.ok(error instanceof Error)
    assert.equal(isRedirect(error), false)
  })

  test('still sends an authoritatively unauthenticated profile request to sign in', async () => {
    useAuthStore.getState().auth.reset('idle')
    setDevelopmentAuthRefreshAdapter(async (config) => ({
      config,
      data: { success: false, code: 'AUTH_UNAUTHORIZED' },
      headers: new AxiosHeaders(),
      status: 401,
      statusText: 'Unauthorized',
    }))
    assert.equal((await bootstrapAuthentication()).kind, 'anonymous')
    assert.equal(useAuthStore.getState().auth.bootstrapState, 'complete')
    const redirect = await runBeforeLoad('/profile')
    assert.ok(isRedirect(redirect))
    assert.equal(redirect.options.to, '/sign-in')
  })

  test('opens ordinary console routes for a signed-in user without a developer grant', async () => {
    authenticate(
      JSON.parse(
        JSON.stringify({
          id: 9,
          username: 'member',
          role: 1,
          developer_access_granted: false,
        })
      )
    )
    for (const path of [
      '/dashboard',
      '/keys',
      '/guide',
      '/todos',
      '/wallet',
      '/profile',
    ]) {
      assert.equal(await runBeforeLoad(path), undefined)
    }
  })

  test('requires a valid session for console routes', async () => {
    useAuthStore.getState().auth.reset('complete')
    for (const path of ['/dashboard', '/keys', '/guide']) {
      const result = await runBeforeLoad(path)
      assert.ok(isRedirect(result))
      assert.equal(result.options.to, '/sign-in')
      assert.deepEqual(result.options.search, { redirect: path })
    }
  })
})
