/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToString } from 'react-dom/server'
import { I18nextProvider, initReactI18next } from 'react-i18next'

import { RemoteQuestionCard } from './controls'

const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
for (const kind of ['input', 'confirm', 'select', 'custom'] as const) {
  for (const state of ['live', 'expired', 'none'] as const) {
    test(`SSR ${kind} question uses the deadline snapshot for ${state}`, () => {
      const realNow = Date.now
      const questionDeadline =
        state === 'none'
          ? undefined
          : realNow() + (state === 'live' ? 60_000 : -60_000)
      Date.now = () => {
        throw new Error('SSR must not read the wall clock')
      }
      let markup: string
      try {
        markup = renderToString(
          <I18nextProvider i18n={i18n}>
            <RemoteQuestionCard
              question={{
                request_id: 'ssr-question',
                kind,
                question: 'Question',
                options: ['Option'],
                expires_at: questionDeadline,
              }}
              disabled={false}
              send={async () => {
                throw new Error('SSR must not send')
              }}
            />
          </I18nextProvider>
        )
      } finally {
        Date.now = realNow
      }
      assert.ok(markup.includes('Pi needs your answer'))
      const control =
        kind === 'custom'
          ? markup.match(/<button[^>]*aria-label="up"[^>]*>/)?.[0]
          : markup.match(/<button\b[^>]*>/)?.[0]
      assert.ok(control)
      assert.equal(/disabled=""/.test(control), false)
    })
  }
}
