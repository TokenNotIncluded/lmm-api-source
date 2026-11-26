/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import path from 'node:path'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
function localOrigin(value) {
  const url = new URL(value)
  assert.equal(url.protocol, 'http:')
  assert.equal(url.hostname, '127.0.0.1')
  assert.equal(url.href, `${url.origin}/`)
  return url.origin
}
const publicOrigin = localOrigin(
  process.env.LAYOUT_PUBLIC_ORIGIN ?? 'http://127.0.0.1:4175'
)
const consoleOrigin = localOrigin(
  process.env.LAYOUT_CONSOLE_ORIGIN ?? 'http://127.0.0.1:4174'
)
assert.notEqual(
  publicOrigin,
  consoleOrigin,
  'Use the normal entry for guests and the debug entry for signed-in users'
)
const output = process.env.STANDALONE_REVIEW_OUTPUT
if (output) await mkdir(output, { recursive: true })
const dashboard = '/dashboard/overview'
const browser = await chromium.launch({
  headless: true,
  executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE,
})

const publicReads = {
  '/api/status': {
    system_name: 'LMM API',
    logo: '/logo.png',
    password_login_enabled: true,
    assistant: { enabled: false },
    announcements_enabled: false,
    backend_capabilities: { bounty_public_read: false },
  },
  '/api/setup': { status: true, root_init: true, database_type: 'postgres' },
  '/api/scripts': [],
  '/api/site/navigation': { value: '{}', revision: 1 },
  '/api/notice': '',
  '/api/privacy-policy': '# Local privacy review\n\nPublic document.',
  '/api/assistant/pre-conversation-presets': {
    generation: 1,
    version: 'local',
    presets: [],
  },
}

async function isolate(context, origin, signedIn, errors) {
  await context.routeWebSocket('**/*', (socket) => socket.close())
  await context.route('**/*', (route) => {
    const request = route.request()
    const { pathname, origin: requestedOrigin } = new URL(request.url())
    if (requestedOrigin !== origin) return route.abort('blockedbyclient')
    if (
      !signedIn &&
      pathname === '/api/user/auth/refresh' &&
      request.method() === 'POST'
    ) {
      return route.fulfill({ status: 401, json: { success: false } })
    }
    // The regular Rsbuild dev entry requests locally compiled route chunks.
    if (pathname === '/_rspack/lazy/trigger' && request.method() === 'POST') {
      return route.continue()
    }
    if (request.method() === 'GET') {
      if (!signedIn && pathname === '/core/v1/pricing/public') {
        return route.fulfill({
          json: {
            schema_version: 1,
            visibility: 'public',
            observed_at: 100,
            credits_per_usd: 500_000,
            rate_basis: 'base_group_rate',
            invocation_authorized: false,
            routing_version: 1,
            pricing_version: 2,
            image_pricing_version: null,
            groups: [],
            models: [],
          },
        })
      }
      if (
        (!signedIn || pathname === '/api/status') &&
        Object.hasOwn(publicReads, pathname)
      ) {
        return route.fulfill({
          json: { success: true, data: publicReads[pathname] },
        })
      }
      if (!/^\/(?:api|core|v1|mj|pg|mcp)(?:\/|$)/.test(pathname)) {
        return route.continue()
      }
    }
    errors.push(`Unexpected local request: ${request.method()} ${pathname}`)
    return route.abort('blockedbyclient')
  })
}

async function checkCompactHeader(page, width) {
  const header = page.locator('header nav').first()
  for (const name of ['切换主题', '主题选项', '个人资料', '切换导航菜单']) {
    const button = header.getByRole('button', { name, exact: true })
    const rect = await button.boundingBox()
    assert.ok(
      rect &&
        rect.width >= 44 &&
        rect.height >= 44 &&
        rect.x >= 0 &&
        rect.x + rect.width <= width,
      `${name} must remain a fully visible 44px target: ${JSON.stringify(rect)}`
    )
  }
  const menu = header.getByRole('button', { name: '切换导航菜单', exact: true })
  await menu.click()
  await page.locator('#public-mobile-navigation[aria-hidden="false"]').waitFor()
  await page.keyboard.press('Escape')
  await page
    .locator('#public-mobile-navigation[aria-hidden="true"]')
    .waitFor({ state: 'attached' })
}

