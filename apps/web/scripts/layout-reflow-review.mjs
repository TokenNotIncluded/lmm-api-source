/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const output = process.env.LAYOUT_REVIEW_OUTPUT
assert.ok(output, 'LAYOUT_REVIEW_OUTPUT is required')
await mkdir(output, { recursive: true })

function localOrigin(value) {
  const url = new URL(value)
  assert.equal(url.protocol, 'http:')
  assert.equal(url.hostname, '127.0.0.1')
  assert.equal(url.username + url.password + url.search + url.hash, '')
  assert.equal(url.pathname, '/')
  return url.origin
}

const consoleOrigin = localOrigin(
  process.env.LAYOUT_CONSOLE_ORIGIN ?? 'http://127.0.0.1:4174'
)
const publicOrigin = localOrigin(
  process.env.LAYOUT_PUBLIC_ORIGIN ?? 'http://127.0.0.1:4175'
)
assert.notEqual(
  consoleOrigin,
  publicOrigin,
  'Use separate debug and production servers'
)
const registry = await readFile(
  new URL(
    '../src/features/system-settings/site/section-registry.tsx',
    import.meta.url
  ),
  'utf8'
)
assert.match(registry, /id: 'system-info'/)

const targets = [
  {
    path: '/keys',
    persona: 'l1',
    ready: '.console-section-content',
    filters: true,
  },
  {
    path: '/users',
    persona: 'admin',
    ready: '.console-section-content',
    filters: true,
  },
  { path: '/wallet', persona: 'l1', ready: '#wallet-checkout' },
  { path: '/developers', persona: 'l1', ready: '#request-builder-title' },
  {
    path: '/system-settings/site/system-info',
    persona: 'admin',
    ready: 'input[name="SystemName"]',
  },
  { path: '/sign-in', ready: '.auth-form-stage form input' },
]
const allScenarios = [
  { id: 'desktop', width: 1440, height: 900 },
  { id: 'phone', width: 390, height: 844 },
  { id: 'compact', width: 320, height: 740 },
  { id: 'short', width: 844, height: 390 },
  { id: 'phone-dark', width: 390, height: 844, theme: 'dark' },
  { id: 'root-text-200', width: 1440, height: 900, rootFontScale: 2 },
  ...[1.25, 1.5, 1.75, 2, 4].map((zoom) => ({
    id: `browser-zoom-${zoom * 100}`,
    width: zoom === 4 ? 1280 : 1440,
    height: 900,
    zoom,
  })),
]
const selectedScenarios = process.env.LAYOUT_REVIEW_SCENARIOS?.split(',')
if (selectedScenarios) {
  assert.ok(
    selectedScenarios.every((id) =>
      allScenarios.some((item) => item.id === id)
    ),
    'LAYOUT_REVIEW_SCENARIOS must contain known scenario IDs'
  )
}
const scenarios = selectedScenarios
  ? allScenarios.filter((item) => selectedScenarios.includes(item.id))
  : allScenarios
const revision = execFileSync('git', ['rev-parse', 'HEAD'], {
  encoding: 'utf8',
}).trim()
await writeFile(path.join(output, 'revision.txt'), `${revision}\n`)
const report = {
  schema: 'lmm-layout-reflow-review.v1',
  revision,
  startedAt: new Date().toISOString(),
  scope:
    'Local synthetic layout self-test; no production account, API or payment',
  sources: {
    console:
      'src/features/debug/persona-runtime.ts; scripts/console-page-review.mjs',
    public:
      'Production preview with scripts/mobile-experience-review.mjs public fixtures',
    zoom: 'https://developer.chrome.com/docs/extensions/reference/api/tabs#method-setZoom',
    extensions:
      'https://github.com/microsoft/playwright/blob/v1.56.1/docs/src/chrome-extensions-js-python.md',
  },
  scenarios,
  targets,
  cases: [],
}
async function saveReport() {
  await writeFile(
    path.join(output, 'report.json'),
    JSON.stringify(report, null, 2)
  )
}
await saveReport()

const statusFixture = {
  system_name: 'LMM API',
  logo: '/logo.png',
  register_enabled: true,
  password_register_enabled: true,
  password_login_enabled: true,
  email_verification: false,
  github_oauth: true,
  github_client_id: 'fixture-client',
  assistant: { enabled: false },
  announcements_enabled: false,
  backend_capabilities: { bounty_public_read: false },
}
const documentFixture =
  '# 使用说明\n\n## 账号\n使用自己的账号。\n\n## 密钥\n不要分享密码和密钥。'

