/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

import type { SyncDiffData } from '../../types'

const dom = new Window({ url: 'https://synthetic.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value:
      key === 'getComputedStyle' ? dom.getComputedStyle.bind(dom) : dom[key],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
mock.module('@/components/dialog', () => ({
  Dialog: ({
    open,
    children,
    footer,
  }: {
    open: boolean
    children: ReactNode
    footer?: ReactNode
  }) =>
    open ? (
      <section>
        {children}
        {footer}
      </section>
    ) : null,
}))
mock.module('@/components/json-code-editor', () => ({
  JsonCodeEditor: ({
    value,
    onChange,
  }: {
    value: string
    onChange: (value: string) => void
  }) => (
    <textarea
      aria-label='Synthetic JSON editor'
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}))
mock.module('@/hooks/use-mobile', () => ({ useIsMobile: () => false }))
let conflicts: SyncDiffData['conflicts'] = []
mock.module('../models-provider', () => ({
  useModels: () => ({
    upstreamConflicts: conflicts,
    setUpstreamConflicts: (value: SyncDiffData['conflicts']) => {
      conflicts = value
    },
    syncWizardOptions: { locale: 'en' },
  }),
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider } = await import('react-i18next')
const { ParamOverrideEditorDialog } =
  await import('@/features/channels/components/dialogs/param-override-editor-dialog')
const { UpstreamConflictDialog } = await import('./upstream-conflict-dialog')
const i18n = createInstance()
await i18n.init({
  lng: 'en',
  fallbackLng: 'en',
  keySeparator: false,
  nsSeparator: false,
  resources: { en: { translation: {} } },
})
const client = new QueryClient({
  defaultOptions: { queries: { retry: false } },
})
let root: ReturnType<typeof createRoot> | undefined
async function render(node: ReactNode) {
  if (!root) {
    const host = document.createElement('div')
    document.body.append(host)
    root = createRoot(host)
  }
  await act(async () =>
    root?.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>{node}</I18nextProvider>
      </QueryClientProvider>
    )
  )
}
afterEach(async () => {
  await act(async () => root?.unmount())
  root = undefined
  document.body.replaceChildren()
  client.clear()
})
after(() => dom.close())
async function typeJson(value: string) {
  const input = document.querySelector('textarea')
  assert.ok(input)
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLTextAreaElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(
      new dom.Event('input', { bubbles: true }) as unknown as Event
    )
  })
}
function button(label: string) {
  const found = [...document.querySelectorAll('button')].find(
    (item) => item.textContent === label
  )
  assert.ok(found)
  return found
}

test('parameter editor preserves edits across rerenders and resets on value change or reopen', async () => {
  const saved: string[] = []
  const show = (open: boolean, value: string) =>
    render(
      <ParamOverrideEditorDialog
        open={open}
        value={value}
        onOpenChange={() => {}}
        onSave={(value) => saved.push(value)}
      />
    )
  await show(true, '[1]')
  assert.deepEqual(
    JSON.parse(document.querySelector('textarea')?.value ?? ''),
    [1]
  )
  await typeJson('[2]')
  await show(true, '[1]')
  assert.deepEqual(
    JSON.parse(document.querySelector('textarea')?.value ?? ''),
    [2]
  )
  await act(async () => button('Save').click())
  assert.deepEqual(JSON.parse(saved[0]), [2])
  await show(true, '[3]')
  assert.deepEqual(
    JSON.parse(document.querySelector('textarea')?.value ?? ''),
    [3]
  )
  await typeJson('[4]')
  await show(false, '[3]')
  await show(true, '[3]')
  assert.deepEqual(
    JSON.parse(document.querySelector('textarea')?.value ?? ''),
    [3]
  )
})

test('upstream conflicts reset selection and pagination for a refreshed list and reopen', async () => {
  conflicts = Array.from({ length: 12 }, (_, id) => ({
    model_name: `synthetic-${id}`,
    fields: [{ field: 'description', local: 'old', upstream: 'new' }],
  }))
  const show = (open: boolean) =>
    render(<UpstreamConflictDialog open={open} onOpenChange={() => {}} />)
  await show(true)
  assert.match(document.body.textContent ?? '', /Page 1 of 2/)
  const next = document.querySelector<HTMLButtonElement>(
    'button[aria-label="Next page"]'
  )
  assert.ok(next)
  await act(async () => next.click())
  assert.match(document.body.textContent ?? '', /Page 2 of 2/)
  const selection =
    document.querySelector<HTMLButtonElement>('[role="checkbox"]')
  assert.ok(selection)
  await act(async () => selection.click())
  assert.equal(button('Apply Overwrite').disabled, false)
  conflicts = conflicts.slice(0, 1)
  await show(true)
  assert.match(document.body.textContent ?? '', /Page 1 of 1/)
  assert.equal(button('Apply Overwrite').disabled, true)
  await show(false)
  await show(true)
  assert.match(document.body.textContent ?? '', /Page 1 of 1/)
  assert.equal(button('Apply Overwrite').disabled, true)
})
