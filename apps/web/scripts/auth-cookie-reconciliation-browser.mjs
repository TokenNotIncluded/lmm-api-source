/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFile, writeFile, mkdtemp } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const root = fileURLToPath(new URL('../', import.meta.url))
const output = await mkdtemp(path.join(tmpdir(), 'lmm-auth-reconciliation-'))
const entryPath = path.join(output, 'entry.ts')
const from = (name) => JSON.stringify(path.join(root, name))
await writeFile(
  entryPath,
  `
import axios from ${from('node_modules/axios/index.js')}
import * as cookie from ${from('src/lib/auth-cookie.ts')}
import * as recovery from ${from('src/lib/auth-cookie-reconciliation.ts')}
import { capturePasswordLoginGuard, applyAuthBundle, prepareLoginIntentVersion, captureLoginIntentGuard } from ${from('src/lib/auth-session.ts')}
import { useCredentialStore } from ${from('src/stores/auth-store.ts')}
import { createElement, StrictMode } from ${from('node_modules/react/index.js')}
import { createRoot } from ${from('node_modules/react-dom/client.js')}
import { createRouter, createRootRoute, createRoute, RouterProvider } from ${from('node_modules/@tanstack/react-router/dist/esm/index.js')}
import { createInstance } from ${from('node_modules/i18next/dist/esm/i18next.js')}
import { I18nextProvider, initReactI18next } from ${from('node_modules/react-i18next/dist/es/index.js')}
import { AuthCookieRecovery } from ${from('src/components/auth-cookie-recovery.tsx')}
Object.assign(window, { axios, cookie, recovery, capturePasswordLoginGuard, applyAuthBundle, prepareLoginIntentVersion, captureLoginIntentGuard, useCredentialStore })
window.mountRecoveryUI = async () => {
 sessionStorage.setItem('harness:recovery-ui','1')
 const i18n=createInstance(); await i18n.use(initReactI18next).init({lng:'en',resources:{}})
 const route=createRootRoute({component:AuthCookieRecovery});const child=createRoute({getParentRoute:()=>route,path:'$'});const router=createRouter({routeTree:route.addChildren([child])})
 const host=document.createElement('div');document.body.append(host);const renderRoot=createRoot(host);window.unmountRecoveryUI=()=>renderRoot.unmount();renderRoot.render(createElement(StrictMode,null,createElement(I18nextProvider,{i18n},createElement(RouterProvider,{router}))))
}
if(location.pathname==='/sign-in'&&sessionStorage.getItem('harness:recovery-ui')) void window.mountRecoveryUI()
`
)
const built = spawnSync(
  'bun',
  [
    'build',
    entryPath,
    '--target',
    'browser',
    '--outfile',
    path.join(output, 'bundle.js'),
  ],
  { cwd: root, encoding: 'utf8' }
)
assert.equal(built.status, 0, built.stderr)
const key = 'new-api:auth-cookie:uncertain'
const bundle = {
  access_token: 'synthetic-live-access',
  token_type: 'Bearer',
  access_expires_at: Math.floor(Date.now() / 1000) + 600,
  user: { id: 8, username: 'synthetic-root', role: 100 },
  session: {
    sid: 'a'.repeat(64),
    current: true,
    login_method: 'password',
    ip: '',
    user_agent: '',
    created_at: 1,
    last_active_at: 1,
    expires_at: 2000000000,
  },
}
let state = 'current',
  protocol = 'core.browser-session-reconciliation.v1',
  status = 200,
  delayed,
  unlockProbe,
  holdProbe = false
let responseBundle = bundle
let refreshCount = 0,
  holdRefresh = false,
  releaseRefresh,
  expectedCookie,
  postRefreshState = 'current'
const trace = [],
  requests = []
