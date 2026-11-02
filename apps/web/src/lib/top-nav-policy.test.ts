/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  applyHeaderNavFormValues,
  parseHeaderNavModules as parseAdmin,
  serializeHeaderNavModules,
  toHeaderNavFormValues,
} from '../features/system-settings/maintenance/config'
import {
  cloneHeaderNavDefaults,
  parseHeaderNavModules,
} from './header-nav-config'
import { buildTopNavLinks, type TopNavSurface } from './top-nav-policy'

const extraKeys = [
  'directory',
  'developers',
  'guide',
  'scripts',
  'webmcp',
  'signal',
  'challenges',
] as const
const extraRoutes = [
  '/ai-directory',
  '/developers',
  '/guide',
  '/scripts',
  '/webmcp',
  '/games/signal',
  '/challenges',
]
const makeLinks = (
  raw: unknown,
  surface: TopNavSurface = 'default',
  overrides: Partial<Parameters<typeof buildTopNavLinks>[0]> = {}
) =>
  buildTopNavLinks({
    modules: parseHeaderNavModules(raw),
    surface,
    isAuthed: false,
    workspaceHref: '/wallet',
    hasDocsAccess: false,
    translate: (key) => key,
    ...overrides,
  })
const hrefs = (links: ReturnType<typeof makeLinks>) =>
  links.map((link) => link.href)

test('both surfaces retain their established route order with legacy/missing config', () => {
  for (const raw of [
    undefined,
    null,
    '',
    '{}',
    { home: true },
    'invalid',
    [],
  ]) {
    assert.deepEqual(hrefs(makeLinks(raw)), [
      '/',
      '/pricing',
      '/scripts',
      '/developers',
      '/webmcp',
      '/games/signal',
      '/rankings',
      '/security',
      '/about',
    ])
    assert.deepEqual(hrefs(makeLinks(raw, 'public')), [
      '/',
      '/ai-directory',
      '/pricing',
      '/developers',
      '/guide',
      '/scripts',
      '/webmcp',
      '/games/signal',
      '/challenges',
      '/security',
    ])
  }
})

test('each extra switch honors false in object/JSON and numeric/string legacy forms', () => {
  for (const disabled of [false, 0, '0', 'false', ' FALSE ']) {
    for (const [index, key] of extraKeys.entries()) {
      for (const raw of [
        { [key]: disabled },
        JSON.stringify({ [key]: disabled }),
      ]) {
        assert.equal(parseHeaderNavModules(raw)[key], false)
        assert.equal(parseAdmin(raw)[key], false)
        for (const surface of ['default', 'public'] as const) {
          assert.ok(
            !hrefs(makeLinks(raw, surface)).includes(extraRoutes[index])
          )
        }
      }
    }
  }
  for (const enabled of [true, 1, '1', 'true']) {
    assert.ok(
      extraKeys.every(
        (key) => parseHeaderNavModules({ [key]: enabled })[key] === true
      )
    )
  }
})

test('all disabled produces an empty list on both surfaces without a default resurrection', () => {
  const raw = Object.fromEntries(
    Object.keys(cloneHeaderNavDefaults()).map((key) => [key, false])
  )
  for (const surface of ['default', 'public'] as const) {
    assert.deepEqual(
      makeLinks(raw, surface, {
        isAuthed: true,
        hasDocsAccess: true,
        docsLink: 'https://docs.example.test',
      }),
      []
    )
    const saved = serializeHeaderNavModules(
      applyHeaderNavFormValues(
        parseAdmin(raw),
        toHeaderNavFormValues(parseAdmin(raw))
      )
    )
    assert.deepEqual(makeLinks(saved, surface), [])
  }
})

test('pricing/security visibility and auth flags survive both surfaces and admin roundtrip', () => {
  for (const surface of ['default', 'public'] as const) {
    const raw = {
      pricing: { enabled: 1, requireAuth: '1' },
      security: { enabled: 'true', requireAuth: true },
    }
    const config = parseAdmin(raw)
    const saved = serializeHeaderNavModules(
      applyHeaderNavFormValues(config, toHeaderNavFormValues(config))
    )
    for (const isAuthed of [true, false]) {
      const links = makeLinks(saved, surface, { isAuthed })
      for (const href of ['/pricing', '/security']) {
        assert.equal(
          links.find((link) => link.href === href)?.requiresAuth,
          !isAuthed
        )
      }
    }
    const hidden = makeLinks({ pricing: 0, security: 'false' }, surface)
    assert.ok(
      !hidden.some((link) => ['/pricing', '/security'].includes(link.href))
    )
  }
  assert.equal(
    makeLinks({ rankings: { enabled: 1, requireAuth: 1 } }).find(
      (link) => link.href === '/rankings'
    )?.requiresAuth,
    true
  )
})