async function review(signedIn, width, theme) {
  const origin = signedIn ? consoleOrigin : publicOrigin
  const phone = width < 640
  const language = phone ? 'zh-CN' : 'en'
  const label = phone ? '返回控制台' : 'Back to Dashboard'
  const name = [signedIn ? 'member' : 'guest', width, theme].join('-')
  const context = await browser.newContext({
    viewport: { width, height: phone ? 844 : 900 },
    locale: language,
    isMobile: phone,
    hasTouch: phone,
    colorScheme: theme,
    reducedMotion: 'reduce',
    serviceWorkers: 'block',
  })
  try {
    await context.addCookies([
      { name: 'vite-ui-theme', value: theme, url: origin },
    ])
    await context.addInitScript((locale) => {
      if (location.protocol !== 'http:') return
      localStorage.setItem('i18nextLng', locale)
      localStorage.setItem('lmm:source-consent:v2', 'no')
    }, language)
    const errors = []
    await isolate(context, origin, signedIn, errors)
    const page = await context.newPage()
    page.on('pageerror', (error) => errors.push(error.message))
    const back = page.getByRole('link', { name: label, exact: true })
    if (!signedIn) {
      await page.goto(origin)
      await page.locator('.lmm-home').waitFor()
      await page.locator('header nav a[href="/"]').first().waitFor()
      assert.equal(await back.count(), 0, `${name} public homepage`)
      assert.equal(await page.getByTestId('persona-debug-trigger').count(), 0)
    }
    const pages = width === 320 ? ['/developers'] : ['/developers', '/webmcp']
    if (!signedIn && theme === 'light') pages.push('/privacy-policy')
    for (const route of pages) {
      const url = new URL(route, origin)
      if (signedIn) url.search = 'debug_persona=l1&console_review=1'
      await page.goto(url.href, { waitUntil: 'domcontentloaded' })
      if (signedIn) await page.getByTestId('persona-debug-trigger').waitFor()
      if (route === '/developers') {
        await page.locator('#request-builder-title').waitFor()
      }
      if (route === '/webmcp') {
        await page
          .getByRole('heading', { name: 'WebMCP', exact: true })
          .waitFor()
      }
      if (route === '/privacy-policy') {
        await page.getByText('Public document.', { exact: true }).waitFor()
      }
      assert.equal(new URL(page.url()).pathname.replace(/\/$/, ''), route)
      await back.waitFor({ state: 'visible' })
      assert.equal(
        (await back.innerText()).trim(),
        label,
        `${name} visible text`
      )
      const box = await back.boundingBox()
      assert.ok(
        box &&
          box.x >= 0 &&
          box.x < width / 3 &&
          box.y >= 0 &&
          box.y < 120 &&
          box.x + box.width <= width,
        `${name} visible upper-left position`
      )
      assert.equal(
        await page
          .locator('html')
          .evaluate((node) => node.classList.contains('dark')),
        theme === 'dark'
      )
      const documentFits = await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth + 1
      )
      assert.ok(documentFits, `${name} horizontal overflow`)
      if (output && route === '/developers') {
        await page.screenshot({
          path: path.join(output, `${name}-developers.png`),
          animations: 'disabled',
        })
      }
      if (width === 320) await checkCompactHeader(page, width)
      await back.click()
      if (signedIn) {
        await page.waitForURL((location) => location.pathname === dashboard)
        await page
          .locator('[data-slot="sidebar-inset"] .console-section-content')
          .waitFor()
      } else {
        await page.waitForURL((location) => location.pathname === '/sign-in')
        assert.equal(
          new URL(page.url()).searchParams.get('redirect'),
          dashboard
        )
        await page.locator('.auth-form-stage form input').first().waitFor()
      }
      assert.deepEqual(errors, [], `${name} ${route}`)
      console.log('PASS', name, route, '→', signedIn ? dashboard : '/sign-in')
    }
  } finally {
    await context.close()
  }
}

try {
  await review(true, 320, 'light')
  for (const signedIn of [false, true]) {
    for (const phone of [false, true]) {
      for (const theme of ['light', 'dark']) {
        await review(signedIn, phone ? 390 : 1440, theme)
      }
    }
  }
} finally {
  await browser.close()
}
