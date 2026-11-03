// SPDX-License-Identifier: AGPL-3.0-or-later
// Real Chromium + static HEAD build + actual Core. No success response mocks.
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const cfg = JSON.parse(await fs.readFile(process.argv[2], 'utf8'))
const { chromium } = await import(pathToFileURL(cfg.playwright).href)
const cases = Object.fromEntries([
  'admin-login', 'notice-save-refresh', 'announcement-add-alias-refresh',
  'other-settings-readonly', 'narrow-screen', 'storage-error-no-retry',
  'commit-unknown-no-retry', 'ordinary-no-write',
  'navigation-save-refresh', 'navigation-restart-public', 'navigation-ordinary-denial',
  'finance-create-readback', 'finance-restart-readback', 'finance-ordinary-denial',
].map(k => [k, 'NOT_RUN']))
const events = [], errors = [], external = [], writeAttempts = [], financeRequests = []
let browser, page, current = 'admin-login'
let token
const save = (name, value) => fs.writeFile(path.join(cfg.root, name), JSON.stringify(value, null, 2) + '\n')
async function capture(label) {
  await fs.writeFile(path.join(cfg.root, label + '.aria.txt'), await page.locator('body').ariaSnapshot())
  await fs.writeFile(path.join(cfg.root, label + '.dom.html'), await page.content())
  await page.screenshot({ path: path.join(cfg.root, label + '.png'), fullPage: true, timeout: 15000 })
  await save('browser-events.json', events)
}
async function control(action) {
  const r = await fetch(cfg.origin + '/__fixture', {
    method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Fixture-Key': cfg.fixtureKey },
    body: JSON.stringify({ action }),
  })
  assert.equal(r.status, 200)
  return r.json()
}
async function ready(locator) { await locator.waitFor({ state: 'visible', timeout: 20000 }) }
async function poll(check, label) {
  for (let i = 0; i < 40; i++) {
    if (await check()) return
    await new Promise(r => setTimeout(r, 100))
  }
  assert.fail(label)
}
function writes() { return writeAttempts }
async function context() {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US', colorScheme: 'light', serviceWorkers: 'block' })
  await ctx.route('**/*', route => {
    const url = route.request().url()
    if (url.startsWith(cfg.origin + '/') || /^(data|blob|about):/.test(url)) return route.continue()
    external.push({ method: route.request().method(), origin: new URL(url).origin })
    return route.abort('blockedbyclient')
  })
  const p = await ctx.newPage()
  p.setDefaultTimeout(15000)
  // Full page navigation rotates the real Web session. Supplemental reads must
  // use the credential actually sent by the current page, never the old login token.
  p.on('request', req => {
    if (!req.url().startsWith(cfg.origin + '/')) return
    const requestPath = new URL(req.url()).pathname
    if (/^\/(?:core\/v1\/(?:subscriptions|promotions)|api\/(?:subscription|redemption))/.test(requestPath))
      financeRequests.push({ case: current, method: req.method(), path: requestPath })
    if (!requestPath.startsWith('/api/') && !requestPath.startsWith('/core/')) return
    if (new URL(req.url()).pathname.startsWith('/api/option') && ['PUT', 'POST'].includes(req.method())) {
      writeAttempts.push({ case: current, method: req.method(), path: new URL(req.url()).pathname, request: req.postDataJSON(), at: Date.now() })
    }
    const authorization = req.headers().authorization
    if (authorization?.startsWith('Bearer ')) token = authorization.slice(7)
  })
  p.on('pageerror', e => errors.push({ case: current, message: e.message, name: e.name, stack: e.stack }))
  p.on('response', async r => {
    const req = r.request(), url = new URL(r.url())
    if (!url.pathname.startsWith('/api/') && !url.pathname.startsWith('/core/')) return
    const item = { case: current, path: url.pathname, method: req.method(), status: r.status() }
    events.push(item)
    if (url.pathname.startsWith('/api/option')) {
      item.request = req.postDataJSON()
      try { item.response = await r.json() } catch { item.bodyUnavailable = true }
    }
  })
  return p
}
async function login(username, password, target) {
  await page.goto(cfg.origin + '/sign-in?redirect=' + encodeURIComponent(target))
  await ready(page.getByLabel('Username or Email', { exact: true }))
  await capture(username + '-login-form')
  await page.getByLabel('Username or Email', { exact: true }).fill(username)
  await page.getByLabel('Password', { exact: true }).fill(password)
  const result = page.waitForResponse(r => new URL(r.url()).pathname === '/api/user/login' && r.request().method() === 'POST')
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  const response = await result
  assert.equal(response.status(), 200)
  const body = await response.json()
  assert.equal(body.success, true)
  assert.ok(body.data.access_token)
  token = body.data.access_token
  await page.waitForURL(url => url.pathname !== '/sign-in')
  const identityResponse = await page.context().request.get(cfg.origin + '/core/v1/identity', { headers: { Authorization: 'Bearer ' + token } })
  assert.equal(identityResponse.status(), 200)
  const principal = await identityResponse.json()
  await save(username + '-identity.json', principal)
  return principal
}
async function optionRead() {
  const r = await page.context().request.get(cfg.origin + '/api/option', { headers: { Authorization: 'Bearer ' + token } })
  assert.equal(r.status(), 200)
  const b = await r.json()
  assert.equal(b.success, true)
  return Object.fromEntries(b.data.map(x => [x.key, x.value]))
}
async function noticePage() {
  await page.goto(cfg.origin + '/system-settings/site/notice')
  await ready(page.getByLabel('Announcement content', { exact: true }))
}
async function saveNotice(value, expectedStatus, code) {
  await capture(current + '-before')
  await page.getByLabel('Announcement content', { exact: true }).fill(value)
  const pending = page.waitForResponse(r => new URL(r.url()).pathname.replace(/\/$/, '') === '/api/option' && r.request().method() === 'PUT')
  await page.getByRole('button', { name: 'Save notice', exact: true }).click()
  const r = await pending, body = await r.json()
  assert.equal(r.status(), expectedStatus)
  assert.equal(body.success, expectedStatus === 200)
  if (code) {
    assert.equal(body.code, code)
    await ready(page.locator('[data-sonner-toast][data-type="error"]').filter({ hasText: body.message }))
  }
  else assert.equal(body.saved, true)
  await capture(current + '-response')
}
async function run(name, fn) {
  current = name
  console.log('START', name)
  await fn()
  assert.equal(errors.length, 0, 'browser pageerror: ' + JSON.stringify(errors))
  cases[name] = 'PASS'
  await save('browser-progress.json', cases)
  console.log('PASS', name)
}