const server = createServer(async (req, res) => {
  const pathname = new URL(req.url, 'http://local').pathname
  if (pathname === '/bundle.js') {
    res.setHeader('Content-Type', 'application/javascript')
    res.end(await readFile(path.join(output, 'bundle.js')))
    return
  }
  if (pathname === '/late') {
    delayed = res
    trace.push('old_request_started')
    res.on('close', () => trace.push('old_transport_closed'))
    return
  }
  if (pathname === '/forbidden-old-login') {
    requests.push('forbidden-old-login')
    res.end('{}')
    return
  }
  if (pathname === '/api/user/auth/reconcile') {
    requests.push({
      method: req.method,
      path: pathname,
      expected: req.headers['x-auth-session'],
      cookie: req.headers.cookie,
    })
    trace.push('readonly_probe_started')
    let body = ''
    for await (const chunk of req) body += chunk
    assert.equal(body, '')
    assert.equal(req.method, 'POST')
    if (req.headers['x-auth-session'] !== undefined) {
      assert.equal(req.headers['x-auth-session'], bundle.session.sid)
    }
    if (holdProbe) await new Promise((r) => (unlockProbe = r))
    res.statusCode = status
    res.setHeader('Content-Type', 'application/json')
    res.setHeader('Cache-Control', 'no-store')
    res.end(
      JSON.stringify({
        success: true,
        message: '',
        data: {
          protocol,
          status:
            expectedCookie &&
            !req.headers.cookie?.includes(`test_refresh=${expectedCookie}`)
              ? 'previous'
              : state,
          ...(state === 'current' &&
          (!expectedCookie ||
            req.headers.cookie?.includes(`test_refresh=${expectedCookie}`))
            ? { bundle: responseBundle }
            : {}),
        },
      })
    )
    return
  }
  if (pathname === '/api/user/auth/refresh') {
    requests.push({
      method: req.method,
      path: pathname,
      expected: req.headers['x-auth-session'],
      cookie: req.headers.cookie,
    })
    assert.equal(state, 'current_access_expired')
    assert.ok(
      !expectedCookie ||
        req.headers.cookie?.includes(`test_refresh=${expectedCookie}`)
    )
    let body = ''
    for await (const chunk of req) body += chunk
    assert.equal(body, '')
    assert.equal(req.method, 'POST')
    refreshCount++
    expectedCookie = `renewed-current-${refreshCount}`
    responseBundle = {
      ...bundle,
      access_token: `synthetic-renewed-access-${refreshCount}`,
    }
    state = postRefreshState
    trace.push('explicit_refresh_started')
    res.on('close', () => trace.push('explicit_refresh_transport_closed'))
    if (holdRefresh) await new Promise((resolve) => (releaseRefresh = resolve))
    res.setHeader(
      'Set-Cookie',
      `test_refresh=${expectedCookie}; HttpOnly; SameSite=Strict; Path=/`
    )
    res.setHeader('Content-Type', 'application/json')
    res.end(JSON.stringify({ success: true, data: responseBundle }))
    trace.push('explicit_refresh_response_attempted')
    return
  }
  res.setHeader('Content-Type', 'text/html')
  res.setHeader('Cache-Control', 'no-store')
  res.end(
    '<!doctype html><meta charset="utf-8"><script type="module" src="/bundle.js"></script><body>controlled recovery document</body>'
  )
})
await new Promise((r) => server.listen(0, '127.0.0.1', r))
const origin = `http://127.0.0.1:${server.address().port}`
const browser = await chromium.launch({
  headless: true,
  executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE,
  args: ['--disable-dev-shm-usage'],
})
const context = await browser.newContext()
await context.addCookies([
  {
    name: 'test_refresh',
    value: 'original-current',
    httpOnly: true,
    url: origin,
  },
])
const [owner, sticky, unseen] = await Promise.all([
  context.newPage(),
  context.newPage(),
  context.newPage(),
])
async function load(page) {
  await page.goto(`${origin}/writer`)
  await page.waitForFunction(() => Boolean(window.recovery))
}
async function renewalDocument() {
  expectedCookie = 'original-current'
  state = 'current_access_expired'
  responseBundle = bundle
  const profile = await browser.newContext()
  await profile.addCookies([
    {
      name: 'test_refresh',
      value: 'original-current',
      httpOnly: true,
      url: origin,
    },
  ])
  const page = await profile.newPage()
  await load(page)
  await page.evaluate(
    (b) => window.applyAuthBundle({ ...b, access_expires_at: 1 }, false),
    bundle
  )
  await page.evaluate(() =>
    window.cookie
      .withAuthCookieLock((t) =>
        t.request(() =>
          window.axios.post('/late', {}, { timeout: 150, adapter: 'xhr' })
        )
      )
      .catch(() => {})
  )
  await page.evaluate(() => window.mountRecoveryUI())
  await page
    .getByRole('button', { name: 'Recover browser session', exact: true })
    .click()
  await page
    .getByRole('button', { name: 'Renew browser session', exact: true })
    .waitFor()
  const marker = await page.evaluate((k) => localStorage.getItem(k), key)
  return { profile, page, marker }
}
const evidence = { syntheticServer: true, cases: [], trace, requests }
try {
  await Promise.all([load(owner), load(sticky), load(unseen)])
  await owner.evaluate((b) => window.applyAuthBundle(b, false), bundle)
  await unseen.evaluate(async () => {
    const version = await window.prepareLoginIntentVersion()
    window.stalePasswordGuard = window.capturePasswordLoginGuard()
    window.stalePasskeyGuard = window.captureLoginIntentGuard(version.version)
  })
  await owner.evaluate(() => {
    window.nativeEvents = []
    const Native = window.XMLHttpRequest
    window.XMLHttpRequest = class extends Native {
      send(...a) {
        for (const type of ['timeout', 'abort', 'error', 'loadend']) {
          this.addEventListener(type, () =>
            window.nativeEvents.push({ type, state: this.readyState })
          )
        }
        return super.send(...a)
      }
    }
    window.oldRequest = window.cookie
      .withAuthCookieLock((t) =>
        t.request(() =>
          window.axios.post('/late', {}, { timeout: 150, adapter: 'xhr' })
        )
      )
      .catch((e) => e.code)
  })
  await owner.waitForFunction(() =>
    window.nativeEvents.some((e) => e.type === 'loadend')
  )
  assert.equal(
    await owner.evaluate(() => window.cookie.getAuthCookieBlockedState()),
    'uncertain'
  )
  assert.ok(
    trace.includes('old_transport_closed'),
    'native HTTP connection must already be closed, not merely its lock released'
  )
  assert.ok(
    (await owner.evaluate(() => window.nativeEvents)).some(
      (e) => e.type === 'abort' && e.state === 4
    )
  )
  evidence.nativeEvents = await owner.evaluate(() => window.nativeEvents)
  await sticky.evaluate(() => window.cookie.getAuthCookieBlockedState())
  const oldMarker = await owner.evaluate((k) => localStorage.getItem(k), key)
  await Promise.all([
    owner.waitForURL('**/sign-in?reauth=1'),
    owner
      .evaluate(async () => {
        await window.recovery.prepareAuthCookieReconciliation()
        const fenced = await window.cookie
          .withAuthCookieActivity(async () => false)
          .catch(() => true)
        sessionStorage.setItem('harness:old-document-fenced', String(fenced))
      })
      .catch((e) => {
        if (!/context|navigation/i.test(e.message)) throw e
      }),
  ])
  await owner.waitForFunction(() => Boolean(window.recovery))
  assert.equal(
    await owner.evaluate((k) => localStorage.getItem(k), key),
    oldMarker
  )
  assert.equal(
    await owner.evaluate(() =>
      sessionStorage.getItem('harness:old-document-fenced')
    ),
    'true'
  )
  evidence.cases.push(
    'explicit full-document handoff keeps original uncertain marker'
  )
  for (const bad of [
    'previous',
    'expired_previous',
    'unknown_sid',
    'unknown_cookie',
    'no_cookie',
    'sid_mismatch',
    'family_inactive',
    'current_access_expired',
    'authority_unavailable',
  ]) {
    state = bad
    assert.equal(
      await owner.evaluate(async (classification) => {
        try {
          await window.recovery.reconcileAuthCookieSession()
          return false
        } catch (error) {
          return classification === 'current_access_expired'
            ? error instanceof
                window.recovery.AuthCookieSessionNeedsRenewalError
            : error instanceof
                window.recovery.AuthCookieSessionReauthenticationError
        }
      }, bad),
      true
    )
    assert.equal(
      await owner.evaluate((k) => localStorage.getItem(k), key),
      oldMarker
    )
    if (bad !== 'current_access_expired') {
      assert.equal(
        await owner.evaluate(async () => {
          try {
            await window.recovery.renewAuthCookieSession()
            return false
          } catch (error) {
            return (
              error instanceof
              window.recovery.AuthCookieSessionReauthenticationError
            )
          }
        }),
        true
      )
      assert.equal(refreshCount, 0)
    }
  }
  evidence.cases.push('all non-current classifications preserve guard')
  state = 'current'
  for (const badStatus of [401, 503]) {
    status = badStatus
    assert.equal(
      await owner.evaluate(async () => {
        try {
          await window.recovery.reconcileAuthCookieSession()
          return false
        } catch {
          return true
        }
      }),
      true
    )
    assert.equal(
      await owner.evaluate((k) => localStorage.getItem(k), key),
      oldMarker
    )
  }
  status = 200
  protocol = 'wrong-protocol'
  assert.equal(
    await owner.evaluate(async () => {
      try {
        await window.recovery.reconcileAuthCookieSession()
        return false
      } catch {
        return true
      }
    }),
    true
  )
  assert.equal(
    await owner.evaluate((k) => localStorage.getItem(k), key),
    oldMarker
  )
  evidence.cases.push('401,503 and invalid protocol never confirm')
  protocol = 'core.browser-session-reconciliation.v1'
  for (const invalid of [
    { ...bundle, access_token: '' },
    { ...bundle, access_expires_at: 1 },
    { ...bundle, session: { ...bundle.session, sid: 'b'.repeat(64) } },
    { ...bundle, session: { ...bundle.session, current: false } },
  ]) {
    responseBundle = invalid
    assert.equal(
      await owner.evaluate(async () => {
        try {
          await window.recovery.reconcileAuthCookieSession()
          return false
        } catch {
          return true
        }
      }),
      true
    )
    assert.equal(
      await owner.evaluate((k) => localStorage.getItem(k), key),
      oldMarker
    )
  }
  responseBundle = bundle
  evidence.cases.push(
    'malformed, expired, mismatched SID and noncurrent bundle never confirm'
  )
  holdProbe = true
  unlockProbe = undefined
  const cancelled = owner.evaluate(() => {
    window.probeController = new AbortController()
    return window.recovery
      .reconcileAuthCookieSession(window.probeController.signal)
      .then(
        () => false,
        () => true
      )
  })
  while (!unlockProbe) await new Promise((r) => setTimeout(r, 5))
  await owner.evaluate(() => window.probeController.abort())
  assert.equal(await cancelled, true)
  assert.equal(
    await owner.evaluate((k) => localStorage.getItem(k), key),
    oldMarker
  )
  unlockProbe()
  unlockProbe = undefined
  evidence.cases.push('cancelled read-only proof preserves the original marker')
  protocol = 'core.browser-session-reconciliation.v1'
  holdProbe = true
  const recovered = owner.evaluate(() =>
    window.recovery.reconcileAuthCookieSession()
  )
  while (!unlockProbe) await new Promise((r) => setTimeout(r, 5))
  await unseen.evaluate(() => {
    window.staleResult = Promise.all(
      [window.stalePasswordGuard, window.stalePasskeyGuard].map((guard) =>
        window.cookie
          .withAuthCookieLock(async (t) => {
            guard()
            await t.request(() => window.axios.post('/forbidden-old-login'))
          })
          .then(
            () => false,
            () => true
          )
      )
    )
  })
  unlockProbe()
  await recovered
  await unseen.waitForFunction(() => window.staleResult !== undefined)
  assert.deepEqual(await unseen.evaluate(() => window.staleResult), [
    true,
    true,
  ])
  assert.equal(requests.includes('forbidden-old-login'), false)
  assert.equal(await owner.evaluate((k) => localStorage.getItem(k), key), null)
  assert.equal(
    await owner.evaluate(
      () => window.useCredentialStore.getState().auth.user?.id
    ),
    8
  )
  assert.equal(
    await sticky.evaluate(() => window.cookie.getAuthCookieBlockedState()),
    'uncertain'
  )
  evidence.cases.push(
    'current server proof commits existing bundle; stale unobserved login/passkey generation rejected; observed old realm stays blocked'
  )
  delayed.writeHead(200, {
    'Set-Cookie':
      'test_refresh=late-old-family; HttpOnly; SameSite=Strict; Path=/',
    'Content-Type': 'application/json',
  })
  delayed.end(JSON.stringify({ success: true, data: bundle }))
  trace.push('late_server_response_attempted')
  await owner.reload()
  await owner.waitForFunction(() => Boolean(window.recovery))
  assert.equal(
    (await context.cookies()).find((c) => c.name === 'test_refresh')?.value,
    'original-current'
  )
  evidence.cases.push(
    'late server Set-Cookie after native XHR abort and document replacement cannot overwrite current cookie'
  )
  await owner.evaluate(
    (k) => localStorage.setItem(k, 'pending:original-operation-id'),
    key
  )
  assert.equal(
    await owner.evaluate(async () => {
      try {
        await window.recovery.prepareAuthCookieReconciliation()
        return false
      } catch {
        return true
      }
    }),
    true
  )
  assert.equal(
    await owner.evaluate((k) => localStorage.getItem(k), key),
    'pending:original-operation-id'
  )
  evidence.cases.push('pending operation ID is never replaced or removed')
  assert.ok(
    trace.indexOf('old_transport_closed') <
      trace.indexOf('readonly_probe_started')
  )

  holdProbe = false
  state = 'previous'
  protocol = 'core.browser-session-reconciliation.v1'
  status = 503
  const uiContext = await browser.newContext()
  await uiContext.addCookies([
    {
      name: 'test_refresh',
      value: 'original-current',
      httpOnly: true,
      url: origin,
    },
  ])
  const ui = await uiContext.newPage()
  const probesBeforeUI = requests.length
  await load(ui)
  await ui.evaluate(() =>
    window.cookie
      .withAuthCookieLock((t) =>
        t.request(() =>
          window.axios.post('/late', {}, { timeout: 150, adapter: 'xhr' })
        )
      )
      .catch(() => {})
  )
  await ui.evaluate(() => window.mountRecoveryUI())
  await ui
    .getByRole('button', { name: 'Recover browser session', exact: true })
    .click()
  await ui
    .getByText(
      'The server could not confirm a current browser session. Recovery has not completed.',
      { exact: true }
    )
    .waitFor()
  const uiMarker = await ui.evaluate((k) => localStorage.getItem(k), key)
  state = 'current'
  status = 200
  holdProbe = true
  unlockProbe = undefined
  await ui
    .getByRole('button', { name: 'Recover browser session', exact: true })
    .click()
  while (!unlockProbe) await new Promise((resolve) => setTimeout(resolve, 5))
  await ui.evaluate(() => window.unmountRecoveryUI())
  unlockProbe()
  holdProbe = false
  unlockProbe = undefined
  await ui.waitForFunction(() => window.cookie.isAuthCookieIdle())
  assert.equal(await ui.evaluate((k) => localStorage.getItem(k), key), uiMarker)
  assert.equal(
    await ui.evaluate(
      () => window.useCredentialStore.getState().auth.accessToken
    ),
    null
  )
  evidence.cases.push(
    'unmount during a manual retry cancels the active proof; a late current reply cannot unlock or apply credentials'
  )
  await ui.evaluate(() => window.mountRecoveryUI())
  await ui.waitForFunction(
    () => window.useCredentialStore?.getState().auth.user?.id === 8
  )
  await ui
    .getByRole('button', { name: 'Recover browser session', exact: true })
    .waitFor({ state: 'detached' })
  assert.equal(await ui.evaluate((k) => localStorage.getItem(k), key), null)
  await ui.waitForURL((url) => url.pathname !== '/sign-in', { timeout: 5000 })
  assert.ok(
    !ui.url().includes('/sign-in'),
    'successful proof must navigate the user out of the recovery sign-in page'
  )
  evidence.cases.push(
    'actual StrictMode recovery button with no cached SID omits optional header, performs document handoff and restores the cookie current account, then leaves sign-in'
  )
  assert.ok(
    requests
      .slice(probesBeforeUI)
      .every((request) => request.expected === undefined)
  )
  await uiContext.close()

  state = 'expired_previous'
  const terminalContext = await browser.newContext()
  await terminalContext.addCookies([
    {
      name: 'test_refresh',
      value: 'original-current',
      httpOnly: true,
      url: origin,
    },
  ])
  const terminal = await terminalContext.newPage()
  await load(terminal)
  await terminal.evaluate(() =>
    window.cookie
      .withAuthCookieLock((t) =>
        t.request(() =>
          window.axios.post('/late', {}, { timeout: 150, adapter: 'xhr' })
        )
      )
      .catch(() => {})
  )
  await terminal.evaluate(() => window.mountRecoveryUI())
  await terminal
    .getByRole('button', { name: 'Recover browser session', exact: true })
    .click()
  await terminal
    .getByText(
      'This browser session is no longer available. Sign in again to continue.',
      { exact: true }
    )
    .waitFor()
  const terminalMarker = await terminal.evaluate(
    (k) => localStorage.getItem(k),
    key
  )
  await terminal
    .getByRole('button', { name: 'Back to Home', exact: true })
    .click()
  await terminal.waitForURL((url) => url.pathname === '/')
  assert.equal(
    await terminal.evaluate((k) => localStorage.getItem(k), key),
    terminalMarker
  )
  assert.equal(
    await terminal.evaluate(
      () => window.useCredentialStore.getState().auth.accessToken
    ),
    null
  )
  await terminal.reload()
  await terminal.waitForFunction(() => Boolean(window.recovery))
  await terminal.evaluate(() => window.mountRecoveryUI())
  await terminal
    .getByRole('button', { name: 'Sign in again', exact: true })
    .click()
  await terminal.waitForURL((url) => url.pathname === '/sign-in')
  await terminal.waitForFunction(
    (k) => localStorage.getItem(k) === null && Boolean(window.recovery),
    key
  )
  assert.equal(
    await terminal.evaluate(() =>
      sessionStorage.getItem('lmm:auth-cookie-reconciliation:v1')
    ),
    null
  )
  assert.equal(refreshCount, 0)
  assert.equal(
    await terminal
      .getByRole('button', { name: 'Sign in again', exact: true })
      .count(),
    0
  )
  evidence.cases.push(
    'expired previous session offers a public-home exit and explicit full-document reauthentication without replaying refresh or repeating the recovery dead end'
  )
  await terminalContext.close()

  // A genuine browser Cookie mutation is separate evidence from read-only proof.
  holdRefresh = false
  postRefreshState = 'current'
  const fresh = await renewalDocument()
  assert.equal(
    refreshCount,
    0,
    'an expired classification must never automatically refresh'
  )
  await fresh.page
    .getByRole('button', { name: 'Renew browser session', exact: true })
    .click()
  await fresh.page.waitForURL((url) => url.pathname !== '/sign-in', {
    timeout: 5000,
  })
  assert.equal(refreshCount, 1)
  assert.equal(
    await fresh.page.evaluate((k) => localStorage.getItem(k), key),
    null
  )
  assert.equal(
    await fresh.page.evaluate(
      () => window.useCredentialStore.getState().auth.accessToken
    ),
    responseBundle.access_token
  )
  assert.equal(
    (await fresh.profile.cookies()).find(
      (cookie) => cookie.name === 'test_refresh'
    )?.value,
    expectedCookie
  )
  assert.deepEqual(
    requests.slice(-3).map((request) => request.path),
    [
      '/api/user/auth/reconcile',
      '/api/user/auth/refresh',
      '/api/user/auth/reconcile',
    ]
  )
  evidence.cases.push(
    'expired access requires an explicit renewal click: read-only expired proof, one real XHR Set-Cookie, matching current proof, then commit'
  )
  await fresh.profile.close()

  postRefreshState = 'previous'
  const badFinal = await renewalDocument()
  await badFinal.page
    .getByRole('button', { name: 'Renew browser session', exact: true })
    .click()
  await badFinal.page
    .getByText(
      'This browser session is no longer available. Sign in again to continue.',
      { exact: true }
    )
    .waitFor()
  assert.equal(
    await badFinal.page.evaluate((k) => localStorage.getItem(k), key),
    badFinal.marker
  )
  assert.equal(
    await badFinal.page.evaluate(
      () => window.useCredentialStore.getState().auth.accessToken
    ),
    null
  )
  evidence.cases.push(
    'a completed refresh and Set-Cookie cannot unlock without a matching terminal current proof'
  )
  await badFinal.profile.close()

  postRefreshState = 'current'
  holdRefresh = true
  releaseRefresh = undefined
  const cancelledRenewal = await renewalDocument()
  await cancelledRenewal.page
    .getByRole('button', { name: 'Renew browser session', exact: true })
    .click()
  while (!releaseRefresh) await new Promise((resolve) => setTimeout(resolve, 5))
  await cancelledRenewal.page.evaluate(() => window.unmountRecoveryUI())
  assert.equal(
    await cancelledRenewal.page.evaluate(() =>
      window.cookie.isAuthCookieIdle()
    ),
    false,
    'a dispatched Cookie writer must keep the lock through native transport completion'
  )
  releaseRefresh()
  holdRefresh = false
  releaseRefresh = undefined
  await cancelledRenewal.page.waitForFunction(() =>
    window.cookie.isAuthCookieIdle()
  )
  assert.equal(
    await cancelledRenewal.page.evaluate((k) => localStorage.getItem(k), key),
    cancelledRenewal.marker
  )
  assert.equal(
    await cancelledRenewal.page.evaluate(
      () => window.useCredentialStore.getState().auth.accessToken
    ),
    null
  )
  assert.equal(
    requests.at(-1).path,
    '/api/user/auth/refresh',
    'cancellation must not start the terminal probe or apply authority'
  )
  evidence.cases.push(
    'cancelled in-flight renewal retains its lock until native completion, keeps the exact old marker and applies no authority'
  )
  await cancelledRenewal.profile.close()

  holdRefresh = true
  releaseRefresh = undefined
  const timedOut = await renewalDocument()
  const beforeTimeout = refreshCount
  await timedOut.page.evaluate(() => {
    window.renewalNativeEvents = []
    const Native = window.XMLHttpRequest
    window.XMLHttpRequest = class extends Native {
      send(...args) {
        for (const type of ['abort', 'timeout', 'error', 'loadend']) {
          this.addEventListener(type, () =>
            window.renewalNativeEvents.push({ type, state: this.readyState })
          )
        }
        return super.send(...args)
      }
    }
  })
  await timedOut.page
    .getByRole('button', { name: 'Renew browser session', exact: true })
    .click()
  await timedOut.page
    .getByText(
      'The server could not confirm a current browser session. Recovery has not completed.',
      { exact: true }
    )
    .waitFor({ timeout: 15000 })
  assert.equal(refreshCount, beforeTimeout + 1)
  assert.equal(
    await timedOut.page.evaluate((k) => localStorage.getItem(k), key),
    timedOut.marker
  )
  const nativeRenewalEvents = await timedOut.page.evaluate(
    () => window.renewalNativeEvents
  )
  assert.ok(
    nativeRenewalEvents.some(
      (event) => event.type === 'abort' && event.state === 4
    )
  )
  assert.ok(
    nativeRenewalEvents.some(
      (event) => event.type === 'loadend' && event.state === 4
    )
  )
  evidence.renewalTimeoutNativeEvents = nativeRenewalEvents
  const beforeLateAttempt = trace.filter(
    (event) => event === 'explicit_refresh_response_attempted'
  ).length
  releaseRefresh()
  holdRefresh = false
  releaseRefresh = undefined
  while (
    trace.filter((event) => event === 'explicit_refresh_response_attempted')
      .length === beforeLateAttempt
  ) {
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  assert.equal(
    (await timedOut.profile.cookies()).find(
      (cookie) => cookie.name === 'test_refresh'
    )?.value,
    'original-current'
  )
  assert.equal(
    await timedOut.page.evaluate(async () => {
      try {
        await window.recovery.renewAuthCookieSession()
        return false
      } catch {
        return true
      }
    }),
    true
  )
  assert.equal(
    refreshCount,
    beforeTimeout + 1,
    'a previous Cookie after lost refresh must never be replayed'
  )
  assert.equal(
    await timedOut.page.evaluate((k) => localStorage.getItem(k), key),
    timedOut.marker
  )
  evidence.cases.push(
    'real 10s renewal timeout natively aborts; late Set-Cookie cannot overwrite; read-only previous classification prevents refresh replay'
  )
  await timedOut.profile.close()
  evidence.sourceSha256 = Object.fromEntries(
    await Promise.all(
      [
        'src/lib/auth-cookie.ts',
        'src/lib/auth-cookie-reconciliation.ts',
        'src/lib/auth-session.ts',
        'src/components/auth-cookie-recovery.tsx',
        'scripts/auth-cookie-reconciliation-browser.mjs',
      ].map(async (name) => [
        name,
        createHash('sha256')
          .update(await readFile(path.join(root, name)))
          .digest('hex'),
      ])
    )
  )
  evidence.bundleSha256 = createHash('sha256')
    .update(await readFile(path.join(output, 'bundle.js')))
    .digest('hex')
  await writeFile(
    path.join(output, 'result.json'),
    JSON.stringify(evidence, null, 2)
  )
  console.log(
    JSON.stringify({
      output,
      cases: evidence.cases,
      nativeEvents: evidence.nativeEvents,
      requests: requests.length,
    })
  )
} finally {
  await browser.close()
  server.closeAllConnections()
  await new Promise((r) => server.close(r))
}
