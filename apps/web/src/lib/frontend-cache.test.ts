/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import { initializeFrontendCache } from './frontend-cache'

const dom = new Window()
Object.defineProperty(globalThis, 'window', { configurable: true, value: dom })
after(() => dom.close())

test('UI cache rotation preserves pending checkout receipts', () => {
  dom.localStorage.setItem('newapi:default:cache-version', 'older-version')
  dom.localStorage.setItem('wallet-topup-cloud:7', '[{"tradeNo":"pending"}]')
  dom.localStorage.setItem('old-ui-cache', 'stale')
  initializeFrontendCache()
  assert.equal(
    dom.localStorage.getItem('wallet-topup-cloud:7'),
    '[{"tradeNo":"pending"}]'
  )
  assert.equal(dom.localStorage.getItem('old-ui-cache'), null)
})

for (const { name, cacheVersion, marker } of [
  {
    name: 'first-load initialization',
    cacheVersion: null,
    marker: 'pending:interrupted-guest-refresh',
  },
  {
    name: 'version rotation',
    cacheVersion: 'older-version',
    marker: 'uncertain:interrupted-sign-in',
  },
]) {
  test(`UI cache ${name} preserves browser-session recovery and login epochs`, () => {
    dom.localStorage.clear()
    if (cacheVersion) {
      dom.localStorage.setItem('newapi:default:cache-version', cacheVersion)
    }
    const protocolState = new Map([
      ['new-api:auth-cookie:uncertain', marker],
      ['new-api:auth-login-intent', '0d5322b0-7382-413c-bc55-ec2bb78ec6e4'],
      [
        'new-api:auth-login-initialization',
        '5b32b449-2802-426e-9854-607109ef1cad',
      ],
    ])
    for (const [key, value] of protocolState) {
      dom.localStorage.setItem(key, value)
    }
    dom.localStorage.setItem('old-ui-cache', 'stale')

    initializeFrontendCache()

    for (const [key, value] of protocolState) {
      assert.equal(dom.localStorage.getItem(key), value)
    }
    assert.equal(dom.localStorage.getItem('old-ui-cache'), null)
  })
}
