/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import type { NavGroup } from '../types'

const dom = new Window({
  url: 'https://console.example.test/system-settings/site',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
let groups: NavGroup[] = []
const navigations: { to: string }[] = []
let panelOpens = 0
const navigate = (value: { to: string }) => {
  navigations.push(value)
}
const openPanel = () => {
  panelOpens++
}
mock.module('@tanstack/react-router', () => ({
  useNavigate: () => navigate,
  useLocation: ({
    select,
  }: {
    select: (location: { pathname: string }) => unknown
  }) => select({ pathname: '/system-settings/site' }),
}))
mock.module('@/hooks/use-sidebar-view', () => ({
  useRootSidebarNavigation: () => groups,
}))
mock.module('@/context/model-plaza-provider', () => ({
  useModelPlaza: () => ({ openPanel }),
}))
const { act, useEffect } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QuickSwitchProvider, useQuickSwitch } =
  await import('./quick-switch-provider')
let context: ReturnType<typeof useQuickSwitch>
function Probe() {
  const value = useQuickSwitch()
  useEffect(() => {
    context = value
  }, [value])
  return <input aria-label='Editor' />
}
after(() => {
  mock.restore()
  dom.close()
})

async function mount(items: NavGroup[0]['items']) {
  groups = [{ title: 'Console', items }]
  navigations.length = 0
  panelOpens = 0
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const render = () =>
    root.render(
      <QuickSwitchProvider>
        <Probe />
      </QuickSwitchProvider>
    )
  await act(async () => render())
  return {
    host,
    async update(next: NavGroup[0]['items']) {
      groups = [{ title: 'Console', items: next }]
      await act(async () => render())
    },
    async close() {
      await act(async () => root.unmount())
      host.remove()
    },
  }
}
function key(value: string, target: EventTarget = document) {
  target.dispatchEvent(
    new dom.KeyboardEvent('keydown', {
      key: value,
      bubbles: true,
      cancelable: true,
    })
  )
}
async function sequence(value: string, target: EventTarget = document) {
  await act(async () => {
    key('g', target)
    key(value, target)
  })
}

test('hidden and disabled destinations disappear from the sheet and cannot be dispatched by keys or clicks', async () => {
  const mounted = await mount([
    { title: 'Overview', url: '/dashboard/overview' },
    { title: 'Wallet', url: '/wallet', disabled: true },
  ])
  try {
    assert.equal(
      context.shortcuts.some(
        (entry) => entry.to === '/keys' || entry.to === '/wallet'
      ),
      false
    )
    await sequence('k')
    await sequence('w')
    assert.equal(context.runNavigationShortcut('/keys'), false)
    assert.equal(context.runNavigationShortcut('/wallet'), false)
    assert.deepEqual(navigations, [])
  } finally {
    await mounted.close()
  }
})

test('root shortcuts remain available from a nested settings workspace and pricing opens the existing panel', async () => {
  const mounted = await mount([
    { title: 'Overview', url: '/dashboard/overview' },
    {
      title: 'Models and pricing',
      url: '/pricing',
      interaction: 'model-panel',
    },
  ])
  try {
    await sequence('d')
    await sequence('p')
    assert.deepEqual(navigations, [{ to: '/dashboard/overview' }])
    assert.equal(panelOpens, 1)
    assert.equal(context.runNavigationShortcut('/pricing'), true)
    assert.equal(panelOpens, 2)
  } finally {
    await mounted.close()
  }
})

test('a visibility change between keys cannot dispatch a previously visible destination', async () => {
  const mounted = await mount([{ title: 'API Keys', url: '/keys' }])
  try {
    await act(async () => key('g'))
    await mounted.update([])
    await act(async () => key('k'))
    assert.deepEqual(navigations, [])
    await mounted.update([{ title: 'API Keys', url: '/keys' }])
    await sequence('k')
    assert.deepEqual(navigations, [{ to: '/keys' }])
  } finally {
    await mounted.close()
  }
})

test('typing, open dialogs and unmounted providers do not navigate', async () => {
  const mounted = await mount([{ title: 'API Keys', url: '/keys' }])
  try {
    const input = mounted.host.querySelector('input')
    assert.ok(input)
    await sequence('k', input)
    const dialog = document.createElement('div')
    dialog.setAttribute('role', 'dialog')
    document.body.append(dialog)
    await sequence('k')
    dialog.remove()
    assert.deepEqual(navigations, [])
  } finally {
    await mounted.close()
  }
  await sequence('k')
  assert.deepEqual(navigations, [])
})
