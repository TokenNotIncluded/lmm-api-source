/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

const source = readFileSync(
  new URL('./use-sidebar-data.ts', import.meta.url),
  'utf8'
)

describe('authenticated sidebar discovery', () => {
  test('one console navigation serves every signed-in user', () => {
    assert.doesNotMatch(source, /id: 'onboarding'/)
    for (const path of ['/keys', '/developers', '/usage-logs/common', '/wallet']) {
      assert.ok(source.includes(`url: '${path}'`))
    }
    assert.doesNotMatch(source, /remote-control|Remote control/)
  })

  test('exposes the reset workspace only as a root navigation item', () => {
    const resetStart = source.indexOf("title: t('Subscription reset')")
    const resetSection =
      resetStart >= 0 ? source.slice(resetStart, resetStart + 180) : ''

    assert.ok(resetSection)
    assert.match(resetSection, /url: '\/subscriptions\/reset'/)
    assert.match(resetSection, /requiredRole: ROLE\.SUPER_ADMIN/)
  })

  test('keeps the model square visible without making it navigable', () => {
    const generalStart = source.indexOf("id: 'general'")
    const personalStart = source.indexOf("id: 'personal'", generalStart)
    const activatedGeneralSection =
      generalStart >= 0 && personalStart > generalStart
        ? source.slice(generalStart, personalStart)
        : ''

    assert.ok(activatedGeneralSection)
    assert.match(activatedGeneralSection, /title: t\('Models and pricing'\)/)
    assert.match(activatedGeneralSection, /url: '\/pricing'/)
    assert.match(activatedGeneralSection, /interaction: 'model-panel'/)
  })

  test('separates developer tools, ecosystem, account and standalone services', () => {
    const activated = source.slice(source.indexOf("id: 'general'"))
    const section = (id: string, next: string) =>
      activated.slice(
        activated.indexOf(`id: '${id}'`),
        activated.indexOf(`id: '${next}'`)
      )
    assert.match(section('general', 'developer'), /title: t\('Use AI'\)/)
    assert.doesNotMatch(section('general', 'developer'), /getting-started/)
    assert.match(section('general', 'developer'), /url: '\/chat-management'/)
    assert.match(section('developer', 'forge'), /url: '\/keys'/)
    assert.match(section('developer', 'forge'), /url: '\/developers'/)
    assert.doesNotMatch(
      section('developer', 'forge'),
      /\/guide|Task Logs|\/usage-logs\/(?:task|drawing)/
    )
    assert.match(section('forge', 'personal'), /url: '\/scripts'/)
    assert.match(section('personal', 'services'), /url: '\/wallet'/)
    assert.doesNotMatch(
      section('personal', 'services'),
      /temporary-activations/
    )
    assert.match(section('services', 'admin'), /url: '\/temporary-activations'/)
    assert.doesNotMatch(
      activated.slice(0, activated.indexOf("id: 'admin'")),
      /url: '\/(users|operations|channels)/
    )
  })
})
