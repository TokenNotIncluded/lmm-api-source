/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

const { chromium } = await import(
  process.env.PLAYWRIGHT_MODULE ??
    '/home/lightjunction/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright/index.mjs'
)
const originUrl = new URL(
  process.env.TEAM_PROFILE_REVIEW_ORIGIN ?? 'http://127.0.0.1:40571'
)
assert.equal(originUrl.protocol, 'http:')
assert.equal(originUrl.hostname, '127.0.0.1')
assert.equal(
  originUrl.username + originUrl.password + originUrl.search + originUrl.hash,
  ''
)
assert.equal(originUrl.pathname, '/')
const origin = originUrl.origin
const output = process.env.TEAM_PROFILE_REVIEW_OUTPUT
assert.ok(output, 'TEAM_PROFILE_REVIEW_OUTPUT is required')
await mkdir(output, { recursive: true })
const language = process.env.TEAM_PROFILE_REVIEW_LANGUAGE ?? 'zh-CN'
assert.ok(['en', 'zh-CN', 'zh-TW', 'fr', 'ru', 'ja', 'vi'].includes(language))
const locale = JSON.parse(
  await readFile(
    path.resolve(
      import.meta.dirname,
      '../src/i18n/locales',
      `${language}.json`
    ),
    'utf8'
  )
).translation
const label = (key) => locale[key] ?? key
const targets = [
  { name: 'users', destination: '/users', persona: 'admin' },
  { name: 'profile', destination: '/profile', persona: 'l1' },
  { name: 'account', destination: '/profile?tab=account', persona: 'l1' },
  { name: 'teams', destination: '/profile?tab=teams', persona: 'l1' },
]
const sizes = [
  {
    id: 'desktop-light',
    width: 1440,
    height: 1000,
    theme: 'light',
    interact: false,
  },
  {
    id: 'desktop-dark',
    width: 1440,
    height: 1000,
    theme: 'dark',
    interact: true,
  },
  {
    id: 'mobile-light',
    width: 390,
    height: 844,
    theme: 'light',
    interact: false,
  },
  {
    id: 'compact-dark',
    width: 320,
    height: 740,
    theme: 'dark',
    interact: true,
  },
]
const names =
  process.env.TEAM_PROFILE_REVIEW_PAGES?.split(',') ??
  targets.map((target) => target.name)
const scenarios =
  process.env.TEAM_PROFILE_REVIEW_SCENARIOS?.split(',') ??
  sizes.map((size) => size.id)
for (const name of names) {
  assert.ok(
    targets.some((target) => target.name === name),
    `Unknown page: ${name}`
  )
}
for (const scenario of scenarios) {
  assert.ok(
    sizes.some((size) => size.id === scenario),
    `Unknown scenario: ${scenario}`
  )
}
const report = {
  synthetic: true,
  scope:
    'Loopback UI fixtures only. Backend permission and budget enforcement require separate real-service tests.',
  origin,
  language,
  results: [],
}
const saveReport = () =>
  writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2))
const browser = await chromium.launch({
  headless: true,
  executablePath:
    process.env.CHROME ??
    process.env.PLAYWRIGHT_CHROME_EXECUTABLE ??
    '/usr/bin/chromium',
})

async function capture(page, info, state, suffix = '') {
  await page.evaluate(() => document.fonts.ready)
  await page.waitForTimeout(150)
  const dimensions = await page.evaluate(() => ({
    viewport: innerWidth,
    document: document.documentElement.scrollWidth,
    dialogs: [...document.querySelectorAll('[role="dialog"]')]
      .filter((node) => node.getBoundingClientRect().width > 0)
      .map((node) => {
        const rect = node.getBoundingClientRect()
        return { left: rect.left, right: rect.right }
      }),
    clippedControls: [
      ...document.querySelectorAll('button,input,select,[role="tab"]'),
    ]
      .filter((node) => {
        const rect = node.getBoundingClientRect()
        if (
          !rect.width ||
          !rect.height ||
          (rect.left >= -1 && rect.right <= innerWidth + 1)
        ) {
          return false
        }
        if (node.closest('[data-slot="sidebar-container"]')) return false
        for (
          let parent = node.parentElement;
          parent;
          parent = parent.parentElement
        ) {
          if (
            ['auto', 'scroll', 'hidden', 'clip'].includes(
              getComputedStyle(parent).overflowX
            )
          ) {
            return false
          }
        }
        return true
      })
      .map((node) => ({
        text: (node.textContent ?? '').slice(0, 80),
        label: node.getAttribute('aria-label'),
      }))
      .slice(0, 10),
  }))
  const traces = await page.evaluate(() => ({
    teams: window.__LMM_TEAM_PROFILE_REVIEW__,
    native: window.__LMM_NATIVE_UI_REVIEW__,
  }))
  const text = await page.locator('body').innerText()
  const errors = await page
    .locator('[data-sonner-toast][data-type="error"]:visible')
    .allTextContents()
  const file = `${info.name}-${info.id}${suffix}.png`
  await page.screenshot({
    path: path.join(output, file),
    fullPage: true,
    animations: 'disabled',
  })
  const consoleErrors = state.consoleErrors.filter(
    (error) => !error.includes('net::ERR_BLOCKED_BY_CLIENT')
  )
  const result = {
    ...info,
    suffix,
    file,
    synthetic: true,
    dimensions,
    traces,
    pageErrors: [...state.pageErrors],
    consoleErrors,
    networkEscapes: [...state.networkEscapes],
    blockedAssets: [...state.blockedAssets],
    errorToasts: errors,
    text: text.slice(0, 14000),
  }
  result.pass =
    dimensions.document <= dimensions.viewport + 1 &&
    dimensions.dialogs.every(
      (dialog) => dialog.left >= -1 && dialog.right <= dimensions.viewport + 1
    ) &&
    !dimensions.clippedControls.length &&
    !result.pageErrors.length &&
    !consoleErrors.length &&
    !result.networkEscapes.length &&
    !errors.length &&
    !/PERSONA_DEBUG_UNMOCKED_REQUEST|Something went wrong|Internal Server Error|Invalid language tag|糟糕！出错了/.test(
      text
    )
  report.results.push(result)
  await saveReport()
  console.log(
    JSON.stringify({
      page: info.name,
      scenario: info.id,
      suffix,
      pass: result.pass,
      file,
    })
  )
}

