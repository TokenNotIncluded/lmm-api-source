/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

import { legacySettingsDestination } from './legacy-settings-destination'

function resolve(category: string, section: string) {
  const result = legacySettingsDestination(category, section)
  assert.ok(result, `${category}/${section}`)
  return result
}

function href(category: string, section: string) {
  const result = resolve(category, section)
  assert.equal(result.action, 'redirect')
  return result.href
}

test('every registered section has a disposition; missing functions cannot fall through to the settings overview', () => {
  let total = 0
  for (const [category, expected] of Object.entries({
    auth: 6,
    billing: 7,
    content: 8,
    models: 8,
    operations: 10,
    security: 10,
    site: 7,
  })) {
    const source = readFileSync(
      new URL(`./${category}/section-registry.tsx`, import.meta.url),
      'utf8'
    )
    const ids = Array.from(source.matchAll(/\bid:\s*'([^']+)'/g), (m) => m[1])
    assert.equal(ids.length, expected)
    for (const id of ids) {
      total++
      const result = resolve(category, id)
      assert.ok(result.detail)
      if (result.action === 'unavailable') {
        assert.ok(
          result.coverage === 'pending' || result.coverage === 'unsupported'
        )
        assert.ok(!('href' in result))
      }
      if (result.action === 'redirect') {
        assert.match(result.href, /^\/(?!\/)/)
        assert.ok(!/^\/system-settings(?:\/site)?\/?$/.test(result.href))
      }
    }
  }
  assert.equal(total, 56)
})

test('customer model prices, business groups and upstream cost open their exact editors', () => {
  assert.equal(href('billing', 'model-pricing'), '/channels?view=model-pricing')
  assert.equal(href('billing', 'group-pricing'), '/channels?view=group-pricing')
  assert.equal(
    href('billing', 'router-pricing'),
    '/channels?view=model-pricing&mode=upstream_cost'
  )
  assert.equal(
    href('security', 'ip-access-routing'),
    '/system-settings/site?domain=identity'
  )
  const raw = resolve('operations', 'raw-json')
  assert.equal(raw.action, 'choose')
  assert.deepEqual(
    raw.choices.map((choice) => choice.href),
    [
      href('billing', 'group-pricing'),
      href('billing', 'model-pricing'),
      href('billing', 'router-pricing'),
      '/system-settings/site?domain=billing',
    ]
  )
  const rewards = resolve('billing', 'quota')
  assert.equal(rewards.action, 'choose')
  assert.deepEqual(
    rewards.choices.map((choice) => choice.href),
    ['/redemption-codes', '/operations/sources', href('billing', 'checkin')]
  )
})

test('plugins and home-page controls stay distinct from unrelated features', () => {
  for (const [cat, id, plugin] of [
    ['site', 'header-navigation', 'community'],
    ['content', 'assistant', 'assistant'],
    ['security', 'market-ai-review', 'toolmarket'],
    ['operations', 'scripts', 'operations'],
    ['billing', 'checkin', 'checkins'],
    ['billing', 'payment', 'payments'],
  ]) {
    assert.equal(href(cat, id), `/extensions/plugins?configure=${plugin}`)
  }
  assert.notEqual(href('site', 'ai-directory-ads'), href('site', 'rss'))
  assert.equal(resolve('operations', 'update-checker').coverage, 'read-only')
  for (const [cat, id] of [
    ['content', 'drawing'],
    ['operations', 'worker'],
    ['models', 'gemini'],
    ['models', 'channel-affinity'],
    ['models', 'service-tier-pricing'],
    ['content', 'faq'],
    ['site', 'sidebar-modules'],
    ['security', 'advanced-security'],
  ]) {
    assert.equal(resolve(cat, id).coverage, 'unsupported')
  }
  for (const id of ['notice', 'ai-directory']) {
    const item = resolve('site', id)
    if (item.action === 'redirect') {
      assert.doesNotMatch(item.href, /manage=ads|configure=store(?:&|$)/)
    }
  }
})

test('unknown paths and inherited object properties do not create destinations', () => {
  for (const [category, section] of [
    ['', ''],
    ['unknown', 'scripts'],
    ['site', 'scripts'],
    ['constructor', 'notice'],
    ['__proto__', 'toString'],
    ['site', '__proto__'],
    ['site', 'constructor'],
    ['site', 'notice?other=1'],
  ]) {
    assert.equal(legacySettingsDestination(category, section), undefined)
  }
})
