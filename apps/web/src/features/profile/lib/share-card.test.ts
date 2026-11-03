/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { profileSharePostText, profileShareXIntent } from './share-card'

test('share text and X intent preserve the configured site link', () => {
  const siteUrl = 'https://relay.test:8443/gateway'
  const message = 'My AI activity on Custom Relay'
  assert.equal(
    profileSharePostText(` ${message} `, siteUrl),
    `${message}\n${siteUrl}`
  )
  const intent = new URL(profileShareXIntent(message, siteUrl))
  assert.equal(intent.origin, 'https://twitter.com')
  assert.equal(intent.searchParams.get('text'), message)
  assert.equal(intent.searchParams.get('url'), siteUrl)
})

test('unavailable public address never inserts the official site', () => {
  assert.equal(profileSharePostText('My activity', null), 'My activity')
  assert.equal(
    new URL(profileShareXIntent('My activity', null)).searchParams.has('url'),
    false
  )
})
