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
  createDiscountCodes,
  deleteExhaustedDiscountCodes,
  listDiscountCodes,
  updateDiscountCodeStatus,
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

describe('discount code API', () => {
  test('lists codes through /core/v1/payment-discounts/admin/codes', async () => {
    api.get = (async (url: string) => {
      assert.equal(
        url,
        '/core/v1/payment-discounts/admin/codes?limit=20&keyword=vip&enabled=true'
      )
      return {
        data: {
          success: true,
          data: {
            items: [
              {
                id: '42',
                code: 'VIP10',
                name: 'VIP',
                discount_percent: 10,
                min_credit_units: '100',
                max_uses: '5',
                enabled: true,
                starts_at: '1000',
                ends_at: '2000',
                used_count: '1',
                reserved_count: '0',
              },
            ],
            next_after: null,
          },
        },
      }
    }) as typeof api.get

    const result = await listDiscountCodes({
      page: 1,
      pageSize: 20,
      keyword: 'vip',
      status: '1',
    })
    assert.equal(result.success, true)
    assert.equal(result.data?.items[0]?.id, 42)
    assert.equal(result.data?.items[0]?.min_amount, 100)
    assert.equal(result.data?.items[0]?.status, 1)
  })

  test('creates batch codes and maps items array', async () => {
    api.post = (async (url: string, body: Record<string, unknown>) => {
      assert.equal(url, '/core/v1/payment-discounts/admin/codes/batch')
      assert.equal(typeof body.request_key, 'string')
      assert.equal(body.min_credit_units, 50)
      assert.equal(body.starts_at, 1000)
      assert.equal(body.ends_at, 2000)
      assert.equal(body.enabled, true)
      return {
        data: {
          success: true,
          data: {
            items: [
              {
                id: '7',
                code: 'ABC',
                name: 'Batch',
                discount_percent: 20,
                min_credit_units: '50',
                max_uses: '10',
                enabled: true,
                starts_at: '1000',
                ends_at: '2000',
                used_count: '0',
                reserved_count: '0',
              },
            ],
          },
        },
      }
    }) as typeof api.post

    const result = await createDiscountCodes({
      name: 'Batch',
      count: 1,
      discount_percent: 20,
      min_amount: 50,
      max_uses: 10,
      starts_time: 1000,
      expired_time: 2000,
    })
    assert.equal(result.success, true)
    assert.equal(result.data?.length, 1)
    assert.equal(result.data?.[0]?.code, 'ABC')
  })

  test('updates status with { enabled: boolean }', async () => {
    api.put = (async (url: string, body: Record<string, unknown>) => {
      assert.equal(url, '/core/v1/payment-discounts/admin/codes/9/status')
      assert.deepEqual(body, { enabled: false })
      return {
        data: {
          success: true,
          data: {
            id: '9',
            code: 'X',
            name: 'X',
            discount_percent: 5,
            min_credit_units: '0',
            max_uses: '0',
            enabled: false,
            starts_at: '0',
            ends_at: '0',
            used_count: '0',
            reserved_count: '0',
          },
        },
      }
    }) as typeof api.put

    const result = await updateDiscountCodeStatus(9, 2)
    assert.equal(result.success, true)
    assert.equal(result.data?.status, 2)
  })

  test('deletes exhausted codes and normalizes string count', async () => {
    api.delete = (async (url: string) => {
      assert.equal(url, '/core/v1/payment-discounts/admin/codes/exhausted')
      return { data: { success: true, message: '', data: { count: '2' } } }
    }) as typeof api.delete

    const result = await deleteExhaustedDiscountCodes()
    assert.equal(result.success, true)
    assert.equal(result.data?.count, 2)
  })
})
