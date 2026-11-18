/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { TFunction } from 'i18next'

import type { NativeConfiguration } from '../native-api'

type TimeoutConfiguration = Pick<NativeConfiguration, 'model_limits'>

export function nativeGlobalTimeoutMs(
  configuration?: TimeoutConfiguration | null
): number | null {
  const value = configuration?.model_limits?.total_timeout_ms
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
    ? value
    : null
}

export function newNativeTargetTimeoutMs(
  configuration?: TimeoutConfiguration
): number {
  return Math.max(30_000, nativeGlobalTimeoutMs(configuration) ?? 0)
}

export function targetTimeoutTooShort(
  value: unknown,
  minimum: number | null
): boolean {
  return minimum !== null && typeof value === 'number' && value < minimum
}

export class NativeTargetTimeoutError extends Error {
  readonly code = 'CHANNEL_TARGET_TIMEOUT_TOO_SHORT'
  readonly data: { required_min_timeout_ms: number }

  constructor(minimum: number) {
    super('Target timeout is below the server global deadline.')
    this.data = { required_min_timeout_ms: minimum }
  }
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

/** Match Core admission: every target of an enabled upstream is checked. */
export function assertNativeDraftTimeouts(
  body: Record<string, unknown>,
  configuration: NativeConfiguration
): void {
  const minimum = nativeGlobalTimeoutMs(configuration)
  if (minimum === null) return
  const route = record(body.route)
  const targets = route?.targets
  const enabled = new Set(
    configuration.routing.upstreams
      .filter((item) => item.enabled)
      .map((item) => item.id)
  )
  if (Array.isArray(targets)) {
    for (const value of targets) {
      const target = record(value)
      if (
        target &&
        enabled.has(target.upstream_id as number) &&
        targetTimeoutTooShort(target.timeout_ms, minimum)
      ) {
        throw new NativeTargetTimeoutError(minimum)
      }
    }
  }
  if (body.enabled === true && Array.isArray(body.attachments)) {
    for (const value of body.attachments) {
      if (targetTimeoutTooShort(record(value)?.timeout_ms, minimum)) {
        throw new NativeTargetTimeoutError(minimum)
      }
    }
  }
}

/** Handle both a local validation error and the Core HTTP error envelope. */
export function nativeTimeoutErrorMessage(
  cause: unknown,
  t: TFunction
): string | null {
  const error = record(cause)
  const response = record(error?.response)
  const payload = record(response?.data) ?? error
  if (payload?.code !== 'CHANNEL_TARGET_TIMEOUT_TOO_SHORT') return null
  const minimum = record(payload.data)?.required_min_timeout_ms
  if (
    typeof minimum === 'number' &&
    Number.isSafeInteger(minimum) &&
    minimum > 0
  ) {
    return t(
      'Target timeout must be at least {{timeout}} ms for enabled channels.',
      {
        timeout: minimum,
      }
    )
  }
  return t(
    'The target timeout is below the server global deadline. Reload configuration to see the required minimum.'
  )
}