// The extension exists only in an isolated temporary browser profile. Chrome's
// automatic/per-tab zoom changes the browser layout, and is read back below.
// Root font overrides and viewport emulation are reported as separate methods.
async function createZoomExtension(directory) {
  await mkdir(directory, { recursive: true })
  await writeFile(
    path.join(directory, 'manifest.json'),
    JSON.stringify({
      manifest_version: 3,
      name: 'Local layout review zoom',
      version: '1.0',
      host_permissions: ['http://127.0.0.1/*'],
      background: { service_worker: 'background.js' },
    })
  )
  await writeFile(
    path.join(directory, 'background.js'),
    'chrome.runtime.onInstalled.addListener(() => {})\n'
  )
  await writeFile(
    path.join(directory, 'control.html'),
    '<!doctype html><title>Local layout review zoom</title>'
  )
}

async function browserMetrics(page) {
  return page.evaluate(() => ({
    width: innerWidth,
    height: innerHeight,
    dpr: devicePixelRatio,
    visualViewportScale: visualViewport?.scale ?? null,
    rootFontPx: Number.parseFloat(
      getComputedStyle(document.documentElement).fontSize
    ),
  }))
}

async function setBrowserZoom(control, page, factor) {
  const state = await control.evaluate(
    async ({ url, factor }) => {
      const tabs = globalThis.chrome.tabs
      const matches = (await tabs.query({})).filter((tab) => tab.url === url)
      if (matches.length !== 1 || matches[0].id == null) {
        throw new Error(
          `Expected one local target tab; found ${matches.length}`
        )
      }
      const tabId = matches[0].id
      await tabs.setZoomSettings(tabId, { mode: 'automatic', scope: 'per-tab' })
      await tabs.setZoom(tabId, factor)
      return {
        actualFactor: await tabs.getZoom(tabId),
        settings: await tabs.getZoomSettings(tabId),
      }
    },
    { url: page.url(), factor }
  )
  assert.ok(
    Math.abs(state.actualFactor - factor) < 0.001,
    JSON.stringify(state)
  )
  assert.equal(state.settings.mode, 'automatic')
  assert.equal(state.settings.scope, 'per-tab')
  await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve))
      )
  )
  return state
}

async function settle(page) {
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(600)
}

