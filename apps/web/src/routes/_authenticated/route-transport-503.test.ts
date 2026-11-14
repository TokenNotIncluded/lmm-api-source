/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { test } from 'node:test'

import { assertRetryableColdProfile } from '@/test-support/route-transport-fault'

test('keeps a cold profile request retryable after transport status 503', async () => {
  await assertRetryableColdProfile(503)
})
