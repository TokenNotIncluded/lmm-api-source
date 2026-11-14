/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { afterEach, describe, test } from 'node:test'

import { api } from '@/lib/api'

import {
  createVendor,
  deleteVendor,
  getModel,
  getModels,
  getVendors,
  searchModels,
  updateVendor,
} from './api.js'

const originalGet = api.get
const originalPost = api.post
const originalPut = api.put
const originalDelete = api.delete

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
  api.put = originalPut
  api.delete = originalDelete
})

const version = { routing: 1, pricing: 1, bindings: 1 }

describe('models and vendors routing API', () => {
  test('reads, searches and gets models via /core/v1/admin/routing/models', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/core/v1/admin/routing/models')
      return {
        data: {
          success: true,
          data: {
            version,
            items: [
              {
                route: {
                  id: 1,
                  model: 'gpt-4o',
                  group: 'default',
                  enabled: true,
                },
                metadata: {
                  display_name: 'GPT-4o',
                  description: 'flagship',
                  vendor_id: 2,
                },
              },
              {
                route: {
                  id: 2,
                  model: 'claude-3-7',
                  group: 'vip',
                  enabled: false,
                },
                metadata: null,
              },
            ],
          },
        },
      }
    }) as typeof api.get

    const list = await getModels({ vendor: '2' })
    assert.equal(list.success, true)
    assert.equal(list.data?.items.length, 1)
    assert.equal(list.data?.items[0]?.model_name, 'gpt-4o')
    assert.equal(list.data?.items[0]?.status, 1)

    const searched = await searchModels({ keyword: 'claude', status: '2' })
    assert.equal(searched.data?.items.length, 1)
    assert.equal(searched.data?.items[0]?.id, 2)

    const single = await getModel(1)
    assert.equal(single.success, true)
    assert.equal(single.data?.description, 'flagship')
  })

  test('lists, creates, updates and deletes vendors via /core/v1/admin/routing/vendors', async () => {
    api.get = (async (url: string) => {
      assert.equal(url, '/core/v1/admin/routing/vendors')
      return {
        data: {
          success: true,
          data: {
            version,
            items: [{ id: 3, name: 'OpenAI', description: 'AI' }],
          },
        },
      }
    }) as typeof api.get

    const listed = await getVendors()
    assert.equal(listed.data?.items[0]?.name, 'OpenAI')

    api.post = (async (url: string, body: Record<string, unknown>) => {
      assert.equal(url, '/core/v1/admin/routing/vendors')
      assert.deepEqual(body, {
        expected_version: version,
        vendor: { id: 4, name: 'Anthropic', description: 'Claude' },
      })
      return { data: { success: true, data: { version, id: 4 } } }
    }) as typeof api.post

    const created = await createVendor({
      name: 'Anthropic',
      description: 'Claude',
    })
    assert.equal(created.success, true)
    assert.equal(created.data?.id, 4)

    api.put = (async (url: string, body: Record<string, unknown>) => {
      assert.equal(url, '/core/v1/admin/routing/vendors/3')
      assert.deepEqual(body, {
        expected_version: version,
        vendor: { id: 3, name: 'OpenAI Inc', description: 'Updated' },
      })
      return { data: { success: true, data: { version, id: 3 } } }
    }) as typeof api.put

    const updated = await updateVendor({
      id: 3,
      name: 'OpenAI Inc',
      description: 'Updated',
    })
    assert.equal(updated.success, true)
    assert.equal(updated.data?.name, 'OpenAI Inc')

    api.delete = (async (url: string, config?: { data?: unknown }) => {
      assert.equal(url, '/core/v1/admin/routing/vendors/3')
      assert.deepEqual(config?.data, { expected_version: version })
      return { data: { success: true, data: { version } } }
    }) as typeof api.delete

    const removed = await deleteVendor(3)
    assert.equal(removed.success, true)
  })
})
