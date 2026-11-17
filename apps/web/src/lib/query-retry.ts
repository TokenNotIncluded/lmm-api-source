/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { DefaultError } from '@tanstack/react-query'
import { isAxiosError } from 'axios'

import { isRateLimitedError } from './request-rate-limit'

/** Do not amplify backpressure or retry a route the server has not implemented. */
export function createQueryRetry(production: boolean, maximumRetries = 4) {
  return (failureCount: number, error: DefaultError): boolean =>
    production &&
    failureCount < maximumRetries &&
    !isRateLimitedError(error) &&
    !(
      isAxiosError(error) &&
      ([401, 402, 403, 429].includes(error.response?.status ?? 0) ||
        error.response?.data?.code === 'core_migration_incomplete' ||
        error.response?.data?.error?.code === 'core_migration_incomplete')
    )
}
