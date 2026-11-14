/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'

import { isRedirect } from '@tanstack/react-router'
import { AxiosError, AxiosHeaders } from 'axios'

import {
  bootstrapAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { Route } from '@/routes/_authenticated/route'
import { useAuthStore } from '@/stores/auth-store'

import { installSupportedAuthBrowser } from './supported-auth-browser'

// Each caller is its own file/process in scripts/test-serial.mjs. In particular,
// ERR_NETWORK keeps the production uncertainty latch; never reset it for tests.
export async function assertRetryableColdProfile(status: 0 | 500 | 503) {
  installSupportedAuthBrowser()
  assert.ok([0, 500, 503].includes(status))
  useAuthStore.getState().auth.reset('idle')
  setDevelopmentAuthRefreshAdapter(async (config) => {
    throw new AxiosError(
      'Authentication transport unavailable',
      status === 0 ? AxiosError.ERR_NETWORK : AxiosError.ERR_BAD_RESPONSE,
      config,
      undefined,
      status === 0
        ? undefined
        : {
            config,
            data: { success: false },
            headers: new AxiosHeaders(),
            status,
            statusText: 'Unavailable',
          }
    )
  })
  assert.equal((await bootstrapAuthentication()).kind, 'transient_error')
  assert.equal(useAuthStore.getState().auth.bootstrapState, 'idle')

  const error = await runBeforeLoad('/profile')
  assert.ok(error instanceof Error, `status ${status} must offer retry`)
  assert.equal(isRedirect(error), false)
  assert.equal(useAuthStore.getState().auth.user, null)
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
