/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { describe, it } from 'node:test'

import {
  canOpenConfiguration,
  configurationOwner,
  legacyPluginPanel,
} from './configuration'

describe('plugin configuration links', () => {
  for (const [path, hash, panel, plugin] of [
    ['/system-settings/content/assistant', '', 'assistant', 'assistant'],
    ['/system-settings/billing/checkin', '', 'checkins', 'checkins'],
    ['/system-settings/operations/hero-sms/', '', 'hero-sms', 'hero-sms'],
    ['/system-settings/operations/scripts', '', 'operations', 'operations'],
    ['/system-settings/site/rss', '', 'rss', 'operations'],
    [
      '/system-settings/site/ai-directory-ads',
      '',
      'directory-ads',
      'community',
    ],
    ['/system-settings/site/ai-directory', '', 'community', 'community'],
    [
      '/system-settings/site/header-navigation',
      '',
      'header-navigation',
      'community',
    ],
    [
      '/system-settings/models/model-deployment',
      '',
      'model-deployment',
      'operations',
    ],
    ['/system-settings/billing/payment', '#merchant-store', 'store', 'store'],
    ['/system-settings/billing/payment/', 'merchant-store', 'store', 'store'],
  ]) {
    it(`opens the published owner for ${path} ${hash}`, () => {
      assert.equal(legacyPluginPanel(path, hash), panel)
      assert.equal(configurationOwner(panel), plugin)
    })
  }
  for (const hash of ['', 'stripe', 'merchant-store-other']) {
    it(`opens payment gateways for payment anchor ${hash}`, () => {
      assert.equal(
        legacyPluginPanel('/system-settings/billing/payment', hash),
        'payments'
      )
    })
  }
  it('routes both markets to a shared editor while keeping owner settings protected', () => {
    assert.equal(
      legacyPluginPanel('/system-settings/security/market-ai-review/'),
      'toolmarket'
    )
    assert.equal(configurationOwner('store-review'), 'store')
    assert.equal(configurationOwner('toolmarket'), 'toolmarket')
    assert.equal(canOpenConfiguration('store-review', false), true)
    assert.equal(canOpenConfiguration('toolmarket', false), true)
    assert.equal(canOpenConfiguration('payments', false), false)
    assert.equal(canOpenConfiguration('payments', true), true)
  })
  it('retains the old root-only boundary for new plugin editors', () => {
    for (const panel of [
      'rss',
      'checkins',
      'hero-sms',
      'community',
      'store',
      'directory-ads',
      'header-navigation',
      'model-deployment',
    ]) {
      assert.equal(canOpenConfiguration(panel, false), false)
      assert.equal(canOpenConfiguration(panel, true), true)
    }
    for (const panel of ['operations', 'assistant', 'acquisition']) {
      assert.equal(canOpenConfiguration(panel, false), true)
    }
  })
  it('rejects unregistered configuration and prototype keys', () => {
    for (const input of [
      undefined,
      [],
      {},
      '',
      'toString',
      '__proto__',
      'unknown',
    ]) {
      assert.equal(canOpenConfiguration(input, true), false)
    }
  })
})
