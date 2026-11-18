/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { AxiosError } from 'axios'

import { resolvePricingRouteAccess } from '@/features/pricing/lib/route-access'
import { useAuthStore } from '@/stores/auth-store'

import { api, getStatus } from './api'
import { getFreshModuleAccess } from './nav-modules'

const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth

afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
})

function navigationResponse(value: unknown) {
  return {
    success: true,
    message: '',
    data: { value: JSON.stringify(value), revision: 2 },
  }
}

test('pricing requires sign-in from site navigation when public status has no navigation fields', async () => {
  useAuthStore.getState().auth.reset('complete')
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    requests.push(config.url ?? '')
    return {
      config,
      data:
        config.url === '/api/status'
          ? { success: true, data: { system_name: 'LMM API' } }
          : navigationResponse({
              pricing: { enabled: true, requireAuth: true },
            }),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }

  assert.equal((await getStatus()).HeaderNavModules, undefined)
  let bootstrapCalls = 0
  const decision = await resolvePricingRouteAccess(
    '/pricing?view=card',
    'marketplace',
    {
      loadModuleAccess: () => getFreshModuleAccess('pricing'),
      bootstrapAuth: async () => {
        bootstrapCalls += 1
      },
      getUser: () => null,
    }
  )

  assert.deepEqual(decision, {
    kind: 'redirect-sign-in',
    redirect: '/pricing?view=card',
  })
  assert.equal(bootstrapCalls, 1)
  assert.deepEqual(requests, ['/api/status', '/api/site/navigation'])
})

test('public site navigation keeps pricing accessible without forcing authentication', async () => {
  useAuthStore.getState().auth.reset('complete')
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/site/navigation')
    return {
      config,
      data: navigationResponse({
        pricing: { enabled: true, requireAuth: false },
      }),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const decision = await resolvePricingRouteAccess('/pricing', 'marketplace', {
    loadModuleAccess: () => getFreshModuleAccess('pricing'),
    bootstrapAuth: async () => {
      assert.fail('Public pricing must not force sign-in')
    },
    getUser: () => null,
  })
  assert.deepEqual(decision, { kind: 'allow' })
})

test('disabled pricing in site navigation redirects home before authentication', async () => {
  useAuthStore.getState().auth.reset('complete')
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/site/navigation')
    return {
      config,
      data: navigationResponse({ pricing: false }),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const decision = await resolvePricingRouteAccess('/pricing', 'marketplace', {
    loadModuleAccess: () => getFreshModuleAccess('pricing'),
    bootstrapAuth: async () => {
      assert.fail('A disabled page must not request sign-in')
    },
    getUser: () => null,
  })
  assert.deepEqual(decision, { kind: 'redirect-home' })
})

test('an unavailable navigation read keeps the route closed without falling back to public status', async () => {
  useAuthStore.getState().auth.reset('complete')
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    requests.push(config.url ?? '')
    assert.equal(config.url, '/api/site/navigation')
    assert.equal(config.skipBusinessError, true)
    assert.equal(config.skipErrorHandler, true)
    throw new AxiosError(
      'Navigation unavailable',
      'ERR_BAD_RESPONSE',
      config,
      undefined,
      {
        config,
        data: { success: false, code: 'unavailable' },
        headers: {},
        status: 503,
        statusText: 'Service Unavailable',
      }
    )
  }
  assert.deepEqual(await getFreshModuleAccess('pricing'), {
    enabled: false,
    requireAuth: true,
  })
  assert.deepEqual(requests, ['/api/site/navigation'])
})
