/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { NavGroup } from '@/components/layout/types'
import type { PluginInfo } from '@/features/plugins/api'

import {
  applyRuntimeNavigation,
  capabilityPhase,
  moduleNavigationState,
} from './runtime-capabilities'

const now = 100_000
const plugin: PluginInfo = {
  name: 'toolmarket',
  version: '1.0.0',
  api_version: 1,
  registration_sha256: 'a'.repeat(64),
  available: true,
  installed: true,
  enabled: true,
  navigation_paths: ['/tool-market'],
}
const fresh = { data: [plugin], dataUpdatedAt: now }

test('capability evidence expires without turning a failed read into permission', () => {
  assert.equal(
    capabilityPhase({ data: undefined, dataUpdatedAt: 0 }, now),
    'pending'
  )
  assert.equal(
    capabilityPhase({ data: undefined, dataUpdatedAt: 0, isError: true }, now),
    'error'
  )
  assert.equal(capabilityPhase(fresh, now + 59_999), 'fresh')
  assert.equal(capabilityPhase(fresh, now + 60_000), 'stale')
  assert.equal(
    capabilityPhase({ ...fresh, error: new Error('503') }, now),
    'stale'
  )
  assert.equal(capabilityPhase(fresh, now - 1), 'stale')
})

test('module navigation requires every declared dependency and leaves Core routes alone', () => {
  assert.equal(
    moduleNavigationState('/tool-market', fresh, now),
    'reported_enabled'
  )
  assert.equal(
    moduleNavigationState(
      '/tool-market',
      {
        ...fresh,
        data: [
          { ...plugin, available: false, installed: false, enabled: false },
        ],
      },
      now
    ),
    'unavailable'
  )
  assert.equal(
    moduleNavigationState('/tool-market', fresh, now + 60_000),
    'unknown'
  )
  assert.equal(
    moduleNavigationState(
      '/tool-market',
      {
        ...fresh,
        data: [{ ...plugin, enabled: false }],
      },
      now
    ),
    'unavailable'
  )
  assert.equal(
    moduleNavigationState('/users', { data: undefined, dataUpdatedAt: 0 }, now),
    'unaffected'
  )
  assert.equal(
    moduleNavigationState(
      '/support',
      {
        ...fresh,
        data: [
          {
            ...plugin,
            name: 'support',
            navigation_paths: ['/support'],
            requires: ['store'],
          },
        ],
      },
      now
    ),
    'unavailable'
  )
})

test('navigation follows new host paths and transitive dependencies without a client module map', () => {
  const records: PluginInfo[] = [
    {
      ...plugin,
      name: 'reports',
      navigation_paths: ['/reports'],
      requires: ['records'],
    },
    {
      ...plugin,
      name: 'records',
      navigation_paths: [],
      requires: ['identity'],
    },
    { ...plugin, name: 'identity', navigation_paths: [] },
  ]
  const evidence = { ...fresh, data: records }
  assert.equal(
    moduleNavigationState('/reports/weekly', evidence, now),
    'reported_enabled'
  )
  assert.equal(
    moduleNavigationState('/reports-other', evidence, now),
    'unaffected'
  )
  assert.equal(
    moduleNavigationState('/reports', { ...evidence, isError: true }, now),
    'unknown'
  )
  assert.equal(
    moduleNavigationState(
      '/reports',
      {
        ...evidence,
        data: records.map((entry) =>
          entry.name === 'identity' ? { ...entry, enabled: false } : entry
        ),
      },
      now
    ),
    'unavailable'
  )
  assert.equal(
    moduleNavigationState(
      '/reports',
      { ...evidence, data: records.slice(0, 2) },
      now
    ),
    'unavailable'
  )
})

test('undeclared routes remain unaffected and shared ownership requires every owner', () => {
  assert.equal(
    moduleNavigationState(
      '/tool-market',
      { data: undefined, dataUpdatedAt: 0 },
      now
    ),
    'unaffected'
  )
  assert.equal(
    moduleNavigationState(
      '/tool-market',
      { ...fresh, data: [{ ...plugin, navigation_paths: undefined }] },
      now
    ),
    'unaffected'
  )
  assert.equal(
    moduleNavigationState('/tool-market', { ...fresh, data: [] }, now),
    'unaffected'
  )
  assert.equal(
    moduleNavigationState(
      '/tool-market',
      {
        ...fresh,
        data: [plugin, { ...plugin, name: 'moderation', enabled: false }],
      },
      now
    ),
    'unavailable'
  )
})

test('runtime declarations only narrow existing navigation and cannot grant a disabled action', () => {
  const groups: NavGroup[] = [
    {
      title: 'Existing roles',
      items: [
        {
          title: 'Tools',
          url: '/tool-market',
          disabled: true,
          requiredRole: 100,
        },
        { title: 'Users', url: '/users' },
      ],
    },
  ]
  const labels = { unknown: 'unknown', unavailable: 'unavailable' }
  assert.equal(
    applyRuntimeNavigation(groups, fresh, labels, now)[0].items[0].disabled,
    true
  )
  const narrowed = applyRuntimeNavigation(groups, fresh, labels, now + 60_000)
  assert.equal(narrowed[0].items[0].badge, 'unknown')
  assert.equal(narrowed[0].items[0].requiredRole, 100)
  assert.equal(narrowed[0].items[1].disabled, undefined)
  assert.equal(groups[0].items[0].badge, undefined)
})