async function editBudget(page, info, state, region, suffix) {
  await region
    .getByRole('button', { name: label('Edit'), exact: true })
    .first()
    .click()
  const dialog = page.getByRole('dialog', {
    name: label('Edit spending limit'),
    exact: true,
  })
  await dialog
    .getByLabel(label('Limit (credits)'), { exact: true })
    .fill('6000000')
  await capture(page, info, state, `${suffix}-edit`)
  await dialog.locator('button[type="submit"]').click()
  await dialog.waitFor({ state: 'hidden' })
  assert.equal(
    await region.getByRole('progressbar').first().getAttribute('aria-valuenow'),
    '25'
  )
  await capture(page, info, state, `${suffix}-saved`)
}

async function review(target, size) {
  const info = { ...target, ...size }
  const context = await browser.newContext({
    viewport: { width: size.width, height: size.height },
    hasTouch: size.width < 500,
    reducedMotion: 'reduce',
    colorScheme: size.theme,
    locale: language,
    serviceWorkers: 'block',
  })
  const state = {
    pageErrors: [],
    consoleErrors: [],
    networkEscapes: [],
    blockedAssets: [],
  }
  await context.addInitScript(
    ({ language, theme }) => {
      localStorage.setItem('i18nextLng', language)
      localStorage.setItem('lmm:source-consent:v2', 'no')
      localStorage.setItem('theme', theme)
    },
    { language, theme: size.theme }
  )
  await context.route('**/*', async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    if (url.origin !== origin) {
      const external = `${request.method()} ${url.origin}${url.pathname}`
      if (
        url.href === 'https://cdn.agentlane.com/v1/snippet.js' ||
        !['fetch', 'xhr', 'script', 'websocket'].includes(
          request.resourceType()
        )
      ) {
        state.blockedAssets.push(external)
      } else state.networkEscapes.push(external)
      return route.abort('blockedbyclient')
    }
    if (request.method() === 'GET' && url.pathname === '/api/status') {
      return route.fulfill({
        json: {
          success: true,
          data: {
            system_name: 'Synthetic UI Review',
            logo: '/logo.png',
            assistant: { enabled: false },
            announcements_enabled: false,
            two_factor_enabled: false,
            checkin_enabled: false,
          },
        },
      })
    }
    if (
      /^\/(api|core|v1)\//.test(url.pathname) ||
      !['GET', 'HEAD'].includes(request.method())
    ) {
      state.networkEscapes.push(`${request.method()} ${url.pathname}`)
      return route.abort('blockedbyclient')
    }
    return route.continue()
  })
  const page = await context.newPage()
  page.setDefaultTimeout(15000)
  page.on('pageerror', (error) =>
    state.pageErrors.push(String(error.stack ?? error))
  )
  page.on('console', (message) => {
    if (message.type() === 'error') state.consoleErrors.push(message.text())
  })
  try {
    const url = new URL(target.destination, origin)
    for (const [key, value] of Object.entries({
      debug_persona: target.persona,
      console_review: '1',
      team_profile_review: '1',
      management_review: '1',
      logs_review: '1',
      native_ui_review: '1',
    })) {
      url.searchParams.set(key, value)
    }
    await page.goto(url.href, { waitUntil: 'domcontentloaded', timeout: 45000 })
    await page.getByTestId('persona-debug-trigger').waitFor({ timeout: 45000 })
    await page.evaluate(
      (theme) =>
        document.documentElement.classList.toggle('dark', theme === 'dark'),
      size.theme
    )
    if (target.name === 'users') {
      await page.locator('[data-native-user-id]').first().waitFor()
    }
    if (target.name === 'account') {
      await page
        .getByRole('region', { name: label('Personal budget'), exact: true })
        .getByRole('progressbar')
        .first()
        .waitFor()
    }
    if (target.name === 'teams') {
      await page
        .getByRole('button')
        .filter({ hasText: 'Synthetic research team' })
        .first()
        .waitFor()
    }
    await capture(page, info, state)
    if (target.name === 'users') {
      await page
        .getByRole('button', { name: label('Create user'), exact: true })
        .click()
      const dialog = page.getByRole('dialog', {
        name: label('Create user'),
        exact: true,
      })
      await dialog.locator('#native-create-username').fill('synthetic-review')
      await dialog.locator('#native-create-password').fill('synthetic-password')
      await dialog.locator('button[type="submit"]').click({ trial: true })
      await capture(page, info, state, '-create-user')
      await page.keyboard.press('Escape')
      await dialog.waitFor({ state: 'hidden' })
    }
    if (!size.interact) return
    if (target.name === 'account') {
      await editBudget(
        page,
        info,
        state,
        page.getByRole('region', {
          name: label('Personal budget'),
          exact: true,
        }),
        '-personal-budget'
      )
    }
    if (target.name === 'teams') {
      const panel = page.getByRole('region', {
        name: label('Teams'),
        exact: true,
      })
      await panel
        .getByRole('tab', { name: label('Spending limits'), exact: true })
        .click()
      const budget = panel.getByRole('region', {
        name: label('Team budget'),
        exact: true,
      })
      await budget.getByRole('progressbar').first().waitFor()
      await editBudget(page, info, state, budget, '-team-budget')
      await panel
        .getByRole('button')
        .filter({ hasText: 'Synthetic observer team' })
        .click()
      await panel
        .getByRole('tab', { name: label('Spending limits'), exact: true })
        .click()
      await budget.getByRole('progressbar').first().waitFor()
      assert.equal(
        await budget
          .getByRole('button', { name: label('Edit'), exact: true })
          .count(),
        0
      )
      assert.equal(
        await budget
          .getByRole('button', {
            name: label('Add spending limit'),
            exact: true,
          })
          .count(),
        0
      )
      assert.equal(
        await panel
          .getByRole('tab', { name: label('Settings'), exact: true })
          .count(),
        0
      )
      await capture(page, info, state, '-member-readonly')
      await panel
        .getByRole('button', { name: label('Create team'), exact: true })
        .click()
      const dialog = page.getByRole('dialog', {
        name: label('Create team'),
        exact: true,
      })
      await dialog.locator('#new-team-name').fill('Synthetic review team')
      await capture(page, info, state, '-create-team')
      await dialog.locator('button[type="submit"]').click()
      await dialog.waitFor({ state: 'hidden' })
      await panel
        .getByRole('button')
        .filter({ hasText: 'Synthetic review team' })
        .waitFor()
      assert.equal(
        await panel
          .getByRole('button')
          .filter({ hasText: 'Synthetic review team' })
          .getAttribute('aria-pressed'),
        'true'
      )
      await capture(page, info, state, '-team-created')
    }
  } catch (error) {
    const file = `${info.name}-${info.id}-failure.png`
    await page
      .screenshot({ path: path.join(output, file), fullPage: true })
      .catch(() => {})
    report.results.push({
      ...info,
      synthetic: true,
      pass: false,
      file,
      error: String(error),
      dialogText: await page
        .getByRole('dialog')
        .allTextContents()
        .catch(() => []),
      traces: await page
        .evaluate(() => ({
          teams: window.__LMM_TEAM_PROFILE_REVIEW__,
          native: window.__LMM_NATIVE_UI_REVIEW__,
        }))
        .catch(() => null),
      invalidInputs: await page
        .locator('input:invalid')
        .evaluateAll((inputs) =>
          inputs.map((input) => ({
            id: input.id,
            value: input.value,
            message: input.validationMessage,
          }))
        )
        .catch(() => []),
      ...state,
    })
    await saveReport()
    console.log(
      JSON.stringify({
        page: info.name,
        scenario: info.id,
        pass: false,
        error: String(error),
      })
    )
  } finally {
    await context.close()
  }
}

try {
  for (const size of sizes.filter((item) => scenarios.includes(item.id))) {
    for (const target of targets.filter((item) => names.includes(item.name))) {
      await review(target, size)
    }
  }
} finally {
  await browser.close()
  await saveReport()
}
const failures = report.results.filter((result) => !result.pass)
console.log(
  JSON.stringify({
    captured: report.results.length,
    failed: failures.length,
    output,
  })
)
assert.equal(
  failures.length,
  0,
  failures
    .map(
      (entry) =>
        `${entry.name}/${entry.id}/${entry.suffix ?? ''}: ${
          entry.error ??
          JSON.stringify({
            dimensions: entry.dimensions,
            pageErrors: entry.pageErrors,
            consoleErrors: entry.consoleErrors,
            networkEscapes: entry.networkEscapes,
          })
        }`
    )
    .join('\n')
)
