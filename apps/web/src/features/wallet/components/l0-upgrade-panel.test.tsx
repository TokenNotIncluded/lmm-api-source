/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { createInstance } from 'i18next'
import { I18nextProvider } from 'react-i18next'
import { L0UpgradePanel } from './l0-upgrade-panel'

test('L0 preview is renderable without payment clients or router and disables both actions', async () => {
  const i18n = createInstance()
  await i18n.init({ lng: 'en', resources: { en: { translation: {} } } })
  const html = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <L0UpgradePanel preview />
    </I18nextProvider>
  )
  assert.equal((html.match(/ disabled=""/g) ?? []).length, 2)
  assert.ok(html.includes('Your next idea starts here'))
  assert.ok(html.includes('does not bypass access rules'))
  assert.ok(!html.includes('href='))
})