async function measureLayout(page) {
  return page.evaluate(() => {
    const rendered = (node) => {
      const rect = node.getBoundingClientRect()
      const style = getComputedStyle(node)
      return (
        rect.width > 2 &&
        rect.height > 2 &&
        style.display !== 'none' &&
        style.visibility !== 'hidden' &&
        !node.closest('[hidden], [inert], [aria-hidden="true"]')
      )
    }
    const identify = (node) => ({
      tag: node.tagName.toLowerCase(),
      slot: node.getAttribute('data-slot'),
      name:
        node.getAttribute('aria-label') ??
        node.getAttribute('name') ??
        node.textContent.trim().slice(0, 100),
    })
    const inViewport = (node) => {
      const rect = node.getBoundingClientRect()
      return (
        rect.bottom > 0 &&
        rect.top < innerHeight &&
        rect.right > 0 &&
        rect.left < innerWidth
      )
    }
    const horizontalScroller = (node) => {
      for (
        let parent = node.parentElement;
        parent;
        parent = parent.parentElement
      ) {
        const overflow = getComputedStyle(parent).overflowX
        if (
          ['auto', 'scroll'].includes(overflow) &&
          parent.scrollWidth > parent.clientWidth + 1
        ) {
          return true
        }
      }
      return false
    }
    const regions = [
      ...document.querySelectorAll(
        'main, .console-section-content, .auth-form-stage, form'
      ),
    ]
      .filter(rendered)
      .map((node) => ({
        ...identify(node),
        width: node.clientWidth,
        scrollWidth: node.scrollWidth,
      }))
    const controls = [
      ...document.querySelectorAll(
        'button, input:not([type="hidden"]), select, textarea, summary, [role="tab"], [role="combobox"]'
      ),
    ]
      .filter((node) => rendered(node) && inViewport(node))
      .map((node) => {
        const rect = node.getBoundingClientRect()
        const region = node.closest(
          '.console-section-content, .console-section-header, .console-section-footer, .auth-form-stage'
        )
        const style = region ? getComputedStyle(region) : null
        const availableWidth = region
          ? region.clientWidth -
            Number.parseFloat(style.paddingLeft) -
            Number.parseFloat(style.paddingRight)
          : document.documentElement.clientWidth
        return {
          ...identify(node),
          width: rect.width,
          height: rect.height,
          left: rect.left,
          right: rect.right,
          availableWidth,
          overwide: rect.width > availableWidth + 2,
          outsideViewport:
            !horizontalScroller(node) &&
            (rect.left < -2 || rect.right > innerWidth + 2),
        }
      })
    const header = document.querySelector('.console-section-header')
    const content = document.querySelector('.console-section-content')
    const footer = document.querySelector('.console-section-footer')
    const leftEdge = (node) =>
      node.getBoundingClientRect().left +
      Number.parseFloat(getComputedStyle(node).paddingLeft)
    const alignment =
      header && content && rendered(header) && rendered(content)
        ? {
            headerLeft: leftEdge(header),
            contentLeft: leftEdge(content),
            delta: Math.abs(leftEdge(header) - leftEdge(content)),
            footerDelta:
              footer && rendered(footer)
                ? Math.abs(leftEdge(footer) - leftEdge(content))
                : null,
          }
        : null
    const scrollRoots = [
      ...new Set([
        document.scrollingElement,
        ...document.querySelectorAll(
          'main, .console-section-content, [data-mobile-scroll-root], [data-slot="data-table-viewport"] *'
        ),
      ]),
    ]
      .filter(
        (node) =>
          node && rendered(node) && node.scrollHeight > node.clientHeight + 2
      )
      .map((node) => ({
        ...identify(node),
        height: node.clientHeight,
        scrollHeight: node.scrollHeight,
        overflowY: getComputedStyle(node).overflowY,
      }))
    return {
      width: innerWidth,
      height: innerHeight,
      scrollWidth: Math.max(
        document.documentElement.scrollWidth,
        document.body.scrollWidth
      ),
      regions,
      controls,
      alignment,
      scrollRoots,
    }
  })
}

function layoutIssues(metrics) {
  const issues = []
  if (metrics.scrollWidth > metrics.width + 2) {
    issues.push(`Document overflow: ${metrics.scrollWidth} > ${metrics.width}`)
  }
  for (const region of metrics.regions) {
    if (region.scrollWidth > region.width + 2) {
      issues.push(`Region overflow: ${JSON.stringify(region)}`)
    }
  }
  for (const control of metrics.controls) {
    if (control.overwide || control.outsideViewport) {
      issues.push(`Control overflow: ${JSON.stringify(control)}`)
    }
  }
  if (
    metrics.alignment &&
    (metrics.alignment.delta > 2 || metrics.alignment.footerDelta > 2)
  ) {
    issues.push(
      `Header/content/footer alignment: ${JSON.stringify(metrics.alignment)}`
    )
  }
  return issues
}

async function captureScreenshot(page, options) {
  if (page.viewportSize() !== null) return page.screenshot(options)
  // At native tab zoom, Playwright clips its full-page screenshot to CSS pixels.
  // Capture the real browser surface so the evidence includes the whole viewport.
  const session = await page.context().newCDPSession(page)
  try {
    const { data } = await session.send('Page.captureScreenshot', {
      format: 'png',
      fromSurface: true,
      captureBeyondViewport: false,
    })
    const png = Buffer.from(data, 'base64')
    const expected = await page.evaluate(() => ({
      width: innerWidth * devicePixelRatio,
      height: innerHeight * devicePixelRatio,
      tolerance: Math.ceil(devicePixelRatio),
    }))
    assert.ok(
      Math.abs(png.readUInt32BE(16) - expected.width) <= expected.tolerance &&
        Math.abs(png.readUInt32BE(20) - expected.height) <= expected.tolerance,
      'Native zoom screenshot must include the full browser viewport'
    )
    await writeFile(options.path, png)
  } finally {
    await session.detach()
  }
}

async function capture(page, entry, state) {
  await settle(page)
  const metrics = await measureLayout(page)
  const screenshot = `${entry.id}-${state}.png`
  await captureScreenshot(page, {
    path: path.join(output, screenshot),
    fullPage: true,
    animations: 'disabled',
  })
  entry.states.push({ state, screenshot, metrics })
  entry.issues.push(
    ...layoutIssues(metrics).map((issue) => `${state}: ${issue}`)
  )
}

