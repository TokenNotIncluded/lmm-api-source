/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

import { login, logout } from '@/features/auth/api'
import { api } from '@/lib/api'
import {
  applyAuthBundle,
  clearAuthentication,
  prepareLoginIntentVersion,
} from '@/lib/auth-session'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import {
  beginFederatedLogin,
  finishFederatedLogin,
  parseFederatedChallenge,
} from './federated-api'
import {
  saveFederatedCeremony,
  type FederatedProvider,
} from './lib/federated-oauth'

const oldWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
const dom = new Window({ url: 'https://web.example.test/sign-in' })
Object.defineProperty(globalThis, 'window', { configurable: true, value: dom })
function requestURL(config: { url?: string }): string {
  assert.ok(config.url)
  return config.url
}
const adapter = api.defaults.adapter
// Normal supported browser; install after this fixture replaces its globals.
installSupportedAuthBrowser()

const originalFetch = globalThis.fetch
const provider: FederatedProvider = {
  id: '71',
  kind: 'wechat',
  slug: 'synthetic-wechat',
  name: 'WeChat',
  icon: '',
  revision: '1',
  client_id: '',
  authorization_endpoint: '',
  scopes: '',
  qr_code_url: '',
  native_federated: true,
}
const flow = {
  state: `lmmfe_${'b'.repeat(64)}`,
  expires_at: 2000000000,
  authorization_url: '',
}
function bundle(user = 701, sid = 'synthetic-native-sid'): AuthBundle {
  return {
    access_token: `synthetic-access-${user}`,
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: user, username: 'synthetic', role: 1 },
    session: {
      sid,
      current: true,
      login_method: 'federated',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  }
}
beforeEach(() => {
  globalThis.fetch = (() => {
    throw new Error('Unexpected fetch in unit fixture')
  }) as typeof fetch
  clearAuthentication(false, 'idle')
  dom.sessionStorage.clear()
  dom.localStorage.clear()
})
afterEach(() => {
  globalThis.fetch = originalFetch
  api.defaults.adapter = adapter
  clearAuthentication(false, 'idle')
})
after(() => {
  dom.close()
  if (oldWindow) Object.defineProperty(globalThis, 'window', oldWindow)
  else Reflect.deleteProperty(globalThis, 'window')
})
test('native begin freezes affiliate/legal/state and finish POST applies the real AuthBundle schema once', async () => {
  dom.localStorage.setItem('aff', 'synthetic-inviter')
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    requests.push(requestURL(config))
    assert.equal(config.method, 'post')
    assert.equal(config.authMode, 'anonymous')
    assert.equal(config.skipAuthRefresh, true)
    if (requestURL(config).endsWith('/begin')) {
      assert.deepEqual(JSON.parse(config.data), {
        accepted_legal: true,
        affiliate: 'synthetic-inviter',
      })
    } else {
      assert.deepEqual(JSON.parse(config.data), {
        state: flow.state,
        code: 'synthetic-code',
        iss: 'synthetic-issuer',
      })
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: requestURL(config).endsWith('/begin') ? flow : bundle(),
      },
    }
  }
  assert.deepEqual(await beginFederatedLogin(provider, true), flow)
  await finishFederatedLogin(provider.slug, {
    state: flow.state,
    code: 'synthetic-code',
    iss: 'synthetic-issuer',
  })
  assert.equal(
    useAuthStore.getState().auth.session?.sid,
    'synthetic-native-sid'
  )
  await assert.rejects(
    finishFederatedLogin(provider.slug, {
      state: flow.state,
      code: 'synthetic-code',
    })
  )
  assert.deepEqual(requests, [
    '/api/oauth/federated/71/begin',
    '/api/oauth/federated/71/finish',
  ])
})
test('unknown state, wrong slug, bind intent and expired state never dispatch a login finish', async () => {
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('Must not dispatch')
  }
  await assert.rejects(
    finishFederatedLogin(provider.slug, {
      state: flow.state,
      code: 'synthetic-code',
    })
  )
  saveFederatedCeremony(dom.sessionStorage, provider, flow, 'login')
  await assert.rejects(
    finishFederatedLogin('other-slug', {
      state: flow.state,
      code: 'synthetic-code',
    })
  )
  saveFederatedCeremony(dom.sessionStorage, provider, flow, 'bind')
  await assert.rejects(
    finishFederatedLogin(provider.slug, {
      state: flow.state,
      code: 'synthetic-code',
    })
  )
  saveFederatedCeremony(
    dom.sessionStorage,
    provider,
    { ...flow, expires_at: 1 },
    'login'
  )
  await assert.rejects(
    finishFederatedLogin(provider.slug, {
      state: flow.state,
      code: 'synthetic-code',
    })
  )
  assert.equal(calls, 0)
})
test('native TOTP stores only the MFA token, while passkey remains a separate challenge', async () => {
  saveFederatedCeremony(
    dom.sessionStorage,
    provider,
    flow,
    'login',
    (await prepareLoginIntentVersion()).version
  )
  const challenge = {
    require_2fa: true,
    flow_token: `lmmf_${'c'.repeat(64)}`,
    expires_at: 2000000000,
  }
  api.defaults.adapter = async (config) => ({
    config,
    status: 200,
    statusText: 'OK',
    headers: {},
    data: { success: true, data: challenge },
  })
  await finishFederatedLogin(provider.slug, {
    state: flow.state,
    code: 'synthetic-code',
  })
  assert.equal(
    useAuthStore.getState().auth.pending2FAFlowToken,
    challenge.flow_token
  )
  assert.equal(useAuthStore.getState().auth.user, null)
  assert.ok(
    parseFederatedChallenge({
      require_passkey: true,
      flow_token: `lmmp_${'d'.repeat(64)}`,
      expires_at: 2000000000,
      options: { publicKey: {} },
    })
  )
  assert.throws(() => parseFederatedChallenge({ ...challenge, expires_at: 1 }))
})
test('a delayed native finish cannot replace a later signed-in user and failed finish is never replayed', async () => {
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    assert.equal(request.headers.get('X-Auth-Session'), 'synthetic-native-sid')
    assert.equal(
      request.headers.get('Authorization'),
      'Bearer synthetic-access-701'
    )
    if (request.method === 'DELETE') {
      // Bun 1.4 Request does not reflect credentials; Chromium transport probe owns that assertion.
      return Response.json({
        success: true,
        data: { revoked_sid: 'synthetic-native-sid', current: true },
      })
    }
    return Response.json(
      { success: false, code: 'AUTH_SESSION_MISMATCH' },
      { status: 409 }
    )
  }) as typeof fetch
  saveFederatedCeremony(
    dom.sessionStorage,
    provider,
    flow,
    'login',
    (await prepareLoginIntentVersion()).version
  )
  let release!: () => void
  let entered!: () => void
  const waiting = new Promise<void>((resolve) => {
    entered = resolve
  })
  const gate = new Promise<void>((resolve) => {
    release = resolve
  })
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    entered()
    await gate
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: bundle() },
    }
  }
  const pending = finishFederatedLogin(provider.slug, {
    state: flow.state,
    code: 'synthetic-code',
  })
  await waiting
  useAuthStore.getState().auth.setBundle(bundle(702, 'later-sid'))
  release()
  await assert.rejects(pending, /superseded/)
  assert.equal(useAuthStore.getState().auth.user?.id, 702)
  await assert.rejects(
    finishFederatedLogin(provider.slug, {
      state: flow.state,
      code: 'synthetic-code',
    })
  )
  assert.equal(calls, 1)
})

