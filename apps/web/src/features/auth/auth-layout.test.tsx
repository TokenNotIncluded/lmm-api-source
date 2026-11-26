/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterProvider,
} from '@tanstack/react-router'
import { renderToStaticMarkup } from 'react-dom/server'

import appI18n from '@/i18n/config'

type Session = { user: { id: number } | null; accessToken: string | null }
let session: Session = { user: null, accessToken: null }
const authStore = { ...(await import('@/stores/auth-store')) }
mock.module('@/stores/auth-store', () => ({
  ...authStore,
  useAuthStore: (select: (state: { auth: Session }) => unknown) =>
    select({ auth: session }),
}))
const { AuthLayout } = await import('./auth-layout')

async function renderHeader(auth: Session, allowConsoleReturn = false) {
  session = auth
  if (!appI18n.isInitialized) {
    await new Promise<void>((resolve) => appI18n.on('initialized', resolve))
  }
  await appI18n.changeLanguage('en')
  const route = createRootRoute({
    component: () => (
      <AuthLayout allowConsoleReturn={allowConsoleReturn}>
        <p>Flow content</p>
      </AuthLayout>
    ),
  })
  const router = createRouter({
    routeTree: route,
    history: createMemoryHistory({ initialEntries: ['/'] }),
    isServer: true,
  })
  await router.load()
  return renderToStaticMarkup(<RouterProvider router={router} />)
}

test('the auth header offers a dashboard return only with a usable session and an opted-in flow', async () => {
  const signedIn = { user: { id: 1 }, accessToken: 'session-fixture' }
  for (const session of [
    { user: null, accessToken: null },
    { user: signedIn.user, accessToken: null },
    { user: null, accessToken: signedIn.accessToken },
  ]) {
    const html = await renderHeader(session, true)
    assert.doesNotMatch(html, /href="\/dashboard\/overview"/)
    assert.match(html, /href="\/"/)
  }
  const available = await renderHeader(signedIn, true)
  assert.match(
    available,
    /<header[^>]*>[\s\S]*?href="\/dashboard\/overview"[\s\S]*?<\/header>/
  )
  assert.doesNotMatch(
    await renderHeader(signedIn),
    /href="\/dashboard\/overview"/
  )
})
