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
const dist = path.join(web, 'dist')
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
  index_sha256: createHash('sha256')
    .update(await readFile(path.join(dist, 'index.html')))
    .digest('hex'),
  scope: 'Built Web and synthetic API fixtures, no production services.',
  checks: [],
  screenshots: [],
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
function configuration() {
  return {
    version: { routing: 3, pricing: 2 },
    secret_storage_enabled: false,
    routing: {
      version: 3,
      upstreams: [],
      routes: [],
      aliases: [],
      groups: [
        { name: 'default', enabled: true },
        { name: 'team', enabled: true },
      ],
    },
    prices: {
      version: 2,
      rules: Array.from({ length: 24 }, (_, i) => ({
        id: i + 1,
        model: `gpt-test-${String(i + 1).padStart(2, '0')}`,
        group: i % 2 ? 'team' : 'default',
        speed: 'standard',
        rates: {
          input: 2000000,
          output: 6000000,
          cache_read: 500000,
          cache_write: 1000000,
          request: 0,
        },
        group_multiplier_ppm: 1000000,
        speed_multiplier_ppm: 1000000,
      })),
    },
    bindings: {
      names: {},
      contexts: [],
      vendors: {},
      models: {},
      discoveries: {},
      price_drafts: {},
    },
  }
}
function fixture() {
  return {
    configuration: configuration(),
    rejectHero: false,
    rejectCheckin: false,
    hero: {
      revision: '2',
      enabled: false,
      email_enabled: false,
      sms_enabled: false,
      api_key_configured: true,
      credential_managed_by_file: true,
      pending_work: false,
      purchases_available: false,
      currency: 'USD',
      currency_code: 840,
      price_multiplier: '1',
    },
    checkin: {
      revision: 1,
      enabled: true,
      min_quota: '100000',
      max_quota: '200000',
      writable: true,
      funding_available: true,
      module_enabled: true,
      funding_limit: '500000',
    },

    rejectPricing: false,
    announcementStatus: 0,
    requests: [],
    payment: {
      contract: 'extensions.pancake-configuration.v1',
      merchant_id: 'MER_fixture',
      store_id: 'STO_fixture',
      product_id: 'PROD_fixture',
      return_url: 'https://example.test/wallet',
      environment: 'test',
      private_key_configured: true,
      channels: [
        { id: 7, currency: 'USD', enabled: true, subscription_enabled: false },
      ],
    },
    scripts: {
      revision: 1,
      repository_url: 'https://github.com/example/scripts.git',
      branch: 'main',
      github_key_set: true,
    },
  }
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
    const body = req.postDataJSON()
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
        registration_enabled: false,
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
    } else if (p === '/core/v1/admin/routing/pricing-source-snapshot') {
      result = json(null)
    } else if (p === '/core/v1/admin/routing/configuration') {
      result = json(state.configuration)
    } else if (p === '/core/v1/admin/routing/prices/preview') {
      assert.equal(method, 'POST')
      assert.deepEqual(body.expected_version, state.configuration.version)
      result = json({
        confirmation_digest: 'a'.repeat(64),
        before: state.configuration.prices,
        after: body.prices,
      })
    } else if (p === '/core/v1/admin/routing/prices') {
      assert.equal(method, 'PUT')
      assert.equal(body.confirmation_digest, 'a'.repeat(64))
      assert.deepEqual(body.expected_version, state.configuration.version)
      report.writes.push({ path: p, body })
      if (state.rejectPricing) {
        status = 409
        result = { success: false, message: 'Synthetic version conflict' }
      } else {
        state.configuration = {
          ...state.configuration,
          version: {
            routing: state.configuration.version.routing + 1,
            pricing: body.prices.version,
          },
          routing: {
            ...state.configuration.routing,
            version: state.configuration.routing.version + 1,
          },
          prices: body.prices,
        }
        result = json({ version: state.configuration.version })
      }
    } else if (p === '/api/option/waffo-pancake/config') {
      result = json(state.payment)
    } else if (p === '/api/option/waffo-pancake/save') {
      assert.equal(method, 'POST')
      assert.equal(body.private_key ?? '', '')
      state.payment.return_url = body.return_url
      report.writes.push({ path: p, body })
      result = json({
        store_id: state.payment.store_id,
        product_id: state.payment.product_id,
      })
    } else if (p === '/api/option/waffo-pancake/catalog') {
      result = json({
        stores: [
          {
            id: 'STO_fixture',
            name: 'Synthetic store',
            status: 'active',
            prodEnabled: false,
            onetimeProducts: [
              { id: 'PROD_fixture', name: 'Wallet credits', status: 'active' },
            ],
            subscriptionProducts: [],
          },
        ],
      })
    } else if (p === '/api/scripts') {
      result = json([
        { name: 'setup.sh', size: 128, updated: '2026-11-17T00:00:00Z' },
      ])
    } else if (p === '/api/option/hero-sms') {
      if (method === 'PUT') {
        assert.equal(body.expected_revision, state.hero.revision)
        if (state.rejectHero) {
          status = 409
          result = {
            success: false,
            code: 'STATE_CONFLICT',
            message: 'Synthetic conflict',
          }
        } else {
          state.hero = {
            ...state.hero,
            enabled: body.enabled,
            sms_enabled: body.sms_enabled,
            email_enabled: body.email_enabled,
            price_multiplier: body.price_multiplier,
            revision: String(BigInt(state.hero.revision) + 1n),
          }
          report.writes.push({ path: p, body })
          result = json(state.hero)
        }
      } else result = json(state.hero)
    } else if (p === '/api/option/hero-sms/test') {
      assert.equal(method, 'POST')
      result = json({ ok: true })
    } else if (p === '/api/option/checkin') {
      if (method === 'PUT') {
        assert.equal(body.revision, state.checkin.revision)
        assert.ok(body.operation_key)
        if (state.rejectCheckin) {
          status = 409
          result = {
            success: false,
            code: 'STATE_CONFLICT',
            message: 'Synthetic conflict',
          }
        } else {
          state.checkin = {
            ...state.checkin,
            enabled: body.enabled,
            min_quota: body.min_quota,
            max_quota: body.max_quota,
            revision: state.checkin.revision + 1,
          }
          report.writes.push({ path: p, body })
          result = json(state.checkin)
        }
      } else result = json(state.checkin)
    } else if (p === '/api/scripts/repository') {
      if (method === 'PUT') {
        assert.equal(body.revision, state.scripts.revision)
        state.scripts = {
          ...state.scripts,
          branch: body.branch,
          repository_url: body.repository_url,
          revision: state.scripts.revision + 1,
        }
        report.writes.push({ path: p, body })
      }
      result = json(state.scripts)
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
      })
      const state = fixture()
      await install(context, state, theme)
      const page = await context.newPage()
      activePage = page
      page.on('pageerror', (error) =>
        report.page_errors.push({ label, error: String(error) })
      )
      const go = (route) => page.goto(origin + route)
      const btn = (key) =>
        page.getByRole('button', { name: t(key), exact: true })
      await check(`${label}-pricing`, async () => {
        await go('/system-settings/billing/model-pricing')
        await page
          .getByRole('cell', { name: 'gpt-test-01', exact: false })
          .waitFor()
        assert.equal(await page.locator('tbody tr').count(), 20)
        await capture(page, `${label}-pricing`)
        await btn('Next').click()
        await page
          .getByRole('cell', { name: 'gpt-test-24', exact: false })
          .waitFor()
        assert.equal(await page.locator('tbody tr').count(), 4)
        await page
          .getByRole('textbox', { name: t('Search models'), exact: true })
          .fill('gpt-test-01')
        await page
          .getByRole('cell', { name: 'gpt-test-01', exact: false })
          .waitFor()
        assert.equal(await page.locator('tbody tr').count(), 1)
        await btn('Edit').first().click()
        await page.getByRole('dialog').waitFor()
        await capture(page, `${label}-price-editor`)
        await page
          .getByRole('dialog')
          .getByRole('textbox', {
            name: t('Input price (USD / 1M tokens)'),
            exact: true,
          })
          .fill('2.000001')
        await btn('Apply to draft').click()
        await btn('Preview changes').click()
        await page.getByRole('dialog').waitFor()
        await capture(page, `${label}-price-preview`)
        await btn('Confirm and publish').click()
        await page.getByRole('dialog').waitFor({ state: 'hidden' })
        assert.equal(state.configuration.prices.rules[0].rates.input, 2000001)
        await btn('Edit').first().click()
        await page
          .getByRole('dialog')
          .getByRole('textbox', {
            name: t('Input price (USD / 1M tokens)'),
            exact: true,
          })
          .fill('4.5')
        await btn('Apply to draft').click()
        state.rejectPricing = true
        await btn('Preview changes').click()
        await btn('Confirm and publish').click()
        await page.getByRole('dialog').getByRole('alert').waitFor()
        await capture(page, `${label}-price-failed-publish`)
        await page
          .getByRole('dialog')
          .getByRole('button', { name: t('Cancel'), exact: true })
          .click()
        await btn('Edit').first().click()
        assert.equal(
          await page
            .getByRole('dialog')
            .getByRole('textbox', {
              name: t('Input price (USD / 1M tokens)'),
              exact: true,
            })
            .inputValue(),
          '4.5'
        )
        await page
          .getByRole('dialog')
          .getByRole('button', { name: t('Cancel'), exact: true })
          .click()
      })
      await check(`${label}-payment`, async () => {
        await go('/system-settings/billing/payment')
        const url = page.locator('#native-payment-return-url')
        await url.waitFor()
        assert.equal(await url.isEnabled(), true)
        await capture(page, `${label}-payment`)
        await url.fill('https://example.test/updated-wallet')
        await btn('Save payment settings').click()
        await page
          .getByText(
            t('Payment settings saved and read back from the server.'),
            { exact: true }
          )
          .waitFor()
        assert.equal(
          state.payment.return_url,
          'https://example.test/updated-wallet'
        )
        await btn('Load provider catalog').click()
        await page.getByText('Synthetic store', { exact: true }).waitFor()
        await capture(page, `${label}-payment-saved`)
      })
      await check(`${label}-independent-scripts`, async () => {
        await go('/system-settings/operations/scripts')
        const branch = page.locator('#scripts-repository-branch')
        await branch.waitFor()
        assert.equal(await branch.isEnabled(), true)
        await branch.fill('preview')
        await btn('Save repository').click()
        await page
          .getByText(t('Script repository settings saved'), { exact: true })
          .waitFor()
        assert.equal(state.scripts.branch, 'preview')
        await capture(page, `${label}-scripts`)
      })
      await check(`${label}-announcement-retry`, async () => {
        state.announcementStatus = 503
        await go('/system-settings/billing/model-pricing')
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
          .getByRole('textbox', { name: t('Search models'), exact: true })
          .waitFor()
      })

      await check(`${label}-hero-settings`, async () => {
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
        await go('/system-settings/operations/hero-sms')
        const multiplier = page.locator('input[type="number"]')
        await multiplier.waitFor()
        assert.equal(await multiplier.isEnabled(), true)
        await capture(page, `${label}-hero-settings`)
        await multiplier.fill('2.000001')
        const saved = page.waitForResponse(
          (response) =>
            response.url().endsWith('/api/option/hero-sms') &&
            response.request().method() === 'PUT'
        )
        await btn('Save Changes').click()
        await saved
        await btn('Save Changes').waitFor({ state: 'visible' })
        assert.equal(state.hero.price_multiplier, '2.000001')
        await go('/system-settings/operations/hero-sms')
        await multiplier.waitFor()
        assert.equal(await multiplier.inputValue(), '2.000001')
        await btn('Test connection').click()
        await page
          .getByText(t('Connection test succeeded'), { exact: true })
          .waitFor()
        state.rejectHero = true
        await multiplier.fill('3.000001')
        await btn('Save Changes').click()
        await page
          .getByText(t('Unable to save HeroSMS settings'), { exact: true })
          .first()
          .waitFor()
        assert.equal(await multiplier.inputValue(), '3.000001')
        await capture(page, `${label}-hero-conflict`)
        await btn('Reload').click()
        await page.waitForFunction(
          () =>
            document.querySelector('input[type="number"]')?.value === '2.000001'
        )
      })
      await check(`${label}-checkin-settings`, async () => {
        await go('/system-settings/billing/checkin')
        const minimum = page.locator('#checkin-minimum')
        const maximum = page.locator('#checkin-maximum')
        await minimum.waitFor()
        assert.equal(await minimum.isEnabled(), true)
        await minimum.fill('100001')
        await btn('Save check-in settings').click()
        await page.getByText(t('Setting saved'), { exact: true }).waitFor()
        assert.equal(state.checkin.min_quota, '100001')
        await go('/system-settings/billing/checkin')
        await minimum.waitFor()
        assert.equal(await minimum.inputValue(), '100001')
        await maximum.fill('500001')
        assert.equal(await btn('Save check-in settings').isDisabled(), true)
        await page.getByRole('alert').first().waitFor()
        await capture(page, `${label}-checkin-ceiling`)
        await maximum.fill('200001')
        state.rejectCheckin = true
        await btn('Save check-in settings').click()
        await page
          .getByText(
            t('Settings changed on the server. Reload before saving.'),
            { exact: true }
          )
          .waitFor()
        assert.equal(await maximum.inputValue(), '200001')
        await capture(page, `${label}-checkin-conflict`)
        await btn('Discard changes and reload').click()
        await page.waitForFunction(
          () => document.querySelector('#checkin-maximum')?.value === '200000'
        )
      })
      await context.close()
    }
  }
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