test('a redirect ceremony begun before confirmed logout cannot create a session from a fresh callback guard', async () => {
  let finishes = 0
  api.defaults.adapter = async (config) => {
    if ((config.url ?? '').endsWith('/finish')) finishes++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: (config.url ?? '').endsWith('/begin')
          ? flow
          : (config.url ?? '').endsWith('/finish')
            ? bundle()
            : undefined,
      },
    }
  }
  await beginFederatedLogin(provider, true)
  await logout()
  await assert.rejects(
    finishFederatedLogin(provider.slug, {
      state: flow.state,
      code: 'synthetic-code',
    })
  )
  assert.equal(finishes, 0)
  assert.equal(useAuthStore.getState().auth.user, null)
})

// Core-contract unit model, not real browser/server acceptance of C.
test('C: Cookie-independent logout revokes A while dispatched superseded finish compensates B', async () => {
  const { AxiosError } = await import('axios')
  const saved = new Map(
    ['navigator', 'isSecureContext', 'fetch'].map((key) => [
      key,
      Object.getOwnPropertyDescriptor(globalThis, key),
    ])
  )
  let tail = Promise.resolve()
  const locks = {
    async request<T>(_name: string, _options: unknown, run: () => Promise<T>) {
      const before = tail
      let release!: () => void
      tail = new Promise<void>((resolve) => {
        release = resolve
      })
      await before
      try {
        return await run()
      } finally {
        release()
      }
    },
  }
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: { locks },
  })
  Object.defineProperty(globalThis, 'isSecureContext', {
    configurable: true,
    value: true,
  })
  const a = bundle(710, 'a'.repeat(64)),
    b = bundle(711, 'b'.repeat(64))
  const live = new Map([
    [a.session.sid, true],
    [b.session.sid, false],
  ])
  let cookie: string | undefined = a.session.sid
  const revoked: string[] = []
  Object.defineProperty(globalThis, 'fetch', {
    configurable: true,
    value: async (input: RequestInfo | URL, init?: RequestInit) => {
      const request = new Request(input, init),
        sid = request.headers.get('X-Auth-Session')
      assert.ok(sid)
      const expected = sid === a.session.sid ? a : b
      assert.equal(
        request.headers.get('Authorization'),
        `Bearer ${expected.access_token}`
      )
      if (request.method === 'DELETE') {
        // Cookie omission / ignored Set-Cookie are asserted in the real Chromium transport probe.
        assert.equal(new URL(request.url).pathname, `/api/user/sessions/${sid}`)
        live.set(sid, false)
        revoked.push(sid)
        return Response.json({
          success: true,
          data: { revoked_sid: sid, current: true },
        })
      }
      assert.equal(request.method, 'POST')
      assert.equal(new URL(request.url).pathname, '/api/user/auth/logout')
      if (sid === a.session.sid) {
        // Original A uses Cookie-omit purpose-only logout and exact receipt.
        live.set(sid, false)
        revoked.push(sid)
        return Response.json({ success: true, data: { revoked_sid: sid } })
      }
      if (cookie && cookie !== sid) {
        return Response.json(
          { success: false, code: 'AUTH_SESSION_MISMATCH' },
          { status: 409 }
        )
      }
      live.set(sid, false)
      cookie = undefined
      return Response.json({ success: true })
    },
  })
  let enter!: () => void, release!: () => void
  const entered = new Promise<void>((resolve) => {
      enter = resolve
    }),
    gate = new Promise<void>((resolve) => {
      release = resolve
    })
  applyAuthBundle(a, false)
  api.defaults.adapter = async (config) => {
    if (requestURL(config).endsWith('/finish')) {
      enter()
      await gate
      live.set(b.session.sid, true)
      cookie = b.session.sid
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: { success: true, data: b },
      }
    }
    if (requestURL(config).endsWith('/auth/logout')) {
      const sid = config.headers.get('X-Auth-Session') as string
      if (cookie && cookie !== sid) {
        throw new AxiosError(
          'SID mismatch',
          'ERR_BAD_REQUEST',
          config,
          undefined,
          {
            config,
            status: 409,
            statusText: 'Conflict',
            headers: {},
            data: { success: false, code: 'AUTH_SESSION_MISMATCH' },
          }
        )
      }
      assert.equal(
        config.headers.get('Authorization'),
        `Bearer ${a.access_token}`
      )
      live.set(sid, false)
      cookie = undefined
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: { success: true },
      }
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: flow },
    }
  }
  try {
    await beginFederatedLogin(provider, true)
    const finishing = finishFederatedLogin(provider.slug, {
      state: flow.state,
      code: 'synthetic',
    }).then(
      (value) => ({ value }),
      (error) => ({ error })
    )
    await entered
    const exiting = logout().then(
      (value) => ({ value }),
      (error) => ({ error })
    )
    release()
    assert.ok('error' in (await finishing))
    const outcome = await exiting
    assert.ok(
      'value' in outcome,
      `queued logout must not end in original Cookie-B/expected-A 409: ${'error' in outcome ? String(outcome.error) : ''}`
    )
    assert.equal(outcome.value.success, true)
    assert.deepEqual(revoked, [a.session.sid, b.session.sid])
    assert.deepEqual(
      [...live.values()],
      [false, false],
      'cancellation alone leaves both families alive'
    )
    assert.equal(useAuthStore.getState().auth.user, null)
    assert.equal(cookie, undefined)
  } finally {
    release()
    await tail
    for (const [key, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor)
      else Reflect.deleteProperty(globalThis, key)
    }
  }
})

test('D: password choice invalidates persisted OAuth before a fresh callback dispatch', async () => {
  let finishes = 0
  api.defaults.adapter = async (config) => {
    let data: unknown = flow
    if (requestURL(config).startsWith('/api/user/login?')) {
      data = bundle(712, 'chosen-password-session')
    }
    if (requestURL(config).endsWith('/finish')) {
      finishes++
      data = bundle()
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data },
    }
  }
  await beginFederatedLogin(provider, true)
  await login({ username: 'chosen', password: 'synthetic' })
  await assert.rejects(
    finishFederatedLogin(provider.slug, {
      state: flow.state,
      code: 'old-code',
    }),
    /superseded/
  )
  assert.equal(finishes, 0)
  assert.equal(useAuthStore.getState().auth.user?.id, 712)
})