test('default workspace uses the supplied account-aware landing route and stays after Home', () => {
  for (const workspaceHref of [
    '/wallet',
    '/dashboard',
    '/channels',
    '/system-info',
  ]) {
    const links = makeLinks({}, 'default', { isAuthed: true, workspaceHref })
    assert.equal(links[1].href, workspaceHref)
    assert.equal(links[1].title, 'Open workspace')
    assert.ok(
      !makeLinks({ console: false }, 'default', {
        isAuthed: true,
        workspaceHref,
      }).some((link) => link.title === 'Open workspace')
    )
    assert.ok(
      !makeLinks({}, 'public', { isAuthed: true, workspaceHref }).some(
        (link) => link.title === 'Open workspace'
      )
    )
  }
  assert.ok(!makeLinks({}).some((link) => link.title === 'Open workspace'))
})

test('docs requires explicit rights, enabled config and a server-provided URL', () => {
  for (const hasDocsAccess of [true, false]) {
    for (const docs of [true, false]) {
      for (const docsLink of [
        undefined,
        '',
        'https://docs.example.test/paid',
      ]) {
        const links = makeLinks({ docs }, 'default', {
          isAuthed: true,
          hasDocsAccess,
          docsLink,
        })
        const actual = links.find((link) => link.title === 'Docs')
        assert.equal(!!actual, hasDocsAccess && docs && !!docsLink)
        if (actual) {
          assert.equal(actual.href, docsLink)
          assert.equal(actual.external, true)
          assert.equal(links.at(-2), actual)
        }
        assert.ok(
          !makeLinks({ docs }, 'public', {
            isAuthed: true,
            hasDocsAccess,
            docsLink,
          }).some((link) => link.title === 'Docs')
        )
      }
    }
  }
})

test('form edits preserve all unknown scalar/object/array/null fields and nested access metadata', () => {
  const raw = {
    directory: false,
    scripts: 0,
    security: {
      enabled: false,
      requireAuth: true,
      future: { groups: ['team'] },
    },
    pricing: { enabled: true, requireAuth: true, caption: 'keep' },
    unknownObject: { enabled: false, config: ['x', 3] },
    unknownArray: [false],
    unknownString: 'false',
    unknownNumber: 7,
    unknownNull: null,
    unknownBoolean: false,
  }
  for (const input of [raw, JSON.stringify(raw)]) {
    const parsed = parseAdmin(input)
    const defaults = toHeaderNavFormValues(parsed)
    assert.equal(defaults.directory, false)
    assert.equal(defaults.scripts, false)
    const changed = applyHeaderNavFormValues(parsed, {
      ...defaults,
      developers: false,
      securityEnabled: true,
    })
    const roundtrip = parseAdmin(serializeHeaderNavModules(changed))
    for (const key of Object.keys(raw).filter((key) =>
      key.startsWith('unknown')
    )) {
      assert.deepEqual(roundtrip[key], raw[key as keyof typeof raw])
    }
    assert.deepEqual(roundtrip.security, { ...raw.security, enabled: true })
    assert.deepEqual(roundtrip.pricing, raw.pricing)
    assert.equal(roundtrip.developers, false)
    assert.equal(roundtrip.directory, false)
    assert.equal(parsed.security.enabled, false)
    assert.equal(parsed.developers, true)
    // Reset replaces known form switches, preserving future extension fields.
    const reset = applyHeaderNavFormValues(
      parsed,
      toHeaderNavFormValues(cloneHeaderNavDefaults())
    )
    assert.equal(reset.directory, true)
    assert.deepEqual(reset.unknownObject, raw.unknownObject)
    assert.deepEqual(reset.security.future, raw.security.future)
  }
})

test('invalid access objects use defaults and ordinary legacy keys still decode', () => {
  assert.deepEqual(parseHeaderNavModules({ security: [] }).security, {
    enabled: true,
    requireAuth: false,
  })
  const parsed = parseHeaderNavModules({
    home: '0',
    console: 0,
    docs: 'false',
    about: false,
    rankings: false,
    directory: 2,
  })
  assert.deepEqual(
    [
      parsed.home,
      parsed.console,
      parsed.docs,
      parsed.about,
      parsed.rankings.enabled,
    ],
    [false, false, false, false, false]
  )
  assert.equal(parsed.directory, true)
  assert.deepEqual(parsed, parseAdmin(parsed))
})

test('translation keys are translated once and WebMCP stays a brand label', () => {
  const links = makeLinks({}, 'public', {
    translate: (key) => `translated:${key}`,
  })
  assert.equal(links[1].title, 'translated:AI directory')
  assert.equal(links.find((link) => link.href === '/webmcp')?.title, 'WebMCP')
})
