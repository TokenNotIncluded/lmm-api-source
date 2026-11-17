/* Copyright (C) 2026 LIghtJUNction; AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/system-settings/content',
  settings: { device: { prefersReducedMotion: 'reduce' } },
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLButtonElement',
  'HTMLFormElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'DocumentFragment',
  'ShadowRoot',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const writes: { key: string; value: string | boolean }[] = []
let saveResult: () => Promise<{ success: boolean }> = async () => ({ success: true })
mock.module('../hooks/use-update-option', () => ({
  useUpdateOption: () => ({
    isPending: false,
    mutateAsync: async (request: { key: string; value: string | boolean }) => {
      writes.push(request)
      return saveResult()
    },
  }),
}))
const { act, Profiler } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { UptimeKumaSection } = await import('./uptime-kuma-section')
const { AnnouncementsSection } = await import('./announcements-section')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const roots: ReturnType<typeof createRoot>[] = []
afterEach(async () => {
  await act(async () => {
    for (const root of roots.splice(0)) root.unmount()
  })
  document.body.replaceChildren()
  writes.length = 0
  saveResult = async () => ({ success: true })
})
after(() => dom.close())
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
function button(label: string) {
  const result = Array.from(document.querySelectorAll('button')).find(
    (element) => element.textContent?.trim() === label
  )
  assert.ok(result, `Missing button ${label}`)
  return result
}
async function click(element: HTMLElement) {
  await act(async () => {
    element.click()
    await flush()
  })
}
function checkbox(index: number) {
  const result = document.querySelectorAll<HTMLElement>(
    '[data-slot="checkbox"]'
  )[index]
  assert.ok(result)
  return result
}
function enabledInput() {
  const label = Array.from(document.querySelectorAll('label')).find(
    (element) => element.textContent?.trim() === 'Enabled'
  )
  assert.ok(label)
  const input = document.getElementById(label.htmlFor)
  assert.ok(input instanceof HTMLInputElement)
  assert.ok(input)
  return input
}
const cases = [
  {
    name: 'uptime',
    Component: UptimeKumaSection,
    data: (title: string, id = 0) => ({
      id,
      categoryName: title,
      url: 'https://monitor.example.test',
      slug: 'synthetic-monitor',
    }),
    key: 'console_setting.uptime_kuma_groups',
  },
  {
    name: 'announcements',
    Component: AnnouncementsSection,
    data: (title: string, id = 0) => ({
      id,
      content: title,
      publishDate: '2026-11-14T12:00:00Z',
      type: 'default',
      mandatory: true,
      ackRevision: '7',
    }),
    key: 'console_setting.announcements',
  },
] as const
for (const fixture of cases) {
  const { Component } = fixture
  async function mount() {
    const root = createRoot(
      document.body.appendChild(document.createElement('div'))
    )
    roots.push(root)
    const commits: string[] = []
    async function render(data: string, enabled = true) {
      await act(async () => {
        root.render(
          <I18nextProvider i18n={i18n}>
            <Profiler
              id='list'
              onRender={() => {
                commits.push(document.body.textContent || '')
              }}
            >
              <Component data={data} enabled={enabled} />
            </Profiler>
          </I18nextProvider>
        )
        await flush()
      })
    }
    return { render, commits }
  }
  test(`${fixture.name}: lazy initialization displays rows in first commit`, async () => {
    const ui = await mount()
    await ui.render(JSON.stringify([fixture.data('Initial Alpha')]))
    assert.match(ui.commits[0] || '', /Initial Alpha/)
    assert.equal(enabledInput().checked, true)
    assert.deepEqual(writes, [])
  })
  test(`${fixture.name}: data replacement updates rows and retains selection by generated id`, async () => {
    const ui = await mount()
    await ui.render(JSON.stringify([fixture.data('Old Alpha')]))
    await click(checkbox(1))
    assert.equal(button('Delete (1)').disabled, false)
    await ui.render(JSON.stringify([fixture.data('Replacement Beta')]))
    assert.doesNotMatch(document.body.textContent || '', /Old Alpha/)
    assert.match(document.body.textContent || '', /Replacement Beta/)
    assert.equal(button('Delete (1)').disabled, false)
    assert.deepEqual(writes, [])
  })
  test(`${fixture.name}: enabled changes preserve unsaved list and selection`, async () => {
    const ui = await mount()
    const data = JSON.stringify([
      fixture.data('Delete Alpha', 1),
      fixture.data('Keep Beta', 2),
    ])
    await ui.render(data)
    await click(checkbox(1))
    await click(button('Delete (1)'))
    await click(button('Delete'))
    assert.doesNotMatch(document.body.textContent || '', /Delete Alpha/)
    await click(checkbox(1))
    await ui.render(data, false)
    assert.equal(enabledInput().checked, false)
    assert.doesNotMatch(document.body.textContent || '', /Delete Alpha/)
    assert.match(document.body.textContent || '', /Keep Beta/)
    assert.equal(button('Delete (1)').disabled, false)
    assert.equal(button('Save Settings').disabled, false)
    // deepEqual(writes, []) would narrow this async collector to never[].
    assert.equal(writes.length, 0)
    await click(button('Save Settings'))
    const saved = writes.find((request) => request.key === fixture.key)
    assert.ok(saved)
    assert.deepEqual(JSON.parse(String(saved.value)), [
      fixture.data('Keep Beta', 2),
    ])
  })
  test(`${fixture.name}: non-array JSON preserves current rows; malformed JSON clears them`, async () => {
    const ui = await mount()
    await ui.render(JSON.stringify([fixture.data('Retained Alpha')]))
    await click(checkbox(1))
    for (const data of ['{}', 'null', '42', '"not an array"']) {
      await ui.render(data)
      assert.match(document.body.textContent || '', /Retained Alpha/)
      assert.equal(button('Delete (1)').disabled, false)
    }
    await ui.render('{broken-json')
    assert.doesNotMatch(document.body.textContent || '', /Retained Alpha/)
    assert.equal(
      document.querySelectorAll('tbody tr [data-slot="checkbox"]').length,
      0
    )
    assert.deepEqual(writes, [])
  })
  test(`${fixture.name}: malformed/non-array initial data is empty and valid data can recover`, async () => {
    const ui = await mount()
    await ui.render('{broken-json')
    assert.equal(
      document.querySelectorAll('tbody tr [data-slot="checkbox"]').length,
      0
    )
    await ui.render('null')
    await ui.render(JSON.stringify([fixture.data('Recovered Alpha')]))
    assert.match(document.body.textContent || '', /Recovered Alpha/)
    await ui.render('[]')
    assert.doesNotMatch(document.body.textContent || '', /Recovered Alpha/)
  })
  if (fixture.name === 'announcements') {
    test('announcements: refresh preserves the unsaved list', async () => {
      const ui = await mount()
      await ui.render(JSON.stringify([
        fixture.data('Delete Alpha', 1),
        fixture.data('Keep Beta', 2),
      ]))
      await click(checkbox(1))
      await click(button('Delete (1)'))
      await click(button('Delete'))
      await ui.render(JSON.stringify([
        fixture.data('Server Alpha', 1),
        fixture.data('Server Beta', 2),
      ]))
      assert.doesNotMatch(document.body.textContent || '', /Delete Alpha|Server Alpha|Server Beta/)
      assert.match(document.body.textContent || '', /Keep Beta/)
      assert.equal(button('Save Settings').disabled, false)
      await click(button('Save Settings'))
      assert.deepEqual(JSON.parse(String(writes[0]?.value)), [fixture.data('Keep Beta', 2)])
    })
    test('announcements: acknowledgement retains edits made while saving', async () => {
      const ui = await mount()
      await ui.render(JSON.stringify([
        fixture.data('Delete Alpha', 1),
        fixture.data('Delete Beta', 2),
        fixture.data('Keep Gamma', 3),
      ]))
      await click(checkbox(1))
      await click(button('Delete (1)'))
      await click(button('Delete'))
      let acknowledge!: (result: { success: boolean }) => void
      saveResult = () => new Promise((resolve) => { acknowledge = resolve })
      await click(button('Save Settings'))
      assert.equal(writes.length, 1)
      await click(checkbox(1))
      await click(button('Delete (1)'))
      await click(button('Delete'))
      await act(async () => {
        acknowledge({ success: true })
        await flush()
      })
      assert.equal(button('Save Settings').disabled, false)
      assert.doesNotMatch(document.body.textContent || '', /Delete Alpha|Delete Beta/)
      assert.match(document.body.textContent || '', /Keep Gamma/)
      saveResult = async () => ({ success: true })
      await click(button('Save Settings'))
      assert.deepEqual(JSON.parse(String(writes[1]?.value)), [fixture.data('Keep Gamma', 3)])
      assert.equal(button('Save Settings').disabled, true)
    })
    test('announcements: a rejected save keeps the pending list', async () => {
      const ui = await mount()
      await ui.render(JSON.stringify([fixture.data('Delete Alpha', 1)]))
      await click(checkbox(1))
      await click(button('Delete (1)'))
      await click(button('Delete'))
      saveResult = async () => { throw new Error('Server unavailable') }
      await click(button('Save Settings'))
      assert.equal(button('Save Settings').disabled, false)
      assert.doesNotMatch(document.body.textContent || '', /Delete Alpha/)
      saveResult = async () => ({ success: true })
      await click(button('Save Settings'))
      assert.deepEqual(JSON.parse(String(writes[1]?.value)), [])
      assert.equal(button('Save Settings').disabled, true)
    })
  }
  test(`${fixture.name}: unrelated data update preserves locally acknowledged enabled value`, async () => {
    const ui = await mount()
    await ui.render(JSON.stringify([fixture.data('Alpha')]))
    await click(enabledInput())
    assert.equal(enabledInput().checked, false)
    assert.equal(writes.length, 1)
    await ui.render(JSON.stringify([fixture.data('Beta')]))
    assert.equal(enabledInput().checked, false)
    await ui.render(JSON.stringify([fixture.data('Beta')]), false)
    await ui.render(JSON.stringify([fixture.data('Beta')]), true)
    assert.equal(enabledInput().checked, true)
    assert.equal(writes.length, 1)
  })
}
