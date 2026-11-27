// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { assertNoVisibleCredential, observeApiFailures, viewports } from './usage-log-captures.mjs'

export async function captureConsolePages(browser, session, web, payment, publication) {
  const directory = join(dirname(process.env.BROWSER_RECEIPT), 'console-pages')
  await mkdir(directory, { recursive: true })
  const roles = [
    {
      username: 'browserpayer', user: payment.user, landing: '/wallet',
      pages: [
        { name: 'wallet', path: '/wallet', marker: page => page.locator('#wallet-balance').getByText(/^950\s+Credits$/i).first() },
        { name: 'keys', path: '/keys', marker: page => page.getByText('Browser settlement proof', { exact: true }).first() },
        { name: 'usage-logs', path: '/usage-logs/common?requestId=' + encodeURIComponent(payment.request_id),
          marker: page => page.getByRole('button', { name: 'View request details', exact: true }).first() },
      ],
    },
    {
      username: 'browseradmin', user: 8, landing: '/channels',
      pages: [
        { name: 'channels', path: '/channels', marker: page => page.getByRole('button', { name: 'Model onboarding', exact: true }) },
        { name: 'pricing', path: '/admin/pricing',
          marker: page => page.getByText('Published version: ' + publication.version.pricing, { exact: true }).first() },
      ],
    },
  ]
  const captures = []
  for (const { label, width, height, touch } of viewports) {
    for (const role of roles) {
      const { page, user } = await session(browser, role.username, role.landing, {
        viewport: { width, height }, isMobile: touch, hasTouch: touch, deviceScaleFactor: touch ? 3 : 1,
      })
      try {
        assert.equal(user, role.user)
        await page.context().addCookies([{ name: 'vite-ui-theme', value: 'system', url: web }])
        const failedRequests = new Set()
        observeApiFailures(page, web, failedRequests)
        for (const theme of ['light', 'dark']) {
          await page.emulateMedia({ colorScheme: theme })
          for (const view of role.pages) {
            const filename = view.name + '-' + label + '-' + theme + '.png'
            console.error('Checking real console view ' + filename)
            failedRequests.clear()
            await page.goto(web + view.path)
            await page.locator('html.' + theme).waitFor()
            await view.marker(page).waitFor()
            await page.waitForLoadState('networkidle')
            await page.evaluate(async () => { await document.fonts.ready })
            await assertNoVisibleCredential(page)
            const errors = await page.locator('[data-sonner-toast][data-type="error"]:visible').count()
            assert.deepEqual({ failedRequests: [...failedRequests], errors }, { failedRequests: [], errors: 0 }, filename)
            await page.screenshot({ path: join(directory, filename), scale: 'css' })
            captures.push({ page: view.name, file: 'console-pages/' + filename, width, height, theme, touch })
          }
        }
      } finally {
        await page.context().close()
      }
    }
  }
  return captures
}
