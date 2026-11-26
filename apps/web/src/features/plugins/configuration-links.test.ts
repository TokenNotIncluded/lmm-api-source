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
    it(`keeps payment anchor ${hash} in billing`, () => {
      assert.equal(
        legacyPluginPanel('/system-settings/billing/payment', hash),
        undefined
      )
    })
  }
  it('retains the old root-only boundary for new plugin editors', () => {
    for (const panel of [
      'rss',
      'community',
      'store',
      'directory-ads',
      'header-navigation',
      'model-deployment',
    ]) {
      assert.equal(canOpenConfiguration(panel, false), false)
      assert.equal(canOpenConfiguration(panel, true), true)
    }
    for (const panel of [
      'operations',
      'checkins',
      'hero-sms',
      'assistant',
      'acquisition',
    ]) {
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
