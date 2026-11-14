/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// Actual helper/API code with HappyDOM Storage and a scheduled Web Locks stand-in.
// No browser, Cookie transport, Core or IdP acceptance is claimed.
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

import {
  beginFederatedLogin,
  finishFederatedLogin,
} from '@/features/auth/federated-api'
import {
  saveFederatedCeremony,
  readFederatedCeremony,
  type FederatedProvider,
} from '@/features/auth/lib/federated-oauth'
import { api } from '@/lib/api'
import { AUTH_COOKIE_LOCK } from '@/lib/auth-cookie'
import {
  capturePasswordLoginGuard,
  clearAuthentication,
  invalidatePendingLogins,
  prepareLoginIntentVersion,
  refreshAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

const C = 'new-api:auth-login-intent'
const I = 'new-api:auth-login-initialization'
const dom = new Window({ url: 'https://web.example.test/sign-in' })
const globals = new Map(
  ['window', 'navigator', 'isSecureContext'].map((k) => [
    k,
    Object.getOwnPropertyDescriptor(globalThis, k),
  ])
)
let tail: Promise<unknown> = Promise.resolve()
let lockCalls = 0
let dispatches = 0
let order = 0
const names: string[] = []
const locks = {
  request: (
    name: string,
    options: { signal?: AbortSignal },
    run: () => Promise<unknown>
  ) => {
    lockCalls++
    names.push(name)
    const work = tail.then(() => {
      if (options.signal?.aborted) {
        throw new DOMException('cancelled', 'AbortError')
      }
      return run()
    })
    tail = work.catch(() => undefined)
    return work
  },
}
Object.defineProperty(globalThis, 'window', { configurable: true, value: dom })
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: { locks },
})
Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
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
function flow() {
  return {
    state: `lmmfe_${(++order).toString(16).padStart(64, '0')}`,
    expires_at: 2000000000,
    authorization_url: '',
  }
}
const originalAdapter = api.defaults.adapter
const originalFetch = globalThis.fetch
let lastFlow = flow()
function holdLock() {
  let release!: () => void
  tail = new Promise<void>((r) => {
    release = r
  })
  return release
}
const settle = () => new Promise<void>((resolve) => setImmediate(resolve))
function bundle(token = 'access-a'): AuthBundle {
  return {
    access_token: token,
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 701, username: 'synthetic', role: 1 },
    session: {
      sid: 'same-sid',
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  }
}
beforeEach(() => {
  clearAuthentication(false, 'idle')
  dom.localStorage.clear()
  dom.sessionStorage.clear()
  tail = Promise.resolve()
  lockCalls = dispatches = 0
  names.length = 0
  globalThis.fetch = (() => {
    throw new Error('No HTTP allowed in helper fixture')
  }) as typeof fetch
  api.defaults.adapter = async (config) => {
    dispatches++
    lastFlow = flow()
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: config.url?.endsWith('/begin')
        ? { success: true, data: lastFlow }
        : { success: false },
    }
  }
})
afterEach(async () => {
  await tail
  api.defaults.adapter = originalAdapter
  globalThis.fetch = originalFetch
  clearAuthentication(false, 'idle')
})
after(() => {
  dom.close()
  for (const [key, value] of globals) {
    if (value) Object.defineProperty(globalThis, key, value)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('cold actual prepare must not invalidate an outer click guard or write cancellation C', async () => {
  const outer = capturePasswordLoginGuard()
  await prepareLoginIntentVersion()
  assert.doesNotThrow(outer)
  assert.equal(dom.localStorage.getItem(C), null)
  assert.match(dom.localStorage.getItem(I) ?? '', /^[a-f0-9-]{36}$/)
  assert.equal(lockCalls, 1)
  assert.deepEqual(names, [AUTH_COOKIE_LOCK])
})
test('two cold preparations reuse I under the existing lock and leave C untouched', async () => {
  const [a, b] = await Promise.all([
    prepareLoginIntentVersion(),
    prepareLoginIntentVersion(),
  ])
  assert.equal(dom.localStorage.getItem(C), null)
  assert.match(dom.localStorage.getItem(I) ?? '', /^[a-f0-9-]{36}$/)
  assert.equal(typeof a, 'object')
  assert.equal(typeof b, 'object')
  assert.equal(JSON.stringify(a), JSON.stringify(b))
  assert.equal(lockCalls, 2)
  assert.ok(names.every((name) => name === AUTH_COOKIE_LOCK))
  assert.equal(dispatches, 0)
})
test('queued cold click cannot borrow cancellation generation written while it waits', async () => {
  const release = holdLock()
  const outcome = beginFederatedLogin(provider, true).then(
    () => null,
    (error) => error
  )
  await settle()
  invalidatePendingLogins()
  release()
  assert.match(String(await outcome), /superseded/)
  assert.equal(dispatches, 0)
  assert.equal(dom.localStorage.getItem(I), null)
})
test('a genuinely new click after cancellation can prepare normally', async () => {
  invalidatePendingLogins()
  const generation = dom.localStorage.getItem(C)
  await beginFederatedLogin(provider, true)
  assert.equal(dom.localStorage.getItem(C), generation)
  assert.equal(dispatches, 1)
})
test('identity change during cold preparation rejects before native begin', async () => {
  const release = holdLock()
  const outcome = beginFederatedLogin(provider, true).then(
    () => null,
    (error) => error
  )
  await settle()
  useAuthStore.getState().auth.setBundle(bundle())
  release()
  assert.match(String(await outcome), /superseded/)
  assert.equal(dispatches, 0)
})
test('cleared localStorage invalidates an issued native ceremony without reconstructing I', async () => {
  await beginFederatedLogin(provider, true)
  const state = lastFlow.state
  dispatches = 0
  dom.localStorage.clear()
  await assert.rejects(
    finishFederatedLogin(provider.slug, { state, code: 'old-code' }),
    /superseded/
  )
  assert.equal(dispatches, 0)
  assert.equal(dom.localStorage.getItem(I), null)
})
test('old ceremony cannot resurrect after clear plus a fresh initialization', async () => {
  await beginFederatedLogin(provider, true)
  const state = lastFlow.state
  dom.localStorage.clear()
  await prepareLoginIntentVersion()
  dispatches = 0
  await assert.rejects(
    finishFederatedLogin(provider.slug, { state, code: 'old-code' }),
    /superseded/
  )
  assert.equal(dispatches, 0)
})
test('legacy raw-C redirect versions are rejected even if current C matches', async () => {
  const generation = crypto.randomUUID()
  dom.localStorage.setItem(C, generation)
  const ceremony = flow()
  saveFederatedCeremony(
    dom.sessionStorage,
    provider,
    ceremony,
    'login',
    generation
  )
  await assert.rejects(
    finishFederatedLogin(provider.slug, {
      state: ceremony.state,
      code: 'old-code',
    }),
    /superseded/
  )
  assert.equal(dispatches, 0)
})
test('native redirect stores composite version within128 and finishes once', async () => {
  await beginFederatedLogin(provider, true)
  const state = lastFlow.state
  const receipt = readFederatedCeremony(
    dom.sessionStorage,
    state,
    provider.slug,
    'login'
  )
  assert.ok(receipt)
  const version = receipt.loginIntentVersion
  assert.ok(version)
  assert.match(version, /^lmi1:/)
  assert.ok(version.length <= 128)
  assert.equal(version.split(':')[2], dom.localStorage.getItem(C) ?? 'initial')
  dispatches = 0
  await finishFederatedLogin(provider.slug, { state, code: 'code' })
  assert.equal(dispatches, 1)
  await assert.rejects(
    finishFederatedLogin(provider.slug, { state, code: 'code' })
  )
  assert.equal(dispatches, 1)
})
test('warm preparation does not reacquire Cookie lock', async () => {
  await prepareLoginIntentVersion()
  const cold = lockCalls
  await prepareLoginIntentVersion()
  assert.equal(lockCalls, cold)
})
test('already aborted preparation does not initialize or dispatch', async () => {
  const controller = new AbortController()
  controller.abort()
  await assert.rejects(
    async () => await prepareLoginIntentVersion(controller.signal)
  )
  assert.equal(dom.localStorage.getItem(I), null)
  assert.equal(dispatches, 0)
})
async function passive(extraWrite: boolean, cold = false) {
  if (!cold) await prepareLoginIntentVersion()
  useAuthStore.getState().auth.setBundle(bundle())
  let release!: () => void
  let enter!: () => void
  const entered = new Promise<void>((r) => {
    enter = r
  })
  const blocked = new Promise<void>((r) => {
    release = r
  })
  setDevelopmentAuthRefreshAdapter(async (config) => {
    enter()
    await blocked
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: bundle('passive-token') },
    }
  })
  const refresh = refreshAuthentication()
  await entered
  const pending = beginFederatedLogin(provider, true).then(
    () => null,
    (error) => error
  )
  let unsubscribe = () => {}
  if (extraWrite) {
    unsubscribe = useAuthStore.subscribe((state) => {
      if (state.auth.accessToken === 'passive-token') {
        unsubscribe()
        state.auth.setBundle(bundle('extra-write'))
      }
    })
  }
  release()
  await refresh
  unsubscribe()
  const result = await pending
  if (extraWrite) {
    assert.match(String(result), /superseded/)
    assert.equal(dispatches, 0)
  } else {
    assert.equal(result, null)
    assert.equal(dispatches, 1)
  }
}
test('precise already-running passive-refresh receipt still permits queued native login', () =>
  passive(false))
test('same-user same-SID extra account write cannot masquerade as passive receipt', () =>
  passive(true))

test('cold initialization queued behind an already-running passive refresh keeps its exact receipt', () =>
  passive(false, true))
test('cold initialization does not broaden a passive receipt to extra same-SID writes', () =>
  passive(true, true))
