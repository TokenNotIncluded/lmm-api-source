/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'

import type { NativeConfiguration } from '../native-api'
import {
  assertNativeDraftTimeouts,
  NativeTargetTimeoutError,
  nativeGlobalTimeoutMs,
  nativeTimeoutErrorMessage,
} from './native-target-timeout'

const configuration: NativeConfiguration = {
  version: { routing: 1, pricing: 1 },
  model_limits: { total_timeout_ms: 120_000 },
  routing: {
    version: 1,
    upstreams: [
      {
        id: 1,
        endpoint: 'https://example.com',
        protocol: 'openai_chat',
        credential: { id: 1, revision: 1 },
        enabled: true,
      },
      {
        id: 2,
        endpoint: 'https://example.com',
        protocol: 'openai_chat',
        credential: { id: 2, revision: 1 },
        enabled: false,
      },
    ],
    routes: [],
    groups: [],
    aliases: [],
  },
  prices: { version: 1, rules: [] },
  bindings: {
    names: {},
    contexts: [],
    vendors: {},
    models: {},
    discoveries: {},
    price_drafts: {},
  },
  secret_storage_enabled: true,
}

test('timeout admission follows enabled upstreams and also validates advanced channel drafts', () => {
  const model = {
    route: {
      enabled: false,
      targets: [{ upstream_id: 1, enabled: false, timeout_ms: 30_000 }],
    },
  }
  assert.throws(
    () => assertNativeDraftTimeouts(model, configuration),
    NativeTargetTimeoutError
  )
  assert.doesNotThrow(() =>
    assertNativeDraftTimeouts(
      { route: { targets: [{ upstream_id: 2, timeout_ms: 30_000 }] } },
      configuration
    )
  )
  assert.throws(
    () =>
      assertNativeDraftTimeouts(
        { enabled: true, attachments: [{ timeout_ms: 30_000 }] },
        configuration
      ),
    NativeTargetTimeoutError
  )
  assert.doesNotThrow(() =>
    assertNativeDraftTimeouts(
      { enabled: false, attachments: [{ timeout_ms: 30_000 }] },
      configuration
    )
  )
  assert.doesNotThrow(() =>
    assertNativeDraftTimeouts(
      { enabled: true, attachments: [{ timeout_ms: 120_000 }] },
      configuration
    )
  )
  assert.equal(nativeGlobalTimeoutMs({ model_limits: null }), null)
  assert.equal(nativeGlobalTimeoutMs({}), null)
  assert.doesNotThrow(() =>
    assertNativeDraftTimeouts(model, { ...configuration, model_limits: null })
  )
})

test('local and Core timeout errors show the exact minimum without blaming credentials', async () => {
  const i18n = createInstance()
  await i18n.init({
    lng: 'en',
    resources: { en: { translation: {} } },
    keySeparator: false,
    nsSeparator: false,
  })
  assert.equal(
    nativeTimeoutErrorMessage(new NativeTargetTimeoutError(120_000), i18n.t),
    'Target timeout must be at least 120000 ms for enabled channels.'
  )
  assert.equal(
    nativeTimeoutErrorMessage(
      {
        response: {
          data: {
            success: false,
            code: 'CHANNEL_TARGET_TIMEOUT_TOO_SHORT',
            message: 'internal wording is not UI copy',
            data: { required_min_timeout_ms: 180_000 },
          },
        },
      },
      i18n.t
    ),
    'Target timeout must be at least 180000 ms for enabled channels.'
  )
  assert.match(
    nativeTimeoutErrorMessage(
      {
        code: 'CHANNEL_TARGET_TIMEOUT_TOO_SHORT',
        data: { required_min_timeout_ms: 'unsafe' },
      },
      i18n.t
    ) ?? '',
    /Reload configuration/
  )
  assert.equal(
    nativeTimeoutErrorMessage(
      { code: 'CHANNEL_BINDING_NOT_CONFIGURED' },
      i18n.t
    ),
    null
  )
})
