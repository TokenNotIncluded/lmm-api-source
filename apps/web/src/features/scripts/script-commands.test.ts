/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { commandFor, scriptUrl } from './script-commands'

test('script commands and downloads share the configured prefix', () => {
  const root = 'https://relay.test:8443/gateway'
  assert.equal(scriptUrl(root, 'menu.sh'), `${root}/scripts/menu.sh`)
  assert.equal(
    commandFor(root, 'menu.sh', 'linux'),
    `curl -fsSL '${root}/scripts/menu.sh' | bash`
  )
  assert.equal(
    commandFor(root, 'menu.zsh', 'macos'),
    `curl -fsSL '${root}/scripts/menu.zsh' | zsh`
  )
  assert.equal(
    commandFor(root, 'menu.ps1', 'windows'),
    `irm '${root}/scripts/menu.ps1' | iex`
  )
  assert.equal(commandFor(null, 'menu.sh', 'linux'), null)
  assert.equal(scriptUrl(null, 'menu.sh'), null)
})

test('shell metacharacters remain literal in copied commands', () => {
  const root = "https://relay.test/customer's-$name"
  assert.equal(
    commandFor(root, 'menu.sh', 'linux'),
    "curl -fsSL 'https://relay.test/customer'\\''s-$name/scripts/menu.sh' | bash"
  )
  assert.equal(
    commandFor(root, 'menu.ps1', 'windows'),
    "irm 'https://relay.test/customer''s-$name/scripts/menu.ps1' | iex"
  )
  assert.equal(
    commandFor('https://relay.test', "my'file.sh", 'windows'),
    "Invoke-WebRequest -Uri 'https://relay.test/scripts/my''file.sh' -OutFile 'my''file.sh'; & '.\\my''file.sh'"
  )
})
