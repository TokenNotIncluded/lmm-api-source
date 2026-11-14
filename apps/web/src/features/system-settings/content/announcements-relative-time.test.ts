/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { announcementRelativeTime } from './announcements-relative-time'

const now = Date.UTC(2026, 10, 15, 12)
for (const language of ['en', 'zh-CN', 'zh-TW', 'fr', 'ja', 'ru', 'vi']) {
  test(`announcement relative times use ${language}`, () => {
    const formatter = new Intl.RelativeTimeFormat(language, { numeric: 'auto' })
    for (const [amount, unit, milliseconds] of [
      [-5, 'minute', 60_000],
      [-2, 'hour', 3_600_000],
      [-3, 'day', 86_400_000],
      [5, 'minute', 60_000],
    ] as const) {
      const formatted = announcementRelativeTime(
        new Date(now + amount * milliseconds).toISOString(),
        now,
        language
      )
      assert.equal(formatted, formatter.format(amount, unit))
      if (language !== 'en') {
        assert.notEqual(
          formatted,
          new Intl.RelativeTimeFormat('en', { numeric: 'auto' }).format(
            amount,
            unit
          )
        )
      }
    }
    assert.equal(
      announcementRelativeTime('invalid date', now, language),
      'invalid date'
    )
  })
}
