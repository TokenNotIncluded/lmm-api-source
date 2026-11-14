/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdir, mkdtemp, open, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'

const web = path.resolve(import.meta.dirname, '..')
const output = process.env.LOGS_REVIEW_OUTPUT
if (!output) throw new Error('LOGS_REVIEW_OUTPUT is required')
await mkdir(output, { recursive: true })
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const origin = 'http://127.0.0.1:4196'
const serverLog = await open(path.join(output, 'dev-server.log'), 'w')
const server = spawn(
  process.execPath,
  [path.join(web, 'node_modules/.bin/rsbuild'), 'dev', '--port', '4196'],
  {
    cwd: web,
    env: { ...process.env, LMM_ENABLE_PERSONA_DEBUG: '1' },
    stdio: ['ignore', serverLog.fd, serverLog.fd],
  }
)
const report = []
let browser
let activeContext
let failure
const temporary = await mkdtemp(path.join(tmpdir(), 'lmm-logs-review-'))
const extension = path.join(temporary, 'zoom-extension')
await mkdir(extension)
await writeFile(
  path.join(extension, 'manifest.json'),
  JSON.stringify({
    manifest_version: 3,
    name: 'Local usage logs review zoom',
    version: '1.0',
    host_permissions: ['http://127.0.0.1/*'],
    background: { service_worker: 'background.js' },
  })
)
await writeFile(
  path.join(extension, 'background.js'),
  'chrome.runtime.onInstalled.addListener(() => {})\n'
)
await writeFile(
  path.join(extension, 'control.html'),
  '<!doctype html><title>Local usage logs review zoom</title>'
)
try {
  let ready = false
  for (let attempt = 0; attempt < 240; attempt += 1) {
    if (server.exitCode !== null) {
      throw new Error(
        await readFile(path.join(output, 'dev-server.log'), 'utf8')
      )
    }
    try {
      const response = await fetch(origin, {
        signal: AbortSignal.timeout(1000),
      })
      if (response.ok) {
        ready = true
        break
      }
    } catch {
      /* The child server is still compiling. */
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  assert.ok(ready, 'Local review server did not become ready')
  console.log('Local usage-log review server ready')
  browser = await chromium.launch({
    headless: true,
    ...(process.env.PLAYWRIGHT_CHROME_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE }
      : {}),
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  })
  for (const scenario of [
    {
      name: 'desktop-light',
      width: 1440,
      height: 1000,
      theme: 'light',
      scale: 1,
    },
    {
      name: 'desktop-dark',
      width: 1440,
      height: 1000,
      theme: 'dark',
      scale: 1,
    },
    { name: 'mobile', width: 390, height: 844, theme: 'light', scale: 1 },
    { name: 'narrow', width: 320, height: 844, theme: 'light', scale: 1 },
    {
      name: 'compact-high-density',
      width: 720,
      height: 500,
      theme: 'light',
      scale: 2,
    },
    {
      name: 'root-font-200',
      width: 1440,
      height: 1000,
      theme: 'light',
      scale: 1,
      rootFontScale: 2,
    },
    {
      name: 'native-zoom-200',
      width: 1440,
      height: 1000,
      theme: 'light',
      scale: 1,
      zoom: 2,
    },
  ]) {
    const options = {
      viewport: { width: scenario.width, height: scenario.height },
      deviceScaleFactor: scenario.scale,
      colorScheme: scenario.theme,
      reducedMotion: 'reduce',
      locale: 'zh-CN',
      serviceWorkers: 'block',
    }
    if (scenario.zoom) {
      await browser.close()
      browser = undefined
    }
    const context = scenario.zoom
      ? await chromium.launchPersistentContext(
          path.join(temporary, 'profile'),
          {
            ...options,
            viewport: null,
          deviceScaleFactor: undefined,
            serviceWorkers: 'allow',
            headless: true,
            ...(process.env.PLAYWRIGHT_CHROME_EXECUTABLE
              ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE }
              : { channel: 'chromium' }),
            args: [
              '--no-sandbox',
              '--disable-dev-shm-usage',
              `--window-size=${scenario.width},${scenario.height}`,
              `--disable-extensions-except=${extension}`,
              `--load-extension=${extension}`,
            ],
          }
        )
      : await browser.newContext(options)
    activeContext = context
    let control
    let extensionId
    if (scenario.zoom) {
      const worker =
        context.serviceWorkers()[0] ??
        (await context.waitForEvent('serviceworker', { timeout: 15000 }))
      extensionId = new URL(worker.url()).host
      control = await context.newPage()
      await control.goto(`chrome-extension://${extensionId}/control.html`)
    }
    await context.addInitScript(() => {
      localStorage.setItem('i18nextLng', 'zh-CN')
      localStorage.setItem('lmm:source-consent:v2', 'no')
    })
    const errors = []
    await context.route('**/*', async (route) => {
      const url = new URL(route.request().url())
      if (url.protocol === 'chrome-extension:' && url.host === extensionId) {
        return route.continue()
      }
      if (url.origin !== origin) return route.abort('blockedbyclient')
      if (url.pathname === '/api/status') {
        return route.fulfill({
          json: {
            success: true,
            data: { system_name: 'LMM API', announcements_enabled: false },
          },
        })
      }
      if (/^\/(api|core)\//.test(url.pathname)) {
        errors.push(`Unexpected network request: ${url.pathname}`)
        return route.abort('blockedbyclient')
      }
      return route.continue()
    })
    const page = await context.newPage()
    page.setDefaultTimeout(30000)
    page.on('pageerror', (error) => errors.push(String(error)))
    await page.goto(
      `${origin}/usage-logs/common?debug_persona=admin&console_review=1&logs_review=1`,
      { waitUntil: 'domcontentloaded' }
    )
    await page.getByTestId('persona-debug-trigger').waitFor()
    await page.getByText('deepseek-chat', { exact: true }).first().waitFor()
    let zoomReadback
    if (scenario.zoom) {
      const baselineWidth = await page.evaluate(() => innerWidth)
      zoomReadback = await control.evaluate(
        async ({ url, factor }) => {
          const tabs = globalThis.chrome.tabs
          const matches = (await tabs.query({})).filter(
            (tab) => tab.url === url
          )
          assertSingleTab(matches)
          const tabId = matches[0].id
          await tabs.setZoomSettings(tabId, {
            mode: 'automatic',
            scope: 'per-tab',
          })
          await tabs.setZoom(tabId, factor)
          return tabs.getZoom(tabId)

          function assertSingleTab(matches) {
            if (matches.length !== 1 || matches[0].id == null) {
              throw new Error(
                `Expected one local usage-log tab; found ${matches.length}`
              )
            }
          }
        },
        { url: page.url(), factor: scenario.zoom }
      )
      assert.equal(zoomReadback, scenario.zoom)
      await page.waitForFunction(
        ({ width, factor }) => Math.abs(innerWidth - width / factor) <= 1,
        { width: baselineWidth, factor: scenario.zoom }
      )
    }
    if (scenario.rootFontScale) {
      await page.evaluate((factor) => {
        const root = document.documentElement
        root.style.fontSize = `${Number.parseFloat(getComputedStyle(root).fontSize) * factor}px`
      }, scenario.rootFontScale)
    }
    await page.evaluate(async (theme) => {
      document.documentElement.classList.toggle('dark', theme === 'dark')
      await document.fonts.ready
      await new Promise(requestAnimationFrame)
      await new Promise(requestAnimationFrame)
    }, scenario.theme)
    const dimensions = await page.evaluate(() => ({
      width: innerWidth,
      scroll: document.documentElement.scrollWidth,
      rootFontPx: Number.parseFloat(
        getComputedStyle(document.documentElement).fontSize
      ),
      devicePixelRatio,
    }))
    const body = await page.locator('body').innerText()
    await page.screenshot({
      path: path.join(output, `${scenario.name}.png`),
      fullPage: true,
      animations: 'disabled',
    })
    await writeFile(path.join(output, `${scenario.name}.txt`), body)
    report.push({ ...scenario, zoomReadback, dimensions, errors })
    assert.ok(
      dimensions.scroll <= dimensions.width + 1,
      JSON.stringify(dimensions)
    )
    assert.ok(
      !/PERSONA_DEBUG_UNMOCKED_REQUEST|Internal Server Error|Failed to load logs|加载日志失败/.test(
        body
      ),
      body
    )
    assert.deepEqual(errors, [])
    console.log(`Captured ${scenario.name}`)
    if (scenario.name === 'desktop-light') {
      await page
        .getByRole('button', { name: /^(View request details|查看请求详情)$/ })
        .first()
        .click()
      const dialog = page.getByRole('dialog')
      await dialog.getByText('demo-request-1001', { exact: true }).waitFor()
      await dialog.getByText('upstream-1001', { exact: true }).waitFor()
      await page.screenshot({
        path: path.join(output, 'request-details.png'),
        animations: 'disabled',
      })
      await page.keyboard.press('Escape')
      await dialog.waitFor({ state: 'hidden' })
      const model = page.getByPlaceholder(/^(Model Name|模型名称)$/)
      await model.fill('claude-sonnet-4-5')
      await model.press('Enter')
      await page
        .getByText('deepseek-chat', { exact: true })
        .first()
        .waitFor({ state: 'hidden' })
      await page
        .getByText('claude-sonnet-4-5', { exact: true })
        .first()
        .waitFor()
      assert.equal(
        new URL(page.url()).searchParams.get('model'),
        'claude-sonnet-4-5'
      )
      report.push({
        interaction: 'scoped detail and model filter',
        passed: true,
      })
      console.log('Verified request details and applied URL filter')
    }
    await context.close()
    activeContext = undefined
  }
} catch (error) {
  failure = String(error.stack ?? error)
  throw error
} finally {
  await writeFile(
    path.join(output, 'result.json'),
    JSON.stringify(
      {
        source: 'synthetic local API fixtures; real application components',
        passed: failure === undefined,
        failure,
        scenarios: report,
      },
      null,
      2
    )
  )
  if (activeContext) await activeContext.close()
  if (browser) await browser.close()
  server.kill('SIGTERM')
  await serverLog.close()
  await rm(temporary, { recursive: true, force: true })
}