async function checkReachability(page, entry) {
  const groups = [
    [
      'content-end',
      'main button:visible:not(:disabled), main input:visible:not(:disabled), main textarea:visible:not(:disabled), main a[href]:visible',
    ],
    [
      'form-end',
      'main form button:visible:not(:disabled), main form input:visible:not(:disabled), main form textarea:visible:not(:disabled)',
    ],
    [
      'footer',
      '.console-section-footer button:visible:not(:disabled), .console-section-footer [role="combobox"]:visible:not(:disabled)',
    ],
  ]
  for (const [kind, selector] of groups) {
    const candidates = page.locator(selector)
    const lastRendered = await candidates.evaluateAll((nodes) =>
      nodes.findLastIndex((node) => {
        const rect = node.getBoundingClientRect()
        return (
          rect.width > 2 &&
          rect.height > 2 &&
          !node.closest('[hidden], [inert], [aria-hidden="true"]')
        )
      })
    )
    if (lastRendered === -1) continue
    const target = candidates.nth(lastRendered)
    try {
      await target.scrollIntoViewIfNeeded({ timeout: 5000 })
      await target.focus()
      await page.waitForTimeout(150)
      const result = await target.evaluate((node) => {
        const rect = node.getBoundingClientRect()
        let left = Math.max(0, rect.left),
          right = Math.min(innerWidth, rect.right)
        let top = Math.max(0, rect.top),
          bottom = Math.min(innerHeight, rect.bottom)
        for (
          let parent = node.parentElement;
          parent;
          parent = parent.parentElement
        ) {
          const style = getComputedStyle(parent)
          const bound = parent.getBoundingClientRect()
          if (['auto', 'scroll', 'hidden', 'clip'].includes(style.overflowX)) {
            left = Math.max(left, bound.left)
            right = Math.min(right, bound.right)
          }
          if (['auto', 'scroll', 'hidden', 'clip'].includes(style.overflowY)) {
            top = Math.max(top, bound.top)
            bottom = Math.min(bottom, bound.bottom)
          }
        }
        const hit =
          right > left && bottom > top
            ? document.elementFromPoint((left + right) / 2, (top + bottom) / 2)
            : null
        return {
          name:
            node.getAttribute('aria-label') ??
            node.getAttribute('name') ??
            node.textContent.trim().slice(0, 100),
          reachable: Boolean(hit && (hit === node || node.contains(hit))),
          focused: document.activeElement === node,
          rect: {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
          },
        }
      })
      entry.reachability.push({ kind, ...result })
      if (!result.reachable || !result.focused) {
        entry.issues.push(`${kind} is not reachable: ${JSON.stringify(result)}`)
      }
    } catch (error) {
      entry.reachability.push({ kind, error: String(error) })
      entry.issues.push(`${kind}: ${String(error)}`)
    }
  }
  const screenshot = `${entry.id}-end.png`
  await captureScreenshot(page, {
    path: path.join(output, screenshot),
    animations: 'disabled',
  })
  entry.endScreenshot = screenshot
}

