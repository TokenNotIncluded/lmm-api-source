// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { dirname, join } from 'node:path'

export const viewports = [
  { label: 'desktop', width: 1440, height: 900, touch: false },
  { label: 'phone', width: 390, height: 844, touch: true },
]

// Verify all wrapped glyph runs are inside the viewport and visible through the dialog.
async function readableRequestId(locator) {
  await locator.scrollIntoViewIfNeeded()
  return locator.evaluate(node => {
    const range = document.createRange()
    range.selectNodeContents(node)
    const rects = [...range.getClientRects()].filter(r => r.width > 0 && r.height > 0)
    return rects.length > 0 && rects.every(rect => {
      if (rect.left < 0 || rect.top < 0 || rect.right > innerWidth || rect.bottom > innerHeight) return false
      return node.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2))
    })
  })
}

export function observeApiFailures(page, web, failedRequests) {
  page.on('response', response => {
    const url = new URL(response.url())
    const status = response.status()
    if (url.origin !== web || !['fetch', 'xhr'].includes(response.request().resourceType())) return
    if (status !== 404 && status < 500) return
    // Keep the route template only. Query strings, IDs, and credentials are private.
    const route = url.pathname.split('/').map(part =>
      part === '' || /^[a-z][a-z_-]{0,30}$/.test(part) || /^v[0-9]+$/.test(part) ? part : '{id}'
    ).join('/')
    failedRequests.add(response.request().method() + ' ' + route + ' ' + status)
  })
}

export async function assertNoVisibleCredential(page) {
  const text = await page.locator('body').innerText()
  if (/\b(?:lmmk_|lmm_[ar]t_)[a-zA-Z0-9_-]{12,}|\bBearer\s+[a-zA-Z0-9_-]{12,}/i.test(text)) {
    throw new Error('A credential is visible in the console')
  }
}

export async function capturePaidLogViews(browser, session, payment, web) {
  const directory = join(dirname(process.env.BROWSER_RECEIPT), 'usage-log-details')
  await mkdir(directory, { recursive: true })
  const captures = []
  const failedRequests = new Set()
  const viewsWithErrors = []
  for (const { label, width, height, touch } of viewports) {
    const { page, user } = await session(browser, 'browserpayer', '/wallet', {
      viewport: { width, height }, isMobile: touch, hasTouch: touch, deviceScaleFactor: touch ? 3 : 1,
    })
    try {
      assert.equal(user, payment.user)
      observeApiFailures(page, web, failedRequests)
      // Use the application's real System preference and Chromium's OS color scheme.
      await page.context().addCookies([{ name: 'vite-ui-theme', value: 'system', url: web }])
      for (const theme of ['light', 'dark']) {
        await page.emulateMedia({ colorScheme: theme })
        await page.goto(web + '/usage-logs/common?requestId=' + encodeURIComponent(payment.request_id))
        await page.locator('html.' + theme).waitFor()
        const open = page.getByRole('button', { name: 'View request details', exact: true })
        if (touch) await open.tap()
        else await open.click()
        const dialog = page.getByRole('dialog')
        const id = dialog.getByRole('button', { name: 'Copy Request ID', exact: true })
          .locator('..').getByText(payment.request_id, { exact: true })
        await id.waitFor()
        await page.evaluate(async () => { await document.fonts.ready })
        assert.equal(await readableRequestId(id), true, 'The complete request ID must be readable in ' + label + ' ' + theme)
        await assertNoVisibleCredential(page)
        const filename = label + '-' + width + 'x' + height + '-' + theme + '.png'
        await page.screenshot({ path: join(directory, filename), scale: 'css' })
        if (await page.locator('[data-sonner-toast][data-type="error"]:visible').count()) {
          viewsWithErrors.push(label + '-' + theme)
        }
        captures.push({ file: 'usage-log-details/' + filename, width, height, theme, touch, request_id: payment.request_id })
      }
    } finally {
      await page.context().close()
    }
  }
  assert.deepEqual({ requests: [...failedRequests], viewsWithErrors }, { requests: [], viewsWithErrors: [] }, 'Ordinary request details must load without failed API requests or visible errors')
  return captures
}
