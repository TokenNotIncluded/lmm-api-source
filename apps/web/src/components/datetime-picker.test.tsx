/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'Node',
  'Element',
  'Event',
  'MutationObserver',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}
Object.assign(globalThis, {
  requestAnimationFrame: domWindow.requestAnimationFrame.bind(domWindow),
  cancelAnimationFrame: domWindow.cancelAnimationFrame.bind(domWindow),
  getComputedStyle: domWindow.getComputedStyle.bind(domWindow),
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const i18next = (await import('i18next')).default
const { initReactI18next } = await import('react-i18next')
await i18next.use(initReactI18next).init({
  lng: 'en',
  resources: {
    en: { translation: {} },
    'zh-CN': { translation: { Time: '时间' } },
    'zh-TW': { translation: { Time: '時間' } },
  },
})
const { DateTimePicker } = await import('./datetime-picker')
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

after(() => domWindow.close())

test('external dates synchronize without emitting changes or overwriting local clear on an equivalent render', async () => {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const changes: Array<Date | undefined> = []
  const onChange = (date: Date | undefined) => changes.push(date)
  const initial = new Date(2026, 1, 3, 14, 25)
  const replacement = new Date(2027, 6, 8, 9, 45)
  const timeInput = () => {
    const input = container.querySelector<HTMLInputElement>('input[type=time]')
    assert.ok(input)
    return input
  }
  try {
    await act(async () =>
      root.render(<DateTimePicker value={initial} onChange={onChange} />)
    )
    assert.ok(container.textContent?.includes('2026-02-03'))
    assert.equal(timeInput().value, '14:25')

    await act(async () =>
      root.render(<DateTimePicker value={replacement} onChange={onChange} />)
    )
    assert.ok(container.textContent?.includes('2027-07-08'))
    assert.equal(timeInput().value, '09:45')
    assert.deepEqual(changes, [])

    // Removing the external date retains the time draft, as before.
    await act(async () => root.render(<DateTimePicker onChange={onChange} />))
    assert.equal(timeInput().disabled, true)
    assert.equal(timeInput().value, '09:45')

    await act(async () =>
      root.render(<DateTimePicker value={replacement} onChange={onChange} />)
    )
    const clear =
      container.querySelector<HTMLButtonElement>('[aria-label=Clear]')
    assert.ok(clear)
    await act(async () => clear.click())
    assert.deepEqual(changes, [undefined])
    assert.equal(timeInput().value, '00:00')
    assert.equal(timeInput().disabled, true)

    await act(async () =>
      root.render(
        <DateTimePicker
          value={new Date(replacement.getTime())}
          onChange={onChange}
        />
      )
    )
    assert.equal(timeInput().disabled, true)
    assert.ok(container.textContent?.includes('Select date'))
    assert.deepEqual(changes, [undefined])
  } finally {
    await act(async () => root.unmount())
    container.remove()
  }
})

for (const [language, timeName, firstWeekday] of [
  ['zh-CN', '时间', '一'],
  ['zh-TW', '時間', '一'],
] as const) {
  test(`${language} renders its Chinese calendar and accessible time name`, async () => {
    await i18next.changeLanguage(language)
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)
    try {
      await act(async () =>
        root.render(<DateTimePicker value={new Date(2026, 1, 3, 14, 25)} />)
      )
      const time = container.querySelector<HTMLInputElement>('input[type=time]')
      assert.ok(time)
      assert.equal(time.getAttribute('aria-label'), timeName)
      const trigger = container.querySelector<HTMLButtonElement>('button')
      assert.ok(trigger)
      await act(async () => trigger.click())
      const headers = document.querySelectorAll('th')
      assert.equal(headers.length, 7)
      assert.equal(headers[0].textContent, firstWeekday)
      assert.deepEqual(
        Array.from(headers, (header) => header.textContent),
        ['一', '二', '三', '四', '五', '六', '日']
      )
    } finally {
      await act(async () => root.unmount())
      container.remove()
      await i18next.changeLanguage('en')
    }
  })
}

test('same timestamp in a new Date preserves the browsed month; a changed timestamp synchronizes', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const value = new Date(2026, 1, 3, 14, 25)
  const changes: Array<Date | undefined> = []
  const render = async (date: Date) =>
    act(async () =>
      root.render(
        <DateTimePicker value={date} onChange={(next) => changes.push(next)} />
      )
    )
  const month = () =>
    document.querySelector('[role=grid]')?.getAttribute('aria-label')
  try {
    await render(value)
    const trigger = host.querySelector<HTMLButtonElement>('button')
    assert.ok(trigger)
    await act(async () => trigger.click())
    const initialMonth = month()
    assert.ok(initialMonth)
    const next = [
      ...document.querySelectorAll<HTMLButtonElement>('button'),
    ].find((button) =>
      /next month/i.test(button.getAttribute('aria-label') || '')
    )
    assert.ok(next)
    await act(async () => next.click())
    const browsedMonth = month()
    assert.notEqual(browsedMonth, initialMonth)
    await render(new Date(value.getTime()))
    assert.equal(month(), browsedMonth)
    await render(new Date(2027, 7, 9, 8, 35))
    assert.match(month() || '', /August 2027/)
    assert.equal(
      host.querySelector<HTMLInputElement>('input[type=time]')?.value,
      '08:35'
    )
    assert.deepEqual(changes, [])
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

test('Invalid Date timestamps stabilize without render loops and later valid or undefined values synchronize', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const changes: Array<Date | undefined> = []
  const render = async (value?: Date) =>
    act(async () =>
      root.render(
        <DateTimePicker value={value} onChange={(next) => changes.push(next)} />
      )
    )
  try {
    await render(new Date(Number.NaN))
    await render(new Date(Number.NaN))
    await render(new Date(2027, 7, 9, 8, 35))
    assert.equal(
      host.querySelector<HTMLInputElement>('input[type=time]')?.value,
      '08:35'
    )
    await render(undefined)
    assert.equal(
      host.querySelector<HTMLInputElement>('input[type=time]')?.disabled,
      true
    )
    assert.deepEqual(changes, [])
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})
