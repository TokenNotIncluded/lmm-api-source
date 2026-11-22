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
import { useAuthStore } from '@/stores/auth-store'

import {
  executeSubscriptionReset,
  getAdminPlans,
  getSubscriptionResetEligible,
  getSubscriptionResetVouchers,
  previewSubscriptionReset,
  paySubscriptionEpay,
  redeemSubscriptionResetVoucher,
} from './api.js'
import type {
  SubscriptionResetExecuteRequest,
  SubscriptionResetPreviewRequest,
} from './types.js'

const originalDelete = api.delete
const originalGet = api.get
const originalPost = api.post

afterEach(() => {
  api.delete = originalDelete
  api.get = originalGet
  api.post = originalPost
})

describe('subscription administration API', () => {
  test('requests archived plans for reset eligibility', async () => {
    const getCalls: unknown[][] = []
    api.get = (async (...args: unknown[]) => {
      getCalls.push(args)
      return { data: { success: true, data: [] } }
    }) as typeof api.get

    await getAdminPlans(true)
    assert.deepEqual(getCalls, [
      ['/api/subscription/admin/plans', { params: { include_archived: '1' } }],
    ])
  })

  test('serializes reset-target filters without losing cancellation', async () => {
    const calls: unknown[][] = []
    api.get = (async (...args: unknown[]) => {
      calls.push(args)
      return { data: { success: true, data: { items: [], total: 0 } } }
    }) as typeof api.get
    const controller = new AbortController()

    await getSubscriptionResetEligible(
      {
        page: 2,
        pageSize: 50,
        query: 'pro',
        planIds: [7, 9],
        userIds: [11, 13],
      },
      controller.signal
    )

    assert.deepEqual(calls, [
      [
        '/api/subscription/root/reset-targets',
        {
          params: {
            page: 2,
            page_size: 50,
            query: 'pro',
            plan_ids: '7,9',
            user_ids: '11,13',
          },
          signal: controller.signal,
        },
      ],
    ])
  })

  test('uses preview-bound reset and user voucher contracts', async () => {
    const posts: unknown[][] = []
    api.post = (async (...args: unknown[]) => {
      posts.push(args)
      return { data: { success: true, data: {} } }
    }) as typeof api.post
    api.get = (async (url: string) => ({
      data: { success: true, data: url.includes('vouchers') ? [] : {} },
    })) as typeof api.get

    const preview: SubscriptionResetPreviewRequest = {
      mode: 'soft',
      all_matching: false,
      targets: [{ user_id: 11, plan_id: 7 }],
      filter: { query: 'alice', plan_ids: [7] },
    }
    const execute: SubscriptionResetExecuteRequest = {
      preview_token: 'preview-token',
      operation_id: 'operation-id',
    }

    await previewSubscriptionReset(preview)
    await executeSubscriptionReset(execute)
    await getSubscriptionResetVouchers()
    await redeemSubscriptionResetVoucher(99)

    assert.deepEqual(posts, [
      ['/api/subscription/root/reset/preview', preview],
      ['/api/subscription/root/reset', execute],
      ['/api/subscription/self/reset-vouchers/99/redeem'],
    ])
  })
})

test('EPay returns only the response payload and never recovers a wrapper checkout URL', async () => {
  const payload = { success: true, data: {} }
  api.post = (async () => ({
    data: payload,
    url: 'https://stale.invalid/checkout',
  })) as typeof api.post
  assert.equal(
    await paySubscriptionEpay({ plan_id: 7, payment_method: 'alipay' }),
    payload
  )
  assert.equal(
    (await paySubscriptionEpay({ plan_id: 7, payment_method: 'alipay' })).url,
    undefined
  )
})

test('native reset voucher reads and writes retain the original account and session scope', async () => {
  const originalAuth = useAuthStore.getState().auth
  const login = (id: number, sid: string) =>
    useAuthStore.getState().auth.setBundle({
      user: { id, role: 1, username: 'synthetic-voucher-user' },
      access_token: 'synthetic-voucher-session',
      token_type: 'Bearer',
      access_expires_at: 9999999999,
      session: {
        sid,
        current: true,
        login_method: 'password',
        ip: null,
        user_agent: null,
        created_at: 1,
        last_active_at: 1,
        expires_at: 9999999999,
      },
    })
  let release!: () => void
  const delayed = new Promise<void>((resolve) => {
    release = resolve
  })
  const scope = { userId: 7, sessionId: 'voucher-session-a' }
  let posts = 0
  try {
    login(scope.userId, scope.sessionId)
    api.get = (async (_url, config) => {
      assert.deepEqual(config?.authScope, scope)
      assert.equal(config?.skipAuthRefresh, true)
      await delayed
      return { data: { success: true, data: [] } }
    }) as typeof api.get
    const pending = getSubscriptionResetVouchers(scope)
    login(8, 'voucher-session-b')
    release()
    await assert.rejects(pending, /SESSION_CHANGED/)
    api.post = (async (_url, _body, config) => {
      posts++
      assert.deepEqual(config?.authScope, {
        userId: 8,
        sessionId: 'voucher-session-b',
      })
      assert.equal(config?.skipAuthRefresh, true)
      return {
        data: { success: true, data: { reset_count: 1, restored_quota: 100 } },
      }
    }) as typeof api.post
    await assert.rejects(
      redeemSubscriptionResetVoucher(81, scope),
      /SESSION_CHANGED/
    )
    assert.equal(posts, 0, 'a stale confirmation must not send a mutation')
    await redeemSubscriptionResetVoucher(82, {
      userId: 8,
      sessionId: 'voucher-session-b',
    })
    assert.equal(posts, 1)
  } finally {
    release()
    useAuthStore.setState({ auth: originalAuth })
  }
})