const temporary = await mkdtemp(path.join(tmpdir(), 'lmm-reflow-'))
const extensionDirectory = path.join(temporary, 'zoom-extension')
await createZoomExtension(extensionDirectory)
try {
  for (const scenario of scenarios) {
    let context
    let currentEntry
    try {
      context = await chromium.launchPersistentContext(
        path.join(temporary, scenario.id),
        {
          ...(process.env.PLAYWRIGHT_CHROME_EXECUTABLE
            ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE }
            : { channel: 'chromium' }),
          headless: true,
          viewport: scenario.zoom
            ? null
            : { width: scenario.width, height: scenario.height },
          reducedMotion: 'reduce',
          locale: 'zh-CN',
          args: [
            `--window-size=${scenario.width},${scenario.height}`,
            `--disable-extensions-except=${extensionDirectory}`,
            `--load-extension=${extensionDirectory}`,
          ],
        }
      )
      const worker =
        context.serviceWorkers()[0] ??
        (await context.waitForEvent('serviceworker', { timeout: 15000 }))
      const extensionId = new URL(worker.url()).host
      const control = await context.newPage()
      await control.goto(`chrome-extension://${extensionId}/control.html`)
      await context.addInitScript(() => {
        localStorage.setItem('i18nextLng', 'zh-CN')
        localStorage.setItem('lmm:source-consent:v2', 'no')
      })
      for (const origin of [consoleOrigin, publicOrigin]) {
        await context.addCookies([
          {
            name: 'vite-ui-theme',
            value: scenario.theme ?? 'light',
            url: origin,
          },
        ])
      }
      await context.route('**/*', async (route) => {
        const request = route.request()
        const url = new URL(request.url())
        if (url.protocol === 'chrome-extension:' && url.host === extensionId) {
          return route.continue()
        }
        // Analytics is outside this layout fixture. Serve only this known script
        // locally; all other external requests remain failures and never leave.
        if (
          request.method() === 'GET' &&
          request.resourceType() === 'script' &&
          url.href === 'https://cdn.agentlane.com/v1/snippet.js'
        ) {
          currentEntry?.isolatedScripts.push(url.href)
          return route.fulfill({
            contentType: 'text/javascript',
            body: '/* Analytics disabled in the isolated layout review. */',
          })
        }
        if (![consoleOrigin, publicOrigin].includes(url.origin)) {
          currentEntry?.network.push(
            `Blocked external request: ${request.method()} ${url.origin}${url.pathname}`
          )
          return route.abort('blockedbyclient')
        }
        if (!/^\/(?:api|core|v1|mj|pg)(?:\/|$)/.test(url.pathname)) {
          return route.continue()
        }
        if (url.pathname === '/api/status' && request.method() === 'GET') {
          return route.fulfill({ json: { success: true, data: statusFixture } })
        }
        if (url.origin === publicOrigin) {
          if (
            url.pathname === '/api/user/auth/refresh' &&
            request.method() === 'POST'
          ) {
            return route.fulfill({ status: 401, json: { success: false } })
          }
          const fixtures = {
            '/api/setup': { status: true },
            '/api/site/navigation': { value: '{}', revision: 1 },
            '/api/notice': '',
            '/api/user-agreement': documentFixture,
            '/api/privacy-policy': documentFixture,
            '/api/about': documentFixture,
          }
          if (
            request.method() === 'GET' &&
            Object.hasOwn(fixtures, url.pathname)
          ) {
            return route.fulfill({
              json: { success: true, data: fixtures[url.pathname] },
            })
          }
        }
        currentEntry?.network.push(
          `Unmocked local API: ${request.method()} ${url.pathname}`
        )
        return route.abort('blockedbyclient')
      })
      await context.routeWebSocket('**/*', (socket) => {
        const url = new URL(socket.url())
        if (
          url.protocol === 'ws:' &&
          url.host === new URL(consoleOrigin).host
        ) {
          return socket.connectToServer()
        }
        currentEntry?.network.push(
          `Blocked WebSocket: ${url.origin}${url.pathname}`
        )
        socket.close({ code: 1008, reason: 'Local review only' })
      })

      for (const target of targets) {
        const entry = {
          id: `${scenario.id}-${target.path.slice(1).replaceAll('/', '-')}`,
          scenario: scenario.id,
          path: target.path,
          persona: target.persona ?? null,
          method: scenario.zoom
            ? 'chrome.tabs.setZoom'
            : scenario.rootFontScale
              ? 'root-font-size-override'
              : 'viewport',
          issues: [],
          network: [],
          isolatedScripts: [],
          screenshotMode: scenario.zoom
            ? 'browser-surface-viewport'
            : 'full-page',
          pageErrors: [],
          consoleErrors: [],
          states: [],
          reachability: [],
        }
        currentEntry = entry
        const page = await context.newPage()
        page.setDefaultTimeout(10000)
        page.on('pageerror', (error) =>
          entry.pageErrors.push(String(error.stack ?? error))
        )
        page.on('console', (message) => {
          if (message.type() === 'error') {
            entry.consoleErrors.push(message.text())
          }
        })
        try {
          const url = new URL(
            target.path === '/users' ? '/dashboard/overview' : target.path,
            target.persona ? consoleOrigin : publicOrigin
          )
          if (target.persona) {
            url.searchParams.set('debug_persona', target.persona)
            url.searchParams.set('console_review', '1')
          }
          await page.goto(url.href, {
            waitUntil: 'domcontentloaded',
            timeout: 60000,
          })
          await page.bringToFront()
          if (target.persona) {
            await page
              .getByTestId('persona-debug-trigger')
              .waitFor({ timeout: 60000 })
          }
          // The users route deliberately rejects unknown search parameters.
          // Initialize the development persona first, then use the real clean
          // route without weakening the production search contract.
          if (target.path === '/users') {
            await page.evaluate((destination) => {
              history.pushState({}, '', destination)
              window.dispatchEvent(new PopStateEvent('popstate'))
            }, target.path)
            await page
              .getByRole('heading', { name: '用户', exact: true })
              .waitFor()
          }
          await page
            .locator(target.ready)
            .first()
            .waitFor({ state: 'visible', timeout: 60000 })
          await settle(page)
          assert.equal(
            new URL(page.url()).pathname.replace(/\/$/, ''),
            target.path
          )
          entry.url = page.url()
          entry.before = await browserMetrics(page)
          if (scenario.zoom) {
            entry.zoom = {
              requestedFactor: scenario.zoom,
              ...(await setBrowserZoom(control, page, scenario.zoom)),
            }
            entry.after = await browserMetrics(page)
            assert.ok(
              Math.abs(
                entry.after.width - entry.before.width / scenario.zoom
              ) <= 2,
              `Native zoom did not change the CSS viewport as expected: ${JSON.stringify({ before: entry.before, after: entry.after })}`
            )
            if (scenario.zoom === 4) {
              assert.ok(
                entry.after.width <= 320,
                '400% scenario must reach at most 320 CSS px'
              )
            }
          } else if (scenario.rootFontScale) {
            await page.addStyleTag({
              content: `html { font-size: ${entry.before.rootFontPx * scenario.rootFontScale}px !important; }`,
            })
            entry.after = await browserMetrics(page)
            assert.ok(
              Math.abs(
                entry.after.rootFontPx -
                  entry.before.rootFontPx * scenario.rootFontScale
              ) < 0.1
            )
          }
          entry.titles = await page
            .locator('h1:visible, h2:visible')
            .allTextContents()
          const body = await page.locator('body').innerText()
          if (
            (await page.locator('.error-editorial[data-status]').count()) ||
            /PERSONA_DEBUG_UNMOCKED_REQUEST|Internal Server Error|Something went wrong|糟糕！出错了/.test(
              body
            )
          ) {
            entry.issues.push('Page displays a runtime failure')
          }
          await capture(page, entry, 'initial')
          if (target.filters) {
            const toggle = page
              .locator('.console-toolbar button[aria-controls]')
              .first()
            if (await toggle.count()) {
              await toggle.click()
              assert.equal(await toggle.getAttribute('aria-expanded'), 'true')
              await capture(page, entry, 'filters')
              await toggle.click()
            }
          }
          await checkReachability(page, entry)
        } catch (error) {
          entry.issues.push(String(error.stack ?? error))
          try {
            const screenshot = `${entry.id}-failed.png`
            await captureScreenshot(page, {
              path: path.join(output, screenshot),
              fullPage: true,
              animations: 'disabled',
            })
            entry.failureScreenshot = screenshot
          } catch (screenshotError) {
            entry.screenshotError = String(screenshotError)
          }
        } finally {
          entry.issues.push(...entry.network, ...entry.pageErrors)
          entry.ok = entry.issues.length === 0
          report.cases.push(entry)
          await saveReport()
          console.log(
            JSON.stringify({ id: entry.id, ok: entry.ok, issues: entry.issues })
          )
          await page.close()
          currentEntry = null
        }
      }
    } catch (error) {
      for (const target of targets) {
        if (
          report.cases.some(
            (entry) =>
              entry.scenario === scenario.id && entry.path === target.path
          )
        ) {
          continue
        }
        report.cases.push({
          scenario: scenario.id,
          path: target.path,
          ok: false,
          issues: [`Browser setup: ${String(error.stack ?? error)}`],
        })
      }
      await saveReport()
    } finally {
      if (context) {
        try {
          await context.close()
        } catch (error) {
          report.cases.push({
            scenario: scenario.id,
            path: null,
            ok: false,
            issues: [`Browser cleanup: ${String(error)}`],
          })
          await saveReport()
        }
      }
    }
  }
} finally {
  report.finishedAt = new Date().toISOString()
  report.summary = {
    total: report.cases.length,
    failed: report.cases.filter((entry) => !entry.ok).length,
  }
  await saveReport()
  await rm(temporary, { recursive: true, force: true })
}
console.log(JSON.stringify(report.summary))
assert.equal(report.cases.length, scenarios.length * targets.length)
assert.equal(
  report.summary.failed,
  0,
  'Inspect report.json, screenshots and review.log for failed layout cases'
)
