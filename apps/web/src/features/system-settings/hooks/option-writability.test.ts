import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { SystemOptionsCapabilities } from '../types'
import {
  isOptionWritable,
  settingsUseRestrictedView,
  writableOptionKey,
} from './option-writability'

const capabilities: SystemOptionsCapabilities = {
  read_only: false,
  partial: false,
  writable_keys: ['Notice', 'Announcements', 'console_setting.announcements'],
}

test('writable keys restrict writes even when the whole settings response is not read-only', () => {
  assert.equal(settingsUseRestrictedView(capabilities), true)
  assert.equal(isOptionWritable(capabilities, 'Notice'), true)
  assert.equal(
    writableOptionKey(capabilities, 'console_setting.announcements'),
    'console_setting.announcements'
  )
  assert.equal(writableOptionKey(capabilities, 'AssistantPersona'), undefined)
  assert.equal(
    writableOptionKey(
      { ...capabilities, writable_keys: ['ServerAddress'] },
      'ServerAddress'
    ),
    undefined
  )
  assert.equal(
    writableOptionKey({ read_only: false, partial: false }, 'Notice'),
    undefined
  )
})

test('announcement aliases use the key approved by the server', () => {
  assert.equal(
    writableOptionKey(
      { ...capabilities, writable_keys: ['Announcements'] },
      'console_setting.announcements'
    ),
    'Announcements'
  )
})

test('navigation is writable only when the server advertises it', () => {
  assert.equal(writableOptionKey(capabilities, 'HeaderNavModules'), undefined)
  const navigation = { ...capabilities, writable_keys: ['HeaderNavModules'] }
  assert.equal(
    writableOptionKey(navigation, 'HeaderNavModules'),
    'HeaderNavModules'
  )
  assert.equal(
    writableOptionKey({ ...navigation, read_only: true }, 'HeaderNavModules'),
    undefined
  )
})
