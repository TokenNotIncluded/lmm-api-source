/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

import { CONSOLE_SHORTCUTS } from '@/components/layout/lib/shortcuts'

test('navigation shortcuts target registered native routes directly', () => {
  const tree = readFileSync(
    new URL('../routeTree.gen.ts', import.meta.url),
    'utf8'
  )
  for (const shortcut of CONSOLE_SHORTCUTS) {
    const destination = shortcut.to
    if (destination) {
      const paths = [...tree.matchAll(/^ {2}'([^']+)': typeof /gm)].map(
        (match) => match[1]
      )
      assert.ok(
        paths.some((path) =>
          new RegExp(`^${path.replaceAll(/\$[^/]+/g, '[^/]+')}$`).test(
            destination
          )
        ),
        destination
      )
    }
  }
  assert.equal(
    CONSOLE_SHORTCUTS.find(({ id }) => id === 'Go to settings')?.to,
    '/profile'
  )
  assert.equal(
    CONSOLE_SHORTCUTS.find(({ id }) => id === 'Go to API keys')?.to,
    '/keys'
  )
})

test('generated routes retain native destinations and released bookmarks', () => {
  const tree = readFileSync(
    new URL('../routeTree.gen.ts', import.meta.url),
    'utf8'
  )
  for (const path of ['/keys', '/profile', '/api-keys', '/settings']) {
    assert.ok(tree.includes(`'${path}'`))
  }
  for (const path of [
    '/console',
    '/login',
    '/forbidden',
    '/developer-access',
  ]) {
    assert.ok(!tree.includes(`  '${path}': typeof `), path)
  }
})
