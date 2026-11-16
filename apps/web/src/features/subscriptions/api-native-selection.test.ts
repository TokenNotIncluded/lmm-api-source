/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { AxiosError, type InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://synthetic.example/wallet' })
for (const name of [
  'window',
  'document',
  'navigator',
  'sessionStorage',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: name === 'window' ? dom : dom[name],
  })
}
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { getPublicPlans, getSelfSubscriptionFull, paySubscriptionBalance } =
  await import('./api')
const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth

function login(userId = 52) {
  useAuthStore.getState().auth.setBundle({
    user: { id: userId, role: 1, username: 'synthetic' },
    access_token: 'synthetic-access-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid: `synthetic-${userId}`,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
}

function response(config: InternalAxiosRequestConfig, data: unknown) {
  return { config, data, headers: {}, status: 200, statusText: 'OK' }
}

function failure(config: InternalAxiosRequestConfig, status: number) {
  return new AxiosError(
    'synthetic response',
    'ERR_BAD_RESPONSE',
    config,
    undefined,
    {
      config,
      status,
      statusText: 'Unavailable',
      data: { success: false, message: 'synthetic rejection' },
      headers: {},
    }
  )
}

afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
  dom.sessionStorage.clear()
})
after(() => dom.close())

test('classic balance failures never create a second payment through Core', async () => {
  login()
  for (const status of [404, 503, 0]) {
    const requests: string[] = []
    api.defaults.adapter = async (config) => {
      assert.ok(config.url)
      requests.push(config.url)
      assert.deepEqual(config.authScope, {
        userId: 52,
        sessionId: 'synthetic-52',
      })
      assert.equal(config.skipAuthRefresh, true)
      if (status === 0) throw new Error('acknowledgement lost')
      throw failure(config, status)
    }
    await assert.rejects(paySubscriptionBalance({ plan_id: 4 }))
    assert.deepEqual(requests, ['/api/subscription/balance/pay'])
  }
})

test('a classic payment reply cannot be applied to a replacement account', async () => {
  login()
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    assert.ok(config.url)
    requests.push(config.url)
    assert.deepEqual(config.authScope, {
      userId: 52,
      sessionId: 'synthetic-52',
    })
    login(53)
    return response(config, {
      success: true,
      data: { order_id: 'old-account' },
    })
  }
  await assert.rejects(paySubscriptionBalance({ plan_id: 4 }), {
    code: 'ERR_CANCELED',
    message: 'Authentication changed before request could be sent',
  })
  assert.deepEqual(requests, ['/api/subscription/balance/pay'])
})

test('classic business refusals and malformed replies do not choose another protocol', async () => {
  login()
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    assert.ok(config.url)
    requests.push(config.url)
    return response(config, { success: false, message: 'balance unavailable' })
  }
  assert.deepEqual(await paySubscriptionBalance({ plan_id: 4 }), {
    success: false,
    message: 'balance unavailable',
  })
  assert.deepEqual(await getPublicPlans(undefined, 'en'), {
    success: false,
    message: 'balance unavailable',
  })
  api.defaults.adapter = async (config) => {
    assert.ok(config.url)
    requests.push(config.url)
    return response(config, '<html>not the API</html>')
  }
  await assert.rejects(getPublicPlans(undefined, 'en'), /INVALID_RESPONSE/)
  await assert.rejects(
    paySubscriptionBalance({ plan_id: 4 }),
    /INVALID_RESPONSE/
  )
  assert.ok(requests.every((path) => path.startsWith('/api/subscription/')))
})

test('an explicit missing read route selects the validated Core contract for the original owner', async () => {
  login()
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    assert.ok(config.url)
    requests.push(config.url)
    assert.equal(config.method, 'get')
    assert.deepEqual(config.authScope, {
      userId: 52,
      sessionId: 'synthetic-52',
    })
    if (config.url?.startsWith('/api/subscription/')) throw failure(config, 404)
    return response(config, {
      contract: 'core.subscriptions.v1',
      purchases: [],
    })
  }
  const result = await getSelfSubscriptionFull()
  assert.equal(result.data?.contract, 'core.subscriptions.v1')
  assert.deepEqual(requests, [
    '/api/subscription/self',
    '/core/v1/subscriptions/purchases',
  ])
})

test('an unavailable read route is reported without querying another protocol', async () => {
  login()
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    assert.ok(config.url)
    requests.push(config.url)
    throw failure(config, 503)
  }
  await assert.rejects(getSelfSubscriptionFull())
  await assert.rejects(getPublicPlans(undefined, 'en'))
  assert.deepEqual(requests, [
    '/api/subscription/self',
    '/api/subscription/plans',
  ])
})

test('account changes and aborted reads cannot fall through to a new owner', async () => {
  login()
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    assert.ok(config.url)
    requests.push(config.url)
    login(53)
    throw failure(config, 404)
  }
  await assert.rejects(getSelfSubscriptionFull(), /SESSION_CHANGED/)
  assert.deepEqual(requests, ['/api/subscription/self'])

  login()
  const controller = new AbortController()
  api.defaults.adapter = async (config) => {
    assert.ok(config.url)
    requests.push(config.url)
    controller.abort()
    throw failure(config, 404)
  }
  await assert.rejects(getPublicPlans(controller.signal, 'en'))
  assert.deepEqual(requests, [
    '/api/subscription/self',
    '/api/subscription/plans',
  ])
})
