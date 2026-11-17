/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import type { TFunction } from 'i18next'

import {
  createIPAccessRoutingSchema,
  createRegionRoutingRules,
  DEFAULT_IP_ACCESS_ROUTING_RULES,
} from './ip-access-routing-config'

const t = ((key: string) => key) as TFunction

describe('IP access routing settings', () => {
  test('ships with the China reject rule', () => {
    assert.equal(
      DEFAULT_IP_ACCESS_ROUTING_RULES,
      '# China\ndip(geoip:cn) -> reject'
    )
  })

  test('requires a bounded non-empty rule source before server validation', () => {
    const schema = createIPAccessRoutingSchema(t)
    assert.equal(
      schema.safeParse({
        IPAccessRoutingRules: DEFAULT_IP_ACCESS_ROUTING_RULES,
      }).success,
      true
    )
    assert.equal(
      schema.safeParse({ IPAccessRoutingRules: '   ' }).success,
      false
    )
    assert.equal(
      schema.safeParse({ IPAccessRoutingRules: 'x'.repeat(16 * 1024 + 1) })
        .success,
      false
    )
    assert.equal(
      schema.safeParse({ IPAccessRoutingRules: '中'.repeat(6000) }).success,
      false
    )
  })
})

describe('region rule builder', () => {
  test('block mode preserves access from unmatched regions and sets unknown behavior', () => {
    assert.equal(
      createRegionRoutingRules('cn, jp, CN', 'block', 'direct'),
      'unknown-country: direct\nip(geoip:CN, geoip:JP) -> reject\nfallback: direct'
    )
  })
  test('allow-only mode explicitly rejects other regions', () => {
    assert.equal(
      createRegionRoutingRules('US\nCA', 'allow', 'reject'),
      'unknown-country: reject\nip(geoip:US, geoip:CA) -> direct\nfallback: reject'
    )
  })
  test('invalid country inputs cannot inject arbitrary routing rules', () => {
    assert.equal(createRegionRoutingRules('', 'block', 'reject'), null)
    assert.equal(createRegionRoutingRules('USA', 'block', 'reject'), null)
    assert.equal(
      createRegionRoutingRules('CN) -> direct', 'block', 'reject'),
      null
    )
  })
})
