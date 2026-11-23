/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import vm from 'node:vm'

const bootstrap = readFileSync(
  new URL('../public/extore-callback.js', import.meta.url),
  'utf8'
)
const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8')
const scripts = [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)]
assert.ok(
  scripts.length > 0,
  'the actual page bootstrap scripts must be present'
)
function page(path, runBootstrap = true) {
  const location = new URL('https://shop.example' + path)
  const elements = []
  const document = {
    cookie: '',
    documentElement: { classList: { add: () => {} } },
    body: { dataset: {}, setAttribute: () => {} },
    querySelector: () => null,
    createElement: (tag) => ({ tag, dataset: {} }),
    head: { appendChild: (el) => elements.push(el) },
  }
  const history = {
    state: null,
    replaceState: (_s, _t, value) => {
      location.href = new URL(value, location).href
    },
  }
  const window = { location, history, matchMedia: () => ({ matches: false }) }
  const context = vm.createContext({
    window,
    location,
    document,
    URLSearchParams,
  })
  for (const [, attributes, code] of scripts) {
    const source = attributes.match(
      /\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))/i
    )
    if (source) {
      assert.equal(
        source[1] ?? source[2] ?? source[3],
        '/extore-callback.js',
        'the page must not introduce an unreviewed script source'
      )
      if (runBootstrap) vm.runInContext(bootstrap, context)
    } else {
      assert.doesNotMatch(attributes, /\bsrc\b/i)
      vm.runInContext(code, context)
    }
  }
  return { window, elements }
}
test('captures callbacks, strips history, and blocks external connections', () => {
  for (const query of [
    'code=fixture&state=fixture&iss=fixture',
    'error=access_denied&state=fixture',
    'code=one&code=two',
    'state=only',
  ]) {
    const { window, elements } = page('/store/manage?' + query)
    assert.equal(
      window.__lmmExtoreCallback,
      'https://shop.example/store/manage?' + query
    )
    assert.equal(window.location.href, 'https://shop.example/store/manage')
    assert.equal(
      elements.some((el) => el.tag === 'script'),
      false
    )
    assert.equal(
      elements.find((el) => el.name === 'referrer')?.content,
      'no-referrer'
    )
    assert.match(
      elements.find((el) => el.httpEquiv === 'Content-Security-Policy')
        ?.content,
      /connect-src 'self'/
    )
  }
})
test('ordinary pages and callbacks never inject tracking, including a missing bootstrap', () => {
  assert.equal(page('/').elements.filter((el) => el.tag === 'script').length, 0)
  assert.equal(
    page('/store/manage?code=fixture', false).elements.filter(
      (el) => el.tag === 'script'
    ).length,
    0
  )
  assert.ok(html.indexOf('/extore-callback.js') < html.indexOf('rel="icon"'))
  assert.doesNotMatch(bootstrap, /localStorage|sessionStorage|fetch\(/)
})
