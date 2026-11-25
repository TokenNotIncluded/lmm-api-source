/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// Production bundle, synthetic loopback APIs. Never contacts production.
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFile, writeFile, mkdir, stat } from 'node:fs/promises'
import { createServer } from 'node:http'
import path from 'node:path'

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const web = path.resolve(import.meta.dirname, '..')
const dist = path.resolve(
  process.env.SETTINGS_REVIEW_DIST ?? path.join(web, 'dist')
)
const output = process.env.SETTINGS_REVIEW_OUTPUT
assert.ok(output, 'SETTINGS_REVIEW_OUTPUT is required')
await mkdir(output, { recursive: true })
const translations = JSON.parse(
  await readFile(path.join(web, 'src/i18n/locales/zh-CN.json'), 'utf8')
).translation
const t = (key) => translations[key] ?? key
const mime = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.json': 'application/json',
  '.png': 'image/png',
  '.webp': 'image/webp',
  '.svg': 'image/svg+xml',
  '.woff2': 'font/woff2',
}
const server = createServer(async (req, res) => {
  try {
    const file = path.resolve(
      dist,
      `.${new URL(req.url, 'http://127.0.0.1').pathname}`
    )
    assert.ok(file.startsWith(dist + path.sep) || file === dist)
    const actual = (await stat(file).catch(() => null))?.isFile()
      ? file
      : path.join(dist, 'index.html')
    res.writeHead(200, {
      'content-type': mime[path.extname(actual)] ?? 'application/octet-stream',
      'cache-control': 'no-store',
    })
    res.end(await readFile(actual))
  } catch {
    res.writeHead(404)
    res.end('Not found')
  }
})
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const origin = `http://127.0.0.1:${server.address().port}`
const report = {
  source:
    process.env.SETTINGS_REVIEW_SOURCE ??
    execFileSync('git', ['rev-parse', 'HEAD'], {
      cwd: web,
      encoding: 'utf8',
    }).trim(),
  script_sha256: createHash('sha256')
    .update(await readFile(import.meta.filename))
    .digest('hex'),
  index_sha256: createHash('sha256')
    .update(await readFile(path.join(dist, 'index.html')))
    .digest('hex'),
  scope: 'Built Web and synthetic API fixtures, no production services.',
  checks: [],
  screenshots: [],
  redirects: [],
  page_errors: [],
  unexpected_requests: [],
  writes: [],
}
const now = Math.floor(Date.now() / 1000)
const user = {
  id: 1099,
  username: 'visual_admin',
  display_name: '验收管理员',
  email: 'preview@example.test',
  role: 100,
  status: 1,
  group: 'default',
  developer_access_granted: true,
  quota: 25000000,
  used_quota: 4500000,
  request_count: 100,
  permissions: { docs_access: true },
  onboarding: {
    activation_complete: true,
    credential_complete: true,
    first_request_complete: true,
    stage: 'complete',
  },
  language: 'zh-CN',
}
const bundle = {
  access_token: 'synthetic-local-visual-session',
  token_type: 'Bearer',
  access_expires_at: now + 86400,
  user,
  session: {
    sid: 'settings-closure-local',
    current: true,
    login_method: 'password',
    ip: '127.0.0.1',
    user_agent: 'local-browser-test',
    created_at: now,
    last_active_at: now,
    expires_at: now + 86400,
  },
}
// Keep bookmarked addresses independent of the retired editor implementation.
const legacySections = {
  site: [
    'system-info',
    'notice',
    'header-navigation',
    'sidebar-modules',
    'ai-directory',
    'rss',
    'ai-directory-ads',
  ],
  auth: [
    'humangrant',
    'basic-auth',
    'oauth',
    'passkey',
    'bot-protection',
    'custom-oauth',
  ],
  billing: [
    'quota',
    'currency',
    'model-pricing',
    'router-pricing',
    'group-pricing',
    'payment',
    'checkin',
  ],
  models: [
    'service-tier-pricing',
    'global',
    'routing-reliability',
    'gemini',
    'claude',
    'grok',
    'channel-affinity',
    'model-deployment',
  ],
  security: [
    'honeypot',
    'market-ai-review',
    'moderation',
    'rate-limit',
    'sensitive-words',
    'advanced-security',
    'ip-access-routing',
    'anti-relay',
    'ssrf',
    'token-limits',
  ],
  content: [
    'dashboard',
    'announcements',
    'api-info',
    'faq',
    'uptime-kuma',
    'assistant',
    'chat',
    'drawing',
  ],
  operations: [
    'behavior',
    'alerts',
    'email',
    'worker',
    'hero-sms',
    'logs',
    'performance',
    'raw-json',
    'update-checker',
    'scripts',
  ],
}
const json = (data, extra = {}) => ({
  success: true,
  message: '',
  data,
  ...extra,
})
async function install(context, state, theme) {
  await context.addInitScript(
    ({ theme }) => {
      localStorage.setItem('i18nextLng', 'zh-CN')
      document.cookie = `vite-ui-theme=${theme}; path=/`
    },
    { theme }
  )
  await context.route('**/*', async (route) => {
    const req = route.request()
    const url = new URL(req.url())
    if (url.origin !== origin) {
      await route.abort()
      return
    }
    const p = url.pathname
    if (!p.startsWith('/api/') && !p.startsWith('/core/')) {
      await route.continue()
      return
    }
    const method = req.method()
    state.requests.push({ method, path: p })
    if (
      !['GET', 'HEAD'].includes(method) &&
      !(method === 'POST' && p === '/api/user/auth/refresh')
    ) {
      report.writes.push({ method, path: p })
      await route.fulfill({ status: 405, json: { success: false } })
      return
    }
    let result
    let status = 200
    if (p === '/api/user/auth/refresh') result = json(bundle)
    else if (p === '/api/user/self') result = json(user)
    else if (p === '/api/hero-sms/capabilities') {
      assert.equal(method, 'GET')
      result = json({ purchases_available: false })
    } else if (p === '/api/setup') result = json({ status: true })
    else if (p === '/api/status') {
      result = json({
        system_name: 'LMM',
        version: 'visual-fixture',
        quota_per_unit: 500000,
        register_enabled: false,
        assistant: { enabled: false },
        announcements_enabled: false,
      })
    } else if (p === '/api/site/navigation') {
      result = json({ value: '{}', revision: 1 })
    } else if (p === '/api/extensions/plugins') result = json([])
    else if (p === '/api/channel/ops') {
      result = json({ native_configuration: true, native_secrets: false })
    } else if (p === '/api/option/') {
      result = json([], {
        capabilities: { read_only: true, writable_keys: [] },
      })
    } else if (p === '/api/user/self/announcements') {
      status = state.announcementStatus || 200
      result =
        status === 200
          ? json([])
          : { success: false, message: 'Synthetic announcement failure' }
    } else if (
      ['/api/release-notes/latest', '/api/assistant/handoff/latest'].includes(p)
    ) {
      result = json(null)
    } else if (p === '/api/notice') result = json('')
    else if (p === '/api/todos') {
      result = json({
        items: [],
        page: 1,
        page_size: 50,
        total: 0,
        category: 'all',
        unread_count: 0,
        total_unread_count: 0,
        unread_by_category: {},
        categories: [],
      })
    } else if (p === '/api/ratio-notifications') result = json([])
    else if (p === '/api/user/self/onboarding/todo') {
      result = json({ items: [], count: 0 })
    } else if (p === '/api/assistant/preferences') {
      result = json({ enabled: false })
    } else if (p === '/api/user/self/groups') {
      result = json({ default: { desc: 'Default', ratio: 1 } })
    } else {
      report.unexpected_requests.push({ method, path: p })
      status = 501
      result = { success: false, message: `Unimplemented synthetic API: ${p}` }
    }
    await route.fulfill({
      status,
      contentType: 'application/json',
      body: JSON.stringify(result),
    })
  })
}
let activePage
async function capture(page, name) {
  await page.evaluate(() => document.fonts.ready)
  const details = await page.evaluate(() => ({
    url: location.href,
    width: innerWidth,
    height: innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
    theme: document.documentElement.className,
    body: document.body.innerText.slice(0, 12000),
  }))
  await page.screenshot({
    path: path.join(output, `${name}.png`),
    fullPage: true,
    animations: 'disabled',
  })
  report.screenshots.push({ name, ...details })
  assert.ok(
    details.scrollWidth <= details.width + 1,
    `Document horizontal overflow: ${name}`
  )
}
async function check(name, fn) {
  try {
    await fn()
    report.checks.push({ name, passed: true })
  } catch (error) {
    report.checks.push({ name, passed: false, error: String(error) })
    if (activePage && !activePage.isClosed()) {
      await capture(activePage, `failure-${name}`).catch(() => {})
    }
  }
  await writeFile(
    path.join(output, 'report.json'),
    `${JSON.stringify(report, null, 2)}\n`
  )
  console.log(JSON.stringify(report.checks.at(-1)))
}
const browser = await chromium.launch({
  headless: true,
  ...(process.env.PLAYWRIGHT_CHROME_EXECUTABLE
    ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE }
    : {}),
  args: ['--disable-dev-shm-usage'],
})
report.browser = browser.version()
try {
  for (const width of [1440, 390]) {
    for (const theme of ['light', 'dark']) {
      const label = `${width === 1440 ? 'desktop' : 'mobile'}-${theme}`
      const context = await browser.newContext({
        viewport: { width, height: width === 1440 ? 1000 : 844 },
        locale: 'zh-CN',
        colorScheme: theme,
        reducedMotion: 'reduce',
        serviceWorkers: 'block',
      })
      const state = { requests: [], announcementStatus: 0 }
      await install(context, state, theme)
      const page = await context.newPage()
      page.setDefaultTimeout(10000)
      activePage = page
      page.on('pageerror', (error) =>
        report.page_errors.push({ label, error: String(error) })
      )
      const go = (route) => page.goto(origin + route)
      const btn = (key) =>
        page.getByRole('button', { name: t(key), exact: true })
      const overview = async () => {
        await page.waitForURL(
          (url) => url.pathname.replace(/\/$/, '') === '/system-settings/site'
        )
        await page.getByText('visual-fixture', { exact: true }).waitFor()
        const content = page.locator(
          '[data-section-page] .console-section-content'
        )
        await content
          .getByText('/etc/lmm.conf.d/core.toml', { exact: true })
          .waitFor()
        for (const [field, value] of [
          ['System Name', 'LMM'],
          ['API version', 'visual-fixture'],
          ['Registration Enabled', t('Disabled')],
        ]) {
          const term = content.getByText(t(field), { exact: true })
          assert.equal(await term.locator('+ dd').innerText(), value)
        }
        await content
          .getByRole('link', { name: t('Plugin management') })
          .waitFor()
        assert.equal(
          await content
            .locator(
              'input:not([type=hidden]), textarea, select, button, [role=switch], [contenteditable=true]'
            )
            .count(),
          0
        )
      }
      await check(`${label}-settings-overview`, async () => {
        await go('/system-settings')
        await overview()
        await capture(page, `${label}-settings-overview`)
        const plugins = page
          .locator('[data-section-page] .console-section-content')
          .getByRole('link', { name: t('Plugin management') })
        assert.equal(await plugins.getAttribute('href'), '/extensions/plugins')
        await plugins.click()
        await page.waitForURL(
          (url) => url.pathname.replace(/\/$/, '') === '/extensions/plugins'
        )
        await page
          .getByRole('heading', { name: t('Plugin management'), exact: true })
          .waitFor()
      })
      await check(`${label}-bookmarked-settings`, async () => {
        for (const [category, sections] of Object.entries(legacySections)) {
          const sample =
            width === 1440 && theme === 'light'
              ? sections
              : sections.slice(0, 1)
          for (const section of sample) {
            const oldPath = `/system-settings/${category}/${section}`
            await go(oldPath)
            try {
              await overview()
            } catch (cause) {
              throw new Error(
                `Failed legacy route ${oldPath}: ${cause.message}`,
                { cause }
              )
            }
            report.redirects.push({
              label,
              from: oldPath,
              to: new URL(page.url()).pathname,
            })
          }
        }
      })
      await check(`${label}-announcement-retry`, async () => {
        state.announcementStatus = 503
        await go('/system-settings/site')
        await page
          .getByText(
            t('The announcement service is temporarily unavailable.'),
            { exact: true }
          )
          .waitFor()
        await capture(page, `${label}-announcements-unavailable`)
        state.announcementStatus = 0
        await btn('Retry').click()
        await page
          .getByText(
            t('The announcement service is temporarily unavailable.'),
            { exact: true }
          )
          .waitFor({ state: 'hidden' })
        await overview()
      })

      await check(`${label}-hero-purchases-unavailable`, async () => {
        await go('/temporary-activations')
        await page
          .getByText(t('Purchases unavailable'), { exact: true })
          .waitFor()
        assert.equal(
          await page.locator('a[href="/temporary-activations"]').count(),
          0
        )
        assert.equal(
          state.requests.some(
            (request) =>
              request.path.startsWith('/api/hero-sms') &&
              request.path !== '/api/hero-sms/capabilities'
          ),
          false
        )
        await capture(page, `${label}-hero-purchases-unavailable`)
      })
      await context.close()
    }
  }
  assert.deepEqual(report.writes, [], 'Unexpected API mutations')
  assert.equal(report.page_errors.length, 0, 'Browser runtime errors')
  assert.equal(
    report.unexpected_requests.length,
    0,
    'Unimplemented API fixtures'
  )
} catch (error) {
  report.checks.push({
    name: 'browser-errors-and-fixtures',
    passed: false,
    error: String(error),
  })
} finally {
  await browser.close()
  await new Promise((resolve) => server.close(resolve))
  await writeFile(
    path.join(output, 'report.json'),
    `${JSON.stringify(report, null, 2)}\n`
  )
}
console.log(
  JSON.stringify(
    {
      checks: report.checks,
      screenshots: report.screenshots.length,
      page_errors: report.page_errors,
      unexpected_requests: report.unexpected_requests,
      output,
    },
    null,
    2
  )
)
if (report.checks.some((check) => !check.passed)) process.exitCode = 1
