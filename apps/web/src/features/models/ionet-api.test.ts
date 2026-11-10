/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import type { AxiosAdapter, AxiosResponse } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/models/deployments',
})
for (const key of [
  'window',
  'document',
  'localStorage',
  'Event',
  'navigator',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: key === 'window' ? dom : dom[key],
  })
}
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const {
  ioNetMutation,
  pendingIoNetOperations,
  resumeIoNetOperation,
  saveIoNetSettings,
  readPendingIoNetOperations,
} = await import('./ionet-api')
const originalAdapter = api.defaults.adapter
function owner(id: number) {
  useAuthStore
    .getState()
    .auth.setUser({ id, username: `fixture-${id}`, role: 100 })
}
function response(
  config: Parameters<AxiosAdapter>[0],
  data: unknown,
  status = 200
): AxiosResponse {
  return { config, data, status, statusText: 'fixture', headers: {} }
}
afterEach(() => {
  api.defaults.adapter = originalAdapter
  dom.localStorage.clear()
  useAuthStore.getState().auth.setUser(null)
})
after(() => dom.happyDOM.abort())

test('lost response retains only owner handle; Resume never repeats paid draft', async () => {
  owner(501)
  let mutations = 0
  let operation = ''
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/deployments/') {
      mutations++
      operation = String(config.headers.get('Idempotency-Key'))
      throw new Error('offline transport lost after upstream admission')
    }
    assert.equal(
      config.url,
      `/api/deployments/operations/receipts/${operation}/recover`
    )
    assert.equal(config.data, '{}')
    return response(config, {
      data: {
        operation_key: operation,
        kind: 'create',
        state: 'succeeded',
        deployment_id: 'remote-fixture',
        config_revision: 1,
        result: { deployment_id: 'remote-fixture' },
      },
    })
  }
  await assert.rejects(
    ioNetMutation('post', '/api/deployments/', {
      registry_secret: 'private-registry',
      secret_env_variables: { key: 'private-env' },
    }),
    /result is unknown/
  )
  const persisted =
    dom.localStorage.getItem('lmm.ionet.operations.v1:501') ?? ''
  assert.ok(persisted.includes(operation))
  assert.ok(!persisted.includes('private-'))
  assert.equal(pendingIoNetOperations().length, 1)
  await assert.rejects(
    ioNetMutation('post', '/api/deployments/', {
      resource_private_name: 'another-draft',
    }),
    /result is unknown/
  )
  assert.equal(mutations, 1)
  owner(502)
  assert.equal(pendingIoNetOperations().length, 0)
  owner(501)
  const receipt = await resumeIoNetOperation(operation)
  assert.equal(receipt.state, 'succeeded')
  assert.equal(mutations, 1)
  assert.equal(pendingIoNetOperations().length, 0)
})

test('UNKNOWN and cancelled handles preserve explicit recovery semantics', async () => {
  owner(501)
  let key = ''
  let posts = 0
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/deployments/') {
      posts++
      key = String(config.headers.get('Idempotency-Key'))
      return response(
        config,
        {
          data: {
            operation_key: key,
            kind: 'create',
            state: 'unknown',
            config_revision: 3,
          },
        },
        202
      )
    }
    assert.equal(
      config.url,
      `/api/deployments/operations/receipts/${key}/cancel`
    )
    return response(config, {
      data: {
        operation_key: key,
        kind: 'cancel',
        state: 'cancelled',
        config_revision: 0,
      },
    })
  }
  const result = await ioNetMutation('post', '/api/deployments/', {})
  assert.equal(result.success, false)
  assert.equal(pendingIoNetOperations().length, 1)
  await resumeIoNetOperation(key, true)
  assert.equal(posts, 1)
  assert.equal(pendingIoNetOperations().length, 0)
})

test('server pending handles recover without browser form or credential snapshot', async () => {
  owner(501)
  api.defaults.adapter = async (config) => {
    assert.equal(config.method, 'get')
    assert.equal(config.url, '/api/deployments/operations')
    return response(config, {
      success: true,
      data: [
        {
          operation_key: 'persisted_original_0001',
          kind: 'extend',
          state: 'unknown',
          deployment_id: 'deployment-1',
          config_revision: 2,
        },
      ],
    })
  }
  assert.deepEqual(await readPendingIoNetOperations(), [
    {
      key: 'persisted_original_0001',
      method: 'post',
      path: '/api/deployments/deployment-1/extend',
    },
  ])
})

test('settings CAS failure exposes no Axios request credential snapshot', async () => {
  owner(501)
  api.defaults.adapter = async (config) => {
    throw Object.assign(new Error('conflict'), { config })
  }
  await assert.rejects(
    saveIoNetSettings({
      revision: 1,
      enabled: true,
      api_key: 'private-provider-key',
      clear_api_key: false,
    }),
    (error: unknown) => {
      assert.ok(error instanceof Error)
      assert.equal(error.message, 'Failed to save')
      assert.equal('config' in error, false)
      return true
    }
  )
  assert.equal(dom.localStorage.getItem('lmm.ionet.operations.v1:501'), null)
})
