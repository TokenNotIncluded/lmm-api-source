/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'

import { paySubscriptionEpay } from './api.js'

const originalPost = api.post

afterEach(() => {
  api.post = originalPost
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