async function ordinaryNoWrite() {
  await run('ordinary-no-write', async () => {
    await page.context().close()
    page = await context()
    const identity = await login('consoleordinary', cfg.ordinaryPassword, '/system-settings/site/notice')
    assert.ok(identity.platform_level < 6)
    for (const target of ['/system-settings/site/notice', '/system-settings/content/announcements']) {
      await page.goto(cfg.origin + target)
      await page.waitForLoadState('networkidle')
      assert.equal(await page.getByRole('button', { name: /Save notice|Add Announcement|Save Settings/ }).count(), 0)
      assert.equal(await page.getByLabel('Announcement content', { exact: true }).count(), 0)
      await capture('ordinary-' + target.split('/').at(-1))
    }
    assert.equal(writes().filter(e => e.case === current).length, 0)
  })
}

try {
  browser = await chromium.launch({ executablePath: cfg.chromium, headless: true, timeout: 30000,
    args: ['--disable-gpu', '--disable-dev-shm-usage', '--renderer-process-limit=2',
      '--disable-background-networking', '--disable-component-update', '--no-first-run'] })
  await save('browser-version.json', { version: browser.version(), headless: true, desktop: '1440x1000', narrow: ['390x844', '320x844'] })
  page = await context()
  if (cfg.ordinaryOnly) {
    if (!cfg.noticeRetest) await ordinaryNoWrite()
  } else {
  await run('admin-login', async () => {
    const identity = await login('consoleowner', cfg.adminPassword, '/system-settings/site/notice')
    assert.equal(identity.platform_level, 6)
    assert.equal(identity.credential_kind, 'session')
    await ready(page.getByLabel('Announcement content', { exact: true }))
    await capture('admin-notice-initial')
  })
  if (cfg.diagnoseSelector) {
    current = 'selector-diagnosis'
    await page.goto(cfg.origin + '/system-settings/site/system-info')
    await ready(page.getByText('Only part of the settings is available. Missing settings are not displayed.', { exact: true }))
    const hits = await page.locator('main input:visible:enabled, main textarea:visible:enabled').evaluateAll(nodes => nodes.map(el => {
      const describe = node => {
        const css = getComputedStyle(node), r = node.getBoundingClientRect()
        return { tag: node.tagName, id: node.id, testId: node.getAttribute('data-testid'),
          ariaHidden: node.getAttribute('aria-hidden'), inert: node.hasAttribute('inert'),
          class: node.getAttribute('class'), rect: { x: r.x, y: r.y, width: r.width, height: r.height },
          style: { display: css.display, visibility: css.visibility, opacity: css.opacity, pointerEvents: css.pointerEvents, overflowX: css.overflowX } }
      }
      const ancestors = []
      for (let p = el.parentElement; p; p = p.parentElement) ancestors.push(describe(p))
      return { ...describe(el), outerHTML: el.outerHTML, name: el.name, disabled: el.disabled, readOnly: el.readOnly,
        inSettingsSheet: Boolean(el.closest('.settings-sheet')),
        form: el.form ? { ...describe(el.form), outerHTML: el.form.outerHTML } : null, ancestors }
    }))
    await save('selector-hits.json', { selector: 'main input:visible:enabled, main textarea:visible:enabled', count: hits.length, hits })
    await capture('selector-diagnosis')
  } else {
  const notice = cfg.noticeRetest ? 'Browser acceptance Notice — fixed handler successful save.' : 'Browser acceptance Notice — local synthetic content.'
  const announcement = 'Browser announcement — persisted through both public aliases.'
  if (!cfg.resume || cfg.noticeRetest) {
  await run('notice-save-refresh', async () => {
    const before = writes().length
    await saveNotice(notice, 200)
    assert.equal((await optionRead()).Notice, notice)
    await page.reload()
    await ready(page.getByLabel('Announcement content', { exact: true }))
    assert.equal(await page.getByLabel('Announcement content', { exact: true }).inputValue(), notice)
    assert.equal(writes().length - before, 1)
    await capture('notice-refreshed-desktop')
  })
  if (!cfg.noticeRetest) {
  await run('announcement-add-alias-refresh', async () => {
    await page.goto(cfg.origin + '/system-settings/content/announcements')
    await ready(page.getByRole('button', { name: 'Add Announcement', exact: true }))
    await capture('announcements-initial')
    await page.getByRole('button', { name: 'Add Announcement', exact: true }).click()
    const dialog = page.getByRole('dialog')
    await ready(dialog)
    await capture('announcement-dialog')
    await dialog.getByLabel('Content', { exact: true }).fill(announcement)
    await dialog.getByRole('button', { name: 'Add', exact: true }).click()
    await ready(page.getByRole('cell', { name: announcement, exact: true }))
    const pending = page.waitForResponse(r => new URL(r.url()).pathname.replace(/\/$/, '') === '/api/option' && r.request().method() === 'PUT')
    await page.getByRole('button', { name: 'Save Settings', exact: true }).filter({ visible: true }).evaluateAll(nodes => {
      if (nodes.filter(x => !x.disabled).length !== 1) throw new Error('exactly one enabled announcement save required')
    })
    await page.locator('button').filter({ hasText: /^Save Settings$/ }).and(page.locator(':enabled')).click()
    const response = await pending
    assert.equal(response.status(), 200)
    assert.equal((await response.json()).saved, true)
    const values = await optionRead()
    assert.equal(values.Announcements, values['console_setting.announcements'])
    assert.equal(JSON.parse(values.Announcements)[0].content, announcement)
    await page.reload()
    await ready(page.getByRole('cell', { name: announcement, exact: true }))
    await capture('announcements-refreshed-desktop')
  })
  } else {
    assert.equal(cfg.priorCases['announcement-add-alias-refresh'], 'PASS')
    cases['announcement-add-alias-refresh'] = 'REUSED_PASS'
  }
  } else {
    for (const name of ['notice-save-refresh', 'announcement-add-alias-refresh']) {
      assert.equal(cfg.priorCases[name], 'PASS', 'cannot inherit an unpassed case')
      cases[name] = 'REUSED_PASS'
    }
    await save('inherited-cases.json', { from: cfg.reuseRoot, cases: Object.fromEntries(Object.entries(cases).filter(([,v]) => v === 'REUSED_PASS')) })
  }
  if (!cfg.noticeRetest) {
  await run('other-settings-readonly', async () => {
    const before = writes().length
    await page.goto(cfg.origin + '/system-settings/site/system-info')
    await ready(page.getByText('Only part of the settings is available. Missing settings are not displayed.', { exact: true }))
    await capture('other-settings-readonly')
    const sheet = page.locator('.settings-sheet')
    assert.equal(await sheet.count(), 1, 'exactly one business settings region required')
    assert.equal(await sheet.locator('input:visible:enabled, textarea:visible:enabled, select:visible:enabled, [contenteditable="true"]:visible, button[role="switch"]:visible:enabled, button[role="checkbox"]:visible:enabled').count(), 0)
    await save('readonly-business-region.json', { selector: '.settings-sheet', outerHTML: await sheet.evaluate(el => el.outerHTML), priorDiagnosis: cfg.selectorDiagnosis })
    assert.equal(writes().length - before, 0)
    const saves = page.getByRole('button', { name: 'Save Settings', exact: true })
    assert.ok(await saves.count())
    for (const button of await saves.all()) assert.equal(await button.isDisabled(), true)
  })
  } else {
    assert.equal(cfg.priorReadonlyCases['other-settings-readonly'], 'PASS')
    cases['other-settings-readonly'] = 'REUSED_PASS'
    assert.equal(cfg.priorOrdinaryCases['ordinary-no-write'], 'PASS')
    cases['ordinary-no-write'] = 'REUSED_PASS'
  }
  await run('narrow-screen', async () => {
    for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 })
    await noticePage()
    await saveNotice(notice + ' Narrow ' + width + '.', 200)
    await page.reload()
    await ready(page.getByLabel('Announcement content', { exact: true }))
    assert.equal(await page.getByLabel('Announcement content', { exact: true }).inputValue(), notice + ' Narrow ' + width + '.')
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'page overflows narrow viewport')
    await capture('notice-narrow-' + width)
    await page.goto(cfg.origin + '/system-settings/content/announcements')
    await ready(page.getByRole('cell', { name: announcement, exact: true }))
    await page.getByRole('button', { name: 'Add Announcement', exact: true }).click()
    await ready(page.getByRole('dialog'))
    await capture('announcement-dialog-narrow-' + width)
    await page.getByRole('dialog').getByLabel('Content', { exact: true }).fill('Narrow local draft')
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel', exact: true }).click()
    }
    await page.setViewportSize({ width: 1440, height: 1000 })
  })
  await run('navigation-save-refresh', async () => {
    for (const width of [1440, 390, 320]) {
      await page.setViewportSize({ width, height: width === 1440 ? 1000 : 844 })
      await page.goto(cfg.origin + '/system-settings/site/header-navigation')
      const directory = page.getByRole('switch', { name: 'AI directory', exact: true })
      await ready(directory)
      const enabled = !(await directory.isChecked())
      await directory.click()
      const pending = page.waitForResponse(r => new URL(r.url()).pathname.replace(/\/$/, '') === '/api/option' && r.request().method() === 'PUT')
      await page.getByRole('button', { name: 'Save navigation', exact: true }).click()
      const response = await pending, body = await response.json()
      assert.equal(response.status(), 200)
      assert.equal(body.success, true)
      assert.equal(body.saved, true)
      const request = response.request().postDataJSON()
      assert.equal(request.key, 'HeaderNavModules')
      assert.equal(JSON.parse(request.value).directory, enabled)
      assert.equal(JSON.parse((await optionRead()).HeaderNavModules).directory, enabled)
      await page.reload()
      await ready(directory)
      assert.equal(await directory.isChecked(), enabled)
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'navigation settings overflow at ' + width)
      await capture('navigation-editor-' + width)
      const adminPage = page
      page = await context()
      try {
        await page.setViewportSize({ width, height: width === 1440 ? 1000 : 844 })
        const status = page.waitForResponse(r => new URL(r.url()).pathname === '/api/status' && r.request().method() === 'GET')
        await page.goto(cfg.origin + '/')
        const publicResponse = await status, publicBody = await publicResponse.json()
        assert.equal(publicResponse.status(), 200)
        assert.equal(publicBody.success, true)
        assert.equal(JSON.parse(publicBody.data.HeaderNavModules).directory, enabled)
        if (width !== 1440) await page.getByRole('button', { name: 'Toggle navigation menu' }).click()
        const nav = width === 1440 ? page.locator('header nav') : page.getByRole('dialog', { name: 'Header navigation' })
        await ready(nav.locator('a[href="/developers"]').filter({ visible: true }).first())
        await poll(async () => await nav.locator('a[href="/ai-directory"]:visible').count() === (enabled ? 1 : 0) && await page.locator('.home-directory-link').count() === (enabled ? 1 : 0), 'public navigation did not reflect save at ' + width)
        await capture('navigation-public-save-' + width)
      } finally {
        await page.context().close()
        page = adminPage
      }
    }
    await page.setViewportSize({ width: 1440, height: 1000 })
  })
  await run('navigation-restart-public', async () => {
    const before = (await optionRead()).HeaderNavModules
    const savedDirectory = JSON.parse(before).directory
    const restarted = await control('restart-core')
    assert.notEqual(restarted.old_pid, restarted.new_pid)
    assert.equal((await optionRead()).HeaderNavModules, before)
    await page.reload()
    const directory = page.getByRole('switch', { name: 'AI directory', exact: true })
    await ready(directory)
    assert.equal(await directory.isChecked(), savedDirectory)
    const adminPage = page
    page = await context()
    try {
      for (const width of [1440, 390, 320]) {
        await page.setViewportSize({ width, height: width === 1440 ? 1000 : 844 })
        const status = page.waitForResponse(r => new URL(r.url()).pathname === '/api/status' && r.request().method() === 'GET')
        await page.goto(cfg.origin + '/')
        const response = await status, body = await response.json()
        assert.equal(response.status(), 200)
        assert.equal(body.success, true)
        assert.equal(JSON.parse(body.data.HeaderNavModules).directory, savedDirectory)
        if (width !== 1440) {
          await page.getByRole('button', { name: 'Toggle navigation menu' }).click()
        }
        const nav = width === 1440 ? page.locator('header nav') : page.getByRole('dialog', { name: 'Header navigation' })
        await ready(nav.locator('a[href="/developers"]').filter({ visible: true }).first())
        await poll(async () => await nav.locator('a[href="/ai-directory"]:visible').count() === (savedDirectory ? 1 : 0) && await page.locator('.home-directory-link').count() === (savedDirectory ? 1 : 0), 'public navigation did not reflect saved directory at ' + width)
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'public page overflow at ' + width)
        await capture('navigation-public-' + width)
      }
    } finally {
      await page.context().close()
      page = adminPage
    }
  })
  await run('storage-error-no-retry', async () => {
    await noticePage()
    await page.locator('[data-sonner-toast][data-type="success"]').waitFor({ state: 'hidden', timeout: 10000 })
    const dbBefore = await control('snapshot'), count = writes().length
    await control('revoke-audit')
    try {
      await saveNotice('This save must roll back.', 503, 'SYSTEM_OPTIONS_STORAGE_UNAVAILABLE')
      assert.equal(await page.locator('[data-sonner-toast][data-type="success"]').count(), 0)
      await poll(async () => await page.locator('[data-sonner-toast][data-type="error"]').count() > 0, 'missing save error notification')
      await page.waitForTimeout(3500)
      assert.equal(writes().length - count, 1)
      assert.deepEqual(await control('snapshot'), dbBefore)
      await save('storage-error-observation.json', { windowMs: 3500, writeAttempts: writes().length - count, databaseUnchanged: true, expectedErrorToastVisible: true })
      await capture('storage-error-no-retry')
    } finally { await control('restore-grants') }
  })
  await run('commit-unknown-no-retry', async () => {
    await noticePage()
    const count = writes().length
    await control('arm-commit-loss')
    await saveNotice('Committed but reply lost — reconcile by reading.', 503, 'SYSTEM_OPTIONS_COMMIT_UNKNOWN')
    assert.equal(await page.locator('[data-sonner-toast][data-type="success"]').count(), 0)
    await poll(async () => await page.locator('[data-sonner-toast][data-type="error"]').count() > 0, 'missing UNKNOWN notification')
    await page.waitForTimeout(3500)
    assert.equal(writes().length - count, 1)
    const fault = await control('fault-evidence')
    assert.equal(fault.dropped, true)
    assert.deepEqual(fault.errors, [])
    await save('commit-fault-browser.json', { ...fault, windowMs: 3500, writeAttempts: writes().length - count })
    await capture('commit-unknown-no-retry')
    assert.equal((await optionRead()).Notice, 'Committed but reply lost — reconcile by reading.')
    await page.reload()
    await ready(page.getByLabel('Announcement content', { exact: true }))
    assert.equal(await page.getByLabel('Announcement content', { exact: true }).inputValue(), 'Committed but reply lost — reconcile by reading.')
    await capture('commit-unknown-resolved-by-refresh')
  })
  const finance = [
    { route: '/subscriptions', endpoint: '/core/v1/subscriptions/admin/plans', contract: 'core.subscriptions.v1', list: 'plans', create: 'Create Plan', field: 'Plan Name', close: 'Cancel', title: 'Browser synthetic plan', terms: 'terms' },
    { route: '/redemption-codes', endpoint: '/core/v1/promotions/admin/campaigns', contract: 'core.promotions.v1', list: 'campaigns', create: 'Create campaign', field: 'Name', close: 'Close', title: 'Browser synthetic campaign', terms: 'spec' },
  ]
  const created = []
  async function financeList(item) {
    const response = await page.context().request.get(cfg.origin + item.endpoint, { headers: { Authorization: 'Bearer ' + token } })
    assert.equal(response.status(), 200)
    const body = await response.json()
    assert.equal(body.contract, item.contract)
    return body[item.list]
  }
  async function financePage(item) {
    const observed = page.waitForResponse(r => new URL(r.url()).pathname === item.endpoint && r.request().method() === 'GET')
    await page.goto(cfg.origin + item.route)
    const response = await observed
    assert.equal(response.status(), 200)
    assert.equal((await response.json()).contract, item.contract)
    await ready(page.getByRole('button', { name: item.create, exact: true }))
  }
  await run('finance-create-readback', async () => {
    const self = await page.context().request.get(cfg.origin + '/api/user/self', { headers: { Authorization: 'Bearer ' + token } })
    assert.equal(self.status(), 200)
    const selfBody = await self.json()
    assert.equal(selfBody.success, true)
    assert.equal(selfBody.data.wallet_unit, 'core_credit')
    for (const item of finance) {
      for (const width of [1440, 390, 320]) {
        await page.setViewportSize({ width, height: width === 1440 ? 1000 : 844 })
        await financePage(item)
        const before = await financeList(item)
        const requestsBefore = financeRequests.length
        await page.getByRole('button', { name: item.create, exact: true }).click()
        const dialog = page.getByRole('dialog', { name: item.create })
        await ready(dialog)
        await dialog.getByLabel(item.field, { exact: true }).fill(item.title + ' canceled')
        assert.equal(await dialog.getByRole('switch', { name: 'Enabled' }).isChecked(), false)
        assert.ok(await dialog.getByRole('button', { name: 'Confirm', exact: true }).isEnabled())
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'page overflow ' + width)
        const bounds = await dialog.boundingBox()
        assert.ok(bounds && bounds.x >= -1 && bounds.x + bounds.width <= width + 1, 'dialog overflow ' + width)
        await dialog.getByRole('button', { name: 'Confirm', exact: true }).scrollIntoViewIfNeeded()
        assert.ok(await dialog.getByRole('button', { name: 'Confirm', exact: true }).isVisible())
        await capture('finance-dialog-' + item.list + '-' + width)
        await dialog.getByRole('button', { name: item.close, exact: true }).click()
        await dialog.waitFor({ state: 'hidden' })
        assert.equal(financeRequests.slice(requestsBefore).filter(x => x.method !== 'GET').length, 0, 'cancel sent finance write')
        assert.deepEqual(await financeList(item), before, 'cancel changed native list')
      }
      await page.setViewportSize({ width: 1440, height: 1000 })
      await financePage(item)
      const before = await financeList(item)
      assert.equal(before.filter(x => x[item.terms].title === item.title).length, 0)
      const start = financeRequests.length
      await page.getByRole('button', { name: item.create, exact: true }).click()
      const dialog = page.getByRole('dialog', { name: item.create })
      await ready(dialog)
      await dialog.getByLabel(item.field, { exact: true }).fill(item.title)
      assert.equal(await dialog.getByRole('switch', { name: 'Enabled' }).isChecked(), false)
      assert.equal(financeRequests.slice(start).filter(x => x.method !== 'GET').length, 0, 'write before confirmation')
      const pending = page.waitForResponse(r => new URL(r.url()).pathname === item.endpoint && r.request().method() === 'POST')
      await dialog.getByRole('button', { name: 'Confirm', exact: true }).click()
      const response = await pending
      assert.equal(response.status(), 200)
      const body = await response.json(), request = response.request().postDataJSON()
      assert.ok(body.id)
      assert.equal(request.expected_revision, null)
      assert.ok(request.operation_key)
      assert.equal(request[item.terms].title, item.title)
      assert.equal(request[item.terms].enabled, false)
      assert.equal(body[item.terms].title, item.title)
      assert.equal(body[item.terms].enabled, false)
      await dialog.waitFor({ state: 'hidden' })
      assert.equal(financeRequests.slice(start).filter(x => x.method !== 'GET').length, 1)
      const listed = await financeList(item)
      assert.deepEqual(listed.filter(x => x.id === body.id), [body])
      await page.reload()
      await ready(page.getByText(item.title, { exact: false }))
      await capture('finance-created-' + item.list)
      created.push({ item, body, request })
    }
    assert.equal(financeRequests.filter(x => /^\/api\/(subscription|redemption)/.test(x.path)).length, 0, 'legacy finance endpoint used')
  })
  await run('finance-restart-readback', async () => {
    const restarted = await control('restart-finance-core')
    assert.notEqual(restarted.old_pid, restarted.new_pid)
    for (const { item, body } of created) {
      await financePage(item)
      assert.deepEqual((await financeList(item)).filter(x => x.id === body.id), [body])
      await ready(page.getByText(item.title, { exact: false }))
      await capture('finance-restarted-' + item.list)
    }
    assert.equal(financeRequests.filter(x => /^\/api\/(subscription|redemption)/.test(x.path)).length, 0)
  })
  if (cfg.noticeRetest) {
    await page.context().close()
    page = await context()
    const identity = await login('consoleordinary', cfg.ordinaryPassword, '/system-settings/site/header-navigation')
    assert.ok(identity.platform_level < 6)
  } else await ordinaryNoWrite()
    await run('finance-ordinary-denial', async () => {
      for (const { item, request } of created) {
        const response = await page.context().request.get(cfg.origin + item.endpoint, { headers: { Authorization: 'Bearer ' + token } })
        assert.equal(response.status(), 403)
        const code = item.list === 'plans' ? 'SUBSCRIPTION_FORBIDDEN' : 'PROMOTION_FORBIDDEN'
        assert.equal((await response.json()).error.code, code)
        const denied = await page.context().request.post(cfg.origin + item.endpoint, { headers: { Authorization: 'Bearer ' + token }, data: { ...request, operation_key: 'ordinary-denied-' + item.list } })
        assert.equal(denied.status(), 403)
        assert.equal((await denied.json()).error.code, code)
      }
    })
    await run('navigation-ordinary-denial', async () => {
      await page.goto(cfg.origin + '/system-settings/site/header-navigation')
      await page.waitForLoadState('networkidle')
      assert.equal(await page.getByRole('button', { name: 'Save navigation', exact: true }).count(), 0)
      const before = await page.context().request.get(cfg.origin + '/api/status')
      assert.equal(before.status(), 200)
      const beforeBody = await before.json()
      assert.equal(beforeBody.success, true)
      const original = beforeBody.data.HeaderNavModules
      const response = await page.context().request.put(cfg.origin + '/api/option/', {
        headers: { Authorization: 'Bearer ' + token },
        data: { key: 'HeaderNavModules', value: JSON.stringify({ directory: !JSON.parse(original).directory }) },
      })
      assert.equal(response.status(), 403)
      const body = await response.json()
      assert.equal(body.success, false)
      assert.equal(body.code, 'SYSTEM_OPTIONS_FORBIDDEN')
      const status = await page.context().request.get(cfg.origin + '/api/status')
      assert.equal(status.status(), 200)
      const publicBody = await status.json()
      assert.equal(publicBody.success, true)
      assert.equal(publicBody.data.HeaderNavModules, original)
      await capture('navigation-ordinary-denied')
    })
  }
  }
} catch (error) {
  cases[current] = 'FAIL'
  await save('browser-failure.json', { case: current, message: error.message, stack: error.stack, url: page?.url() })
  if (page && !page.isClosed()) { try { await capture('FAIL-' + current) } catch {} }
  console.error('FAIL', current, error.message)
  process.exitCode = 1
} finally {
  if (browser) await browser.close()
  await save('browser-events.json', events)
  await save('write-attempts.json', writeAttempts)
  await save('finance-requests.json', financeRequests)
  await save('browser-errors.json', { pageErrors: errors, blockedExternalRequests: external })
  await save('browser-result.json', { overall: cfg.ordinaryOnly && !process.exitCode ? 'INDEPENDENT_CASE_PASS' : cfg.diagnoseSelector && !process.exitCode ? 'DIAGNOSTIC_ONLY' : Object.values(cases).every(x => x === 'PASS' || x === 'REUSED_PASS') ? 'PASS' : 'INCOMPLETE', cases })
}
