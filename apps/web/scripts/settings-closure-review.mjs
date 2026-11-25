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
const options = [
  { key: 'ServerAddress', value: 'https://settings.example.test' },
  { key: 'RegisterEnabled', value: 'false' },
  { key: 'PasswordRegisterEnabled', value: 'true' },
  { key: 'OAuthRegisterEnabled', value: 'false' },
  { key: 'RegistrationDisabledMethods', value: 'fixture-provider' },
  { key: 'USDExchangeRate', value: '7.125' },
  { key: 'Notice', value: 'Fixture site notice' },
  { key: 'Announcements', value: '[]' },
  {
    key: 'console_setting.announcements',
    value: JSON.stringify([
      { id: 1, content: 'Fixture publication', publishDate: '2026-11-25' },
    ]),
  },
]
const plugin = (name, enabled) => ({
  name,
  version: '1.0',
  api_version: 1,
  available: true,
  installed: true,
  enabled,
  registration_sha256: 'a'.repeat(64),
})
const heroSettings = {
  revision: '23',
  enabled: true,
  email_enabled: false,
  sms_enabled: true,
  policy_managed_by_file: true,
  api_key_configured: true,
  credential_managed_by_file: true,
  pending_work: false,
  purchases_available: false,
  price_multiplier: '12.500000',
  currency: 'USD',
  currency_code: 840,
}
const honeypot = {
  version: 1,
  revision: 4,
  config: { version: 1, policies: [] },
  capabilities: {
    client_protocols: ['openai_chat', 'openai_responses'],
    upstream_protocols: [
      'openai_chat',
      'openai_responses',
      'anthropic_messages',
      'gemini',
    ],
    modes: ['fixed_text', 'custom_upstream'],
    numeric_loopback_only: true,
    credential_references: [],
  },
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
    } else if (p === '/api/extensions/plugins') {
      result = json([plugin('hero-sms', true), plugin('operations', false)])
    } else if (p === '/api/channel/ops') {
      result = json({ native_configuration: true, native_secrets: false })
    } else if (p === '/api/plugins/ssmcpd/admin/capabilities') {
      status = 404
      result = { success: false, message: 'SSMCP plugin is not installed' }
    } else if (p === '/api/option/') {
      result = json(options, {
        capabilities: { read_only: true, partial: true, writable_keys: [] },
      })
    } else if (p === '/api/option/hero-sms') {
      result = json(heroSettings)
    } else if (p === '/core/v1/admin/request-limits') {
      result = {
        version: 1,
        revision: 7,
        policy: {
          enabled: true,
          window_seconds: 43,
          user_limit: 17,
          group_limits: {},
        },
      }
    } else if (p === '/core/v1/admin/routing/auto-groups') {
      result = { version: 1, revision: 3, groups: ['fixture-group', 'default'] }
    } else if (p === '/core/v1/admin/abuse/policies') {
      result = honeypot
    } else if (p === '/core/v2/admin/abuse/users') {
      result = { version: 2, users: [], next_after: null }
    } else if (p === '/core/v1/admin/routing/configuration') {
      result = json({
        authority: {
          mode: 'file',
          source: 'model.config_file',
          version: { routing: 1, pricing: 1 },
          routing_sha256: 'a'.repeat(64),
          pricing_sha256: 'b'.repeat(64),
          git_revision: null,
          observed_at_unix: now,
        },
        version: { routing: 1, pricing: 1 },
        routing: {
          version: 1,
          upstreams: [
            {
              id: 7,
              protocol: 'gemini',
              endpoint: 'https://gemini.example.test/v1beta',
              credential: { id: 12, revision: 1 },
              enabled: true,
            },
          ],
          routes: [
            {
              id: 9,
              model: 'gemini-demo',
              group: 'fixture-group',
              enabled: true,
              speeds: ['standard'],
              targets: [
                {
                  id: 17,
                  upstream_id: 7,
                  upstream_model: 'gemini-flash-fixture',
                  enabled: true,
                  priority: 1,
                  weight: 1,
                  speeds: ['standard'],
                  client_protocols: ['gemini', 'openai_chat'],
                  payload: {},
                  timeout_ms: 30000,
                },
              ],
              failover: {
                max_attempts: 1,
                retry_on: [],
                allow_replay_after_send: false,
              },
            },
          ],
          aliases: [],
          groups: [{ name: 'fixture-group', enabled: true }],
        },
        prices: { version: 1, rules: [] },
        bindings: {
          names: {},
          contexts: [],
          vendors: {},
          models: {},
          discoveries: {},
          price_drafts: {},
        },
        secret_storage_enabled: false,
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
  for (const [width, theme] of [
    [1440, 'light'],
    [390, 'dark'],
  ]) {
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
    page.setDefaultTimeout(5000)
    page.setDefaultNavigationTimeout(10000)
    activePage = page
    page.on('pageerror', (error) =>
      report.page_errors.push({ label, error: String(error) })
    )
    const go = (route) => page.goto(origin + route)
    const btn = (key) => page.getByRole('button', { name: t(key), exact: true })
    const content = page.locator('[data-section-page] .console-section-content')
    const catalog = content.getByRole('navigation', {
      name: t('System Settings'),
    })
    const entry = (suffix) =>
      catalog.locator(`a[href="/system-settings/${suffix}"]`)
    const at = async (expected) => {
      const target = new URL(expected, origin)
      await page.waitForURL(
        (url) =>
          url.pathname.replace(/\/$/, '') ===
            target.pathname.replace(/\/$/, '') &&
          [...target.searchParams].every(
            ([key, value]) => url.searchParams.get(key) === value
          )
      )
    }
    const record = async (from, expected) => {
      await at(expected)
      const current = new URL(page.url())
      report.redirects.push({
        label,
        from,
        to: current.pathname + current.search,
      })
    }
    const overview = async () => {
      await at('/system-settings/site')
      await content.getByText('visual-fixture', { exact: true }).waitFor()
      await catalog.waitFor()
    }
    await check(`${label}-settings-overview`, async () => {
      await go('/system-settings')
      await overview()
      for (const suffix of [
        'site/notice',
        'billing/group-pricing',
        'operations/hero-sms',
        'models/gemini',
        'security/honeypot',
        'operations/scripts',
      ]) {
        assert.equal(await entry(suffix).count(), 1, suffix)
      }
      assert.equal(await entry('content/faq').count(), 0)
      await capture(page, `${label}-settings-overview`)
      await entry('site/notice').click()
      await record(
        '/system-settings/site/notice',
        '/system-settings/site?domain=console'
      )
      await content.getByText('Fixture site notice', { exact: true }).waitFor()
      await content.getByText('Fixture publication', { exact: true }).waitFor()
      assert.equal(
        await content
          .locator(
            'input, textarea, select, [role=switch], [contenteditable=true]'
          )
          .count(),
        0
      )
      await capture(page, `${label}-console-settings`)
    })
    await check(`${label}-gemini-routing`, async () => {
      await go('/system-settings/models?section=gemini')
      await at('/system-settings/models/gemini')
      await content
        .getByText('https://gemini.example.test/v1beta', { exact: true })
        .waitFor()
      await content.getByText('gemini-demo → gemini-flash-fixture').waitFor()
      await content.getByText(/gemini, openai_chat/).waitFor()
      await content.getByText('model.config_file', { exact: true }).waitFor()
      assert.ok(await content.locator('a[href="/channels?edit=7"]').isVisible())
      assert.ok(
        await page
          .getByRole('link', { name: t('Channel Management'), exact: true })
          .first()
          .isVisible()
      )
      await capture(page, `${label}-gemini-routing`)
    })
    await check(`${label}-native-hero-settings`, async () => {
      const from = '/system-settings/operations?section=hero-sms'
      await go(from)
      await record(from, '/system-settings/operations/hero-sms')
      await content.getByText('hero_sms.policy_file', { exact: true }).waitFor()
      await content
        .getByText(heroSettings.price_multiplier, { exact: true })
        .waitFor()
      assert.equal(
        await content.locator('input, form, [role=switch]').count(),
        0
      )
      assert.ok(await btn('Test Connection').isEnabled())
      await capture(page, `${label}-hero-policy`)
    })
    if (width === 1440) {
      await check('core-domain-bookmarks', async () => {
        for (const [from, domain, value] of [
          [
            '/system-settings/auth?section=basic-auth',
            'identity',
            'fixture-provider',
          ],
          ['/system-settings/billing/currency', 'billing', '7.125'],
          ['/system-settings/security/rate-limit', 'routing', 'fixture-group'],
          [
            '/system-settings/site?section=notice',
            'console',
            'Fixture site notice',
          ],
        ]) {
          await go(from)
          await record(from, `/system-settings/site?domain=${domain}`)
          await content.getByText(value, { exact: true }).waitFor()
        }
      })
      await check('honeypot-bookmark', async () => {
        const from = '/system-settings/security?section=honeypot'
        await go(from)
        await record(from, '/system-settings/security/honeypot')
        await content
          .getByRole('tab', { name: t('Policies'), exact: true })
          .waitFor()
        await content
          .getByText(t('No policies configured.'), { exact: true })
          .waitFor()
        assert.equal(
          await content.getByRole('button', { name: t('Add policy') }).count(),
          0
        )
        await capture(page, 'desktop-honeypot')
      })
      await check('unavailable-bookmark', async () => {
        const dest = '/system-settings/content/faq'
        await go(dest)
        await at(dest)
        await page
          .getByText(
            t('The console FAQ list has no native settings contract.'),
            { exact: true }
          )
          .waitFor()
      })
      await check('plugin-owner-redirect', async () => {
        const from = '/system-settings/operations/scripts'
        await go(from)
        await record(from, '/extensions/plugins?configure=operations')
        await page
          .getByRole('status')
          .filter({ hasText: t('Disabled') })
          .first()
          .waitFor()
      })
      await check('group-pricing-owner-redirect', async () => {
        const from = '/system-settings/billing/group-pricing'
        await go(from)
        await record(from, '/channels?view=group-pricing')
        await page
          .getByRole('heading', { name: t('Channels'), exact: true })
          .first()
          .waitFor()
      })
    }
    await check(`${label}-announcement-retry`, async () => {
      state.announcementStatus = 503
      await go('/system-settings/site')
      await page
        .getByText(t('The announcement service is temporarily unavailable.'), {
          exact: true,
        })
        .waitFor()
      await capture(page, `${label}-announcements-unavailable`)
      state.announcementStatus = 0
      await btn('Retry').click()
      await page
        .getByText(t('The announcement service is temporarily unavailable.'), {
          exact: true,
        })
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
