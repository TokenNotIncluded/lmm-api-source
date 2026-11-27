/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, test } from 'node:test'

import type { NavGroup, NavLink } from '../types'
import {
  findConsoleNavItem,
  getConsolePageTitle,
  getConsoleSiteLinks,
  organizeConsoleNavigation,
} from './console-navigation'
import { checkIsActive } from './url-utils'

const overview: NavLink = { title: 'Overview', url: '/dashboard/overview' }
const models: NavLink = {
  title: 'Models and pricing',
  url: '/pricing',
  interaction: 'model-panel',
}
const keys: NavLink = { title: 'API Keys', url: '/keys' }
const wallet: NavLink = { title: 'Wallet', url: '/wallet', badge: '2' }
const drawing: NavLink = { title: 'Drawing studio', url: '/drawing' }
const groups: NavGroup[] = [
  { id: 'general', title: 'Use AI', items: [drawing, models, overview] },
  { id: 'developer', title: 'Developers', items: [keys] },
  { id: 'personal', title: 'Account', items: [wallet] },
]

describe('console navigation organization', () => {
  test('promotes frequent tasks in a stable order without cloning entries', () => {
    const result = organizeConsoleNavigation(groups, 'Console')
    assert.equal(result[0].id, 'console-primary')
    assert.deepEqual(result[0].items, [overview, models, keys, wallet])
    assert.equal(result[0].items[1], models)
    assert.equal(result[0].items[3], wallet)
    assert.deepEqual(result[1].items, [drawing])
  })

  test('does not restore role- or module-filtered destinations', () => {
    const result = organizeConsoleNavigation(
      [{ id: 'general', title: 'AI', items: [drawing] }],
      'Console'
    )
    assert.deepEqual(
      result.flatMap((group) => group.items),
      [drawing]
    )
    assert.equal(
      result.some((group) => group.id === 'console-primary'),
      false
    )
  })

  test('preserves every entry exactly once and never mutates source groups', () => {
    const original = JSON.stringify(groups)
    const result = organizeConsoleNavigation(groups, 'Console')
    const before = groups.flatMap((group) => group.items)
    const after = result.flatMap((group) => group.items)
    assert.equal(after.length, before.length)
    for (const item of before) {
      assert.equal(after.filter((x) => x === item).length, 1)
    }
    assert.equal(JSON.stringify(groups), original)
  })

  test('retains nested admin menus, role requirements and badges', () => {
    const admin: NavGroup = {
      id: 'admin',
      title: 'Admin',
      items: [{ title: 'Operations', requiredRole: 10, items: [wallet] }],
    }
    assert.deepEqual(organizeConsoleNavigation([admin], 'Console'), [admin])
  })

  test('keeps daily administration visible without restoring filtered links', () => {
    const channels: NavLink = { title: 'Channels', url: '/channels' }
    const users: NavLink = { title: 'Users', url: '/users', badge: '3' }
    const settings: NavLink = {
      title: 'System Settings',
      url: '/system-settings/site',
      activeUrls: ['/system-settings'],
      requiredRole: 100,
    }
    const plugins: NavLink = {
      title: 'Plugin management',
      url: '/extensions/plugins',
    }
    const admin: NavGroup = {
      id: 'admin',
      title: 'Admin',
      items: [plugins, settings, users, channels],
    }
    const result = organizeConsoleNavigation(
      [...groups, admin],
      'Console',
      'More admin tools'
    )
    const visible = result.find((group) => group.id === 'console-management')
    assert.deepEqual(visible?.items, [channels, users, settings])
    assert.equal(visible?.items[1], users)
    assert.equal(
      result.find((group) => group.id === 'admin')?.title,
      'More admin tools'
    )
    assert.deepEqual(result.find((group) => group.id === 'admin')?.items, [
      plugins,
    ])
    assert.equal(result.flatMap((group) => group.items).length, 9)
    const filtered = organizeConsoleNavigation(
      [{ ...admin, items: [channels] }],
      'Console',
      'More admin tools'
    )
    assert.deepEqual(
      filtered.flatMap((group) => group.items),
      [channels]
    )
    assert.equal(
      filtered.some((group) => group.id === 'admin'),
      false
    )
    assert.deepEqual(admin.items, [plugins, settings, users, channels])
  })

  test('leaves onboarding untouched and omits empty groups', () => {
    const onboarding = [{ ...groups[0], id: 'onboarding' }]
    assert.equal(organizeConsoleNavigation(onboarding, 'Console'), onboarding)
    assert.deepEqual(organizeConsoleNavigation([], 'Console'), [])
    assert.deepEqual(
      organizeConsoleNavigation(
        [{ id: 'empty', title: '', items: [] }],
        'Console'
      ),
      []
    )
  })
})

