/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { isRedirect } from '@tanstack/react-router'
import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider, initReactI18next } from 'react-i18next'

import zh from '@/i18n/locales/zh-CN.json'

import {
  legacyBookmarkSection,
  resolveLegacySettingsBookmark,
} from './legacy-settings-bookmark'
import { LegacySettingsOutcome } from './legacy-settings-outcome'

const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'zh-CN', resources: { 'zh-CN': zh } })
const lookup = (id: string) => ({ titleKey: id })
function render(category: string, section: string) {
  const result = resolveLegacySettingsBookmark(category, section, lookup)
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <LegacySettingsOutcome {...result} />
    </I18nextProvider>
  )
}
test('legacy loaders preserve query parameters, replace history, and recognize the old monitoring alias', () => {
  for (const [category, section, target] of [
    ['operations', 'monitoring', '/system-settings/site?domain=routing'],
    [
      'billing',
      'router-pricing',
      '/channels?view=model-pricing&mode=upstream_cost',
    ],
    ['site', 'rss', '/rss?manage=feeds'],
  ]) {
    assert.throws(
      () =>
        resolveLegacySettingsBookmark(category, section, () => {
          throw new Error('redirect must not read fallback registry')
        }),
      (error) =>
        isRedirect(error) &&
        error.options.href === target &&
        error.options.replace === true
    )
  }
  assert.equal(legacyBookmarkSection({ section: 'faq' }, 'dashboard'), 'faq')
  assert.equal(legacyBookmarkSection({}, 'dashboard'), 'dashboard')
  assert.equal(
    legacyBookmarkSection({ section: ['unknown'] }, 'dashboard'),
    'dashboard'
  )
})
test('the legacy JSON and reward bookmarks offer the business-specific actions', () => {
  const raw = render('operations', 'raw-json')
  assert.match(raw, /请选择要调整的规则/)
  assert.match(raw, /href="\/channels\?view=group-pricing"/)
  assert.match(
    raw,
    /href="\/channels\?view=model-pricing&amp;mode=upstream_cost"/
  )
  const quota = render('billing', 'quota')
  assert.match(quota, /注册赠额/)
  assert.match(quota, /href="\/redemption-codes"/)
})
test('missing and unknown sections explain the bookmark without mounting a form, linking to a fake destination or accepting arbitrary markup', () => {
  assert.match(render('models', 'claude'), /Claude 请求头/)
  const unknown = resolveLegacySettingsBookmark('site', '<wrong>', () => {
    throw new Error('unknown section must not fall back')
  })
  assert.equal(unknown.destination, null)
  const html = render('site', '<wrong>')
  assert.match(html, /没有与此书签匹配/)
  assert.match(html, /&lt;wrong&gt;/)
  assert.doesNotMatch(html, /<form|href=|<wrong>/)
})
