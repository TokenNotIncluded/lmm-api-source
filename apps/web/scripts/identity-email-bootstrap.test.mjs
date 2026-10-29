/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import vm from 'node:vm'

const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8')
const bootstrap = html.match(
  /<script id="identity-email-bootstrap">([\s\S]*?)<\/script>/
)[1]
const tracker = html
  .match(/<script>\s*\/\/ Also fail closed[\s\S]*?<\/script>/)[0]
  .replaceAll(/<\/?script>/g, '')
const token = '0123456789abcdef0123456789abcdef'
const email = 'synthetic+reset@example.test'
const query = new URLSearchParams({ email, token }).toString()

function page(path, runBootstrap = true) {
  const location = new URL(`https://console.example.test${path}`)
  const elements = []
  const history = {
    state: { preserved: true },
    replaceState: (state, _title, value) => {
      assert.equal(state, history.state)
      location.href = new URL(value, location).href
    },
  }
  const window = { location, history }
  const document = {
    createElement: (tag) => ({ tag, dataset: {} }),
    head: { appendChild: (element) => elements.push(element) },
  }
  const context = vm.createContext({
    window,
    document,
    location,
    URLSearchParams,
  })
  if (runBootstrap) vm.runInContext(bootstrap, context)
  vm.runInContext(tracker, context)
  return { window, elements }
}

test('historical reset links capture once, strip query/hash and block third parties before resources', () => {
  for (const path of ['/reset', '/user/reset', '/reset/', '/user/r%65set']) {
    const { window, elements } = page(`${path}?${query}#private-fragment`)
    assert.equal(window.location.search, '')
    assert.equal(window.location.hash, '')
    assert.equal(window.location.pathname, path)
    assert.equal(window.__lmmEmailReset.email, email)
    assert.equal(window.__lmmEmailReset.token, token)
    assert.equal(
      Object.getOwnPropertyDescriptor(window, '__lmmEmailReset').enumerable,
      false
    )
    assert.equal(elements.filter((el) => el.tag === 'script').length, 0)
    assert.equal(
      elements.find((el) => el.name === 'referrer')?.content,
      'no-referrer'
    )
    assert.match(
      elements.find((el) => el.httpEquiv === 'Content-Security-Policy')
        ?.content,
      /connect-src 'self';/
    )
    delete window.__lmmEmailReset
    assert.equal(window.__lmmEmailReset, undefined)
    assert.equal(page(path).window.__lmmEmailReset, undefined)
  }
  assert.ok(
    html.indexOf('id="identity-email-bootstrap"') <
      html.indexOf('src="/extore-callback.js"')
  )
  assert.ok(
    html.indexOf('src="/extore-callback.js"') < html.indexOf('rel="icon"')
  )
  assert.doesNotMatch(
    bootstrap,
    /localStorage|sessionStorage|fetch\(|sendBeacon/
  )
})

test('formal fragment links survive early cleanup without putting proofs in requests', () => {
  for (const path of ['/reset', '/user/reset', '/user/reset/']) {
    const { window, elements } = page(`${path}#${query}`)
    assert.equal(window.location.search, '')
    assert.equal(window.location.hash, '')
    assert.equal(window.__lmmEmailReset.email, email)
    assert.equal(window.__lmmEmailReset.token, token)
    assert.equal(elements.filter((el) => el.tag === 'script').length, 0)
  }
})

test('mixed, duplicate and incomplete fragment proofs are cleared and rejected', () => {
  for (const suffix of [
    `?${query}#${query}`,
    `?email=${email}#token=${token}`,
    `?token=${token}#email=${email}`,
    `#${query}&token=${token}`,
    `#${query}&email=other@example.test`,
    `#email=${email}`,
    `#token=${token}`,
    `#email=${email}&token=short`,
  ]) {
    const { window } = page(`/user/reset${suffix}`)
    assert.equal(window.__lmmEmailReset, undefined)
    assert.equal(window.location.search, '')
    assert.equal(window.location.hash, '')
  }
})

test('missing, duplicate or malformed credentials never produce a usable reset record', () => {
  for (const query of [
    '',
    `email=${email}`,
    `email=${email}&token=`,
    `email=${email}&token=09ab3f`,
    `email=${email}&token=${token}&token=${token}`,
    `email=${email}&email=other@example.test&token=${token}`,
  ]) {
    const { window, elements } = page(`/user/reset?${query}`)
    assert.equal(window.__lmmEmailReset, undefined)
    assert.equal(window.location.search, '')
    assert.equal(elements.filter((el) => el.tag === 'script').length, 0)
  }
})

test('bootstrap failure still excludes tracking on both reset routes', () => {
  for (const path of ['/reset', '/user/reset', '/reset/', '/r%65set']) {
    assert.equal(
      page(`${path}?${query}`, false).elements.filter(
        (el) => el.tag === 'script'
      ).length,
      0
    )
  }
})

test('ordinary non-auth pages retain their URL and analytics without reset CSP', () => {
  for (const path of ['/', '/pricing?tab=models', '/store', '/games/signal']) {
    const { window, elements } = page(path)
    assert.equal(window.location.href, `https://console.example.test${path}`)
    assert.equal(elements.filter((el) => el.tag === 'script').length, 1)
    assert.equal(
      elements.some((el) => el.httpEquiv === 'Content-Security-Policy'),
      false
    )
  }
})