describe('current location and secondary site links', () => {
  test('resolves query strings, detail routes and trailing slashes', () => {
    assert.equal(getConsolePageTitle(groups, '/keys/?page=2#list'), 'API Keys')
    assert.equal(getConsolePageTitle(groups, '/keys/123'), 'API Keys')
    assert.equal(getConsolePageTitle(groups, '/keys-other'), undefined)
  })

  test('prefers the specific destination and supports aliases and nested items', () => {
    const nested: NavGroup[] = [
      {
        title: 'Admin',
        items: [
          { title: 'Subscriptions', url: '/subscriptions' },
          { title: 'Reset', url: '/subscriptions/reset' },
          {
            title: 'Logs',
            items: [
              {
                title: 'Task Logs',
                url: '/usage-logs/task',
                activeUrls: ['/usage-logs/drawing'],
              },
            ],
          },
        ],
      },
    ]
    assert.equal(getConsolePageTitle(nested, '/subscriptions/reset'), 'Reset')
    assert.equal(
      getConsolePageTitle(nested, '/usage-logs/drawing'),
      'Task Logs'
    )
  })

  test('uses localized child titles for recently visited settings', () => {
    const settings: NavGroup[] = [
      {
        title: '设置',
        items: [
          {
            title: '站点与品牌',
            items: [
              { title: '系统公告', url: '/system-settings/site/notice' },
              {
                title: '侧边栏模块',
                url: '/system-settings/site/sidebar-modules',
              },
            ],
          },
        ],
      },
    ]
    assert.equal(
      findConsoleNavItem(settings, '/system-settings/site/notice')?.title,
      '系统公告'
    )
    assert.equal(
      findConsoleNavItem(settings, '/system-settings/site/sidebar-modules/')
        ?.title,
      '侧边栏模块'
    )
    assert.equal(
      findConsoleNavItem([], '/system-settings/site/notice'),
      undefined
    )
  })

  test('ranks the matched alias instead of an unrelated primary URL', () => {
    const aliases: NavGroup[] = [
      {
        title: 'Console',
        items: [
          {
            title: 'Overview',
            url: '/very-long-overview-path',
            activeUrls: ['/dashboard'],
          },
          {
            title: 'Models',
            url: '/models',
            activeUrls: ['/dashboard/models'],
          },
        ],
      },
    ]
    assert.equal(
      getConsolePageTitle(aliases, '/dashboard/models?range=7d'),
      'Models'
    )
  })

  test('retains disabled parent and child states for recent navigation', () => {
    const disabled: NavGroup[] = [
      {
        title: 'Settings',
        items: [
          {
            title: 'Billing',
            disabled: true,
            items: [{ title: 'Payment', url: '/settings/payment' }],
          },
          { title: 'Keys', url: '/keys', disabled: true },
        ],
      },
    ]
    assert.equal(
      findConsoleNavItem(disabled, '/settings/payment')?.disabled,
      true
    )
    assert.equal(findConsoleNavItem(disabled, '/keys')?.disabled, true)
    assert.equal(disabled[0].items[0].items?.[0].disabled, undefined)
  })

  test('removes duplicate site links without dropping external/disabled semantics', () => {
    const external = {
      title: 'Docs',
      href: 'https://example.invalid/docs',
      external: true,
    }
    const disabled = {
      title: 'Keys unavailable',
      href: '/keys',
      disabled: true,
    }
    assert.deepEqual(
      getConsoleSiteLinks(
        [{ title: 'Keys', href: '/keys' }, external, external, disabled],
        groups
      ),
      [external, disabled]
    )
  })
})

test('settings leaves activate the corresponding root entry and location title', () => {
  const core = { url: '/system-settings', title: 'Core Settings' }
  const plugins = { url: '/extensions/plugins', title: 'Plugin Settings' }
  const navigation = [{ title: 'Admin', items: [core, plugins] }]
  for (const [path, active, inactive] of [
    ['/system-settings/billing/quota', core, plugins],
    ['/system-settings/billing/payment?tab=current', plugins, core],
    ['/system-settings/operations/scripts/', plugins, core],
    ['/extensions/settings/', plugins, core],
    ['/extensions/plugins?configure=rss', plugins, core],
  ] as const) {
    assert.equal(getConsolePageTitle(navigation, path), active.title)
    assert.equal(checkIsActive(path, active), true)
    assert.equal(checkIsActive(path, inactive), false)
  }
  assert.equal(getConsolePageTitle(navigation, '/keys'), undefined)
})

describe('single console navigation contract', () => {
  test('keeps the same sidebar shell and persistence on settings routes', () => {
    const source = readFileSync(
      new URL('../components/authenticated-layout.tsx', import.meta.url),
      'utf8'
    )
    assert.match(source, /showTopNav=\{false\}/)
    assert.match(source, /<AppSidebar \/>/)
    assert.match(source, /defaultOpen=\{defaultOpen\}/)
    assert.doesNotMatch(source, /settingsPage|persistState=/)
    const sidebar = readFileSync(
      new URL('../components/app-sidebar.tsx', import.meta.url),
      'utf8'
    )
    assert.match(sidebar, /const navGroups = useRootSidebarNavigation\(\)/)
    assert.doesNotMatch(sidebar, /FloatingSettingsNavigation|SidebarViewHeader/)
    assert.match(source, /showLanguageSwitcher=\{false\}/)
    assert.match(source, /showConfigDrawer=\{false\}/)
    assert.match(source, /consoleActivated && <ConsoleMobileDock \/>/)
    assert.doesNotMatch(source, /focusedOnboarding/)
  })
})

describe('console search matches visible navigation', () => {
  const source = readFileSync(
    new URL('../../command-menu.tsx', import.meta.url),
    'utf8'
  )

  test('uses the shared role/module-filtered sidebar view', () => {
    assert.match(source, /const navGroups = useRootSidebarNavigation\(\)/)
    assert.doesNotMatch(source, /useSidebarData|getNavGroupsForPath/)
  })

  test('keeps disabled destinations disabled and preserves model panels', () => {
    assert.match(source, /disabled=\{navItem.disabled\}/)
    assert.match(source, /disabled=\{navItem.disabled \|\| subItem.disabled\}/)
    assert.match(source, /navItem.interaction === 'model-panel'/)
    assert.match(source, /subItem.interaction === 'model-panel'/)
  })
})
