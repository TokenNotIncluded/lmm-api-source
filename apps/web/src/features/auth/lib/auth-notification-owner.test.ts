/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { CanceledError } from 'axios'
import type { TFunction } from 'i18next'
import { toast } from 'sonner'

const errors: string[] = []
const infos: string[] = []
const originalInfo = toast.info
toast.info = (value) => {
  infos.push(String(value))
  return 'test-notification'
}
const originalError = toast.error
toast.error = (value) => {
  errors.push(String(value))
  return 'test-notification'
}
after(() => {
  toast.error = originalError
  toast.info = originalInfo
})
const {
  reportAuthError,
  reportOwnedAuthError,
  reportPasskeyCancellation,
  completeAuthResponse,
} = await import('./auth-completion')
const {
  AuthCookieBlockedError,
  getAuthCookieBlockedState,
  withAuthCookieLock,
} = await import('@/lib/auth-cookie')
const translate = ((value: string) => `translated:${value}`) as TFunction

test('unpublished Cookie errors retain their localized fallback notification', () => {
  assert.equal(getAuthCookieBlockedState(), null)
  const error = new AuthCookieBlockedError('uncertain')
  reportAuthError(error, translate, 'fallback')
  assert.deepEqual(errors, [`translated:${error.message}`])
  errors.length = 0
})

test('canceled auth errors stay silent; unpublished typed errors retain their single owner', () => {
  reportAuthError(new CanceledError('superseded'), translate, 'fallback')
  assert.equal(reportOwnedAuthError(new CanceledError(), translate), true)
  assert.deepEqual(errors, [])
  const error = new AuthCookieBlockedError('uncertain')
  assert.equal(reportOwnedAuthError(error, translate), true)
  assert.deepEqual(errors, [`translated:${error.message}`])
  errors.length = 0
})

test('null and NotAllowed Passkey results each issue one informational cancellation', () => {
  assert.equal(reportPasskeyCancellation(null, translate), true)
  assert.equal(
    reportPasskeyCancellation(
      new DOMException('cancel', 'NotAllowedError'),
      translate
    ),
    true
  )
  assert.equal(
    reportPasskeyCancellation(new Error('ordinary failure'), translate),
    false
  )
  assert.deepEqual(infos, [
    'translated:Passkey login was cancelled',
    'translated:Passkey login was cancelled',
  ])
  infos.length = 0
  assert.deepEqual(errors, [])
})

test('safe server OAuth denial remains specific; legal refusal uses its localized dictionary key', () => {
  reportAuthError(
    {
      success: false,
      code: 'OAUTH_ACCESS_DENIED',
      message: 'Ask the workspace owner to enable this sign-in.',
    },
    translate,
    'fallback'
  )
  reportAuthError(
    {
      success: false,
      code: 'AUTH_LEGAL_ACCEPTANCE_REQUIRED',
      message: 'untrusted generic detail',
    },
    translate,
    'fallback'
  )
  reportAuthError(
    { success: false, code: 'OAUTH_ACCESS_DENIED', message: '<b>unsafe</b>' },
    translate,
    'fallback'
  )
  assert.deepEqual(errors, [
    'Ask the workspace owner to enable this sign-in.',
    'translated:Please agree to the legal terms first',
    'translated:OAuth failed',
  ])
  errors.length = 0
})

test('the published recovery alert owns the warning without an obscuring toast', async () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: {},
  })
  let writes = 0
  try {
    await assert.rejects(
      withAuthCookieLock(async () => {
        writes++
      }),
      (error: unknown) => {
        assert.ok(error instanceof AuthCookieBlockedError)
        assert.equal(getAuthCookieBlockedState(), 'coordination_unavailable')
        reportAuthError(error, translate, 'fallback')
        return true
      }
    )
    assert.equal(writes, 0)
    reportAuthError(new Error('late generic error'), translate, 'fallback')
    reportAuthError(
      {
        success: false,
        code: 'OAUTH_ACCESS_DENIED',
        message: 'A custom denial must not compete with Root.',
      },
      translate,
      'fallback'
    )
    reportAuthError(
      { success: false, code: 'AUTH_LEGAL_ACCEPTANCE_REQUIRED' },
      translate,
      'fallback'
    )
    assert.equal(
      reportOwnedAuthError(new Error('late generic error'), translate),
      true
    )
    reportPasskeyCancellation(null, translate)
    reportPasskeyCancellation(
      new DOMException('cancel', 'NotAllowedError'),
      translate
    )
    let applied = false
    await completeAuthResponse(
      {
        success: true,
        data: { require_2fa: true, flow_token: 'blocked-synthetic-flow' },
      },
      {
        alreadyApplied: false,
        onBundle: () => {
          applied = true
        },
        onMFA: () => {
          applied = true
        },
      }
    )
    assert.equal(applied, false)
    assert.deepEqual(infos, [])
    assert.deepEqual(errors, [])
  } finally {
    if (original) Object.defineProperty(globalThis, 'navigator', original)
    else Reflect.deleteProperty(globalThis, 'navigator')
  }
})
