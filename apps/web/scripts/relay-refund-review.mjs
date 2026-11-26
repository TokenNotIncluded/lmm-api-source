/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// Run from apps/web with bun scripts/relay-refund-review.mjs. All order data is local.
import { mkdtemp, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

const executable =
  process.env.CHROMIUM_BIN ||
  Bun.which('chromium') ||
  Bun.which('google-chrome')
if (!executable) {
  throw new Error('Set CHROMIUM_BIN to an installed Chromium executable')
}

const bundle = await Bun.build({
  entrypoints: [
    path.join(import.meta.dir, 'fixtures/relay-refund-checkout.tsx'),
  ],
  target: 'browser',
  tsconfig: path.join(import.meta.dir, '../tsconfig.app.json'),
  jsx: { runtime: 'automatic' },
  define: {
    'process.env.NODE_ENV': '"production"',
    'import.meta.env': JSON.stringify({
      MODE: 'test',
      DEV: false,
      PROD: false,
    }),
  },
})
if (!bundle.success) {
  throw new Error(bundle.logs.join('\n'))
}
const app = bundle.outputs.find((file) => file.path.endsWith('.js'))
if (!app) {
  throw new Error('Missing browser bundle')
}
const result = Promise.withResolvers()
const server = Bun.serve({
  hostname: '127.0.0.1',
  port: 0,
  fetch: async (req) => {
    const url = new URL(req.url)
    if (url.pathname === '/result' && req.method === 'POST') {
      result.resolve(await req.text())
      return new Response('OK')
    }
    if (url.pathname === '/app.js') {
      return new Response(app, {
        headers: { 'Content-Type': 'text/javascript' },
      })
    }
    if (url.pathname !== '/') {
      return new Response(null, { status: 404 })
    }
    return new Response(
      '<!doctype html><meta charset="utf-8"><div id="root"></div><script type="module" src="/app.js"></script>',
      {
        headers: {
          'Content-Type': 'text/html',
          'Content-Security-Policy':
            "default-src 'self'; connect-src 'self'; style-src 'self' 'unsafe-inline'",
        },
      }
    )
  },
})
const profile = await mkdtemp(
  path.join(os.tmpdir(), 'lmm-relay-refund-browser-')
)
let browser, timeout
try {
  browser = Bun.spawn(
    [
      executable,
      '--headless',
      '--disable-gpu',
      '--disable-background-networking',
      '--no-first-run',
      '--disable-dev-shm-usage',
      '--user-data-dir=' + profile,
      server.url.href,
    ],
    { stdout: 'ignore', stderr: 'ignore' }
  )
  const outcome = await Promise.race([
    result.promise,
    new Promise((_, reject) => {
      timeout = setTimeout(
        () => reject(new Error('Browser did not return a test result')),
        90000
      )
    }),
    browser.exited.then((code) => {
      throw new Error('Chromium exited before the test: ' + code)
    }),
  ])
  if (outcome !== 'PASS') {
    throw new Error(outcome)
  }
  console.log(
    'PASS: refund support, pending and refunded states, receipt isolation, one payment request'
  )
} finally {
  clearTimeout(timeout)
  if (browser) {
    browser.kill()
    await browser.exited
  }
  server.stop(true)
  await rm(profile, {
    recursive: true,
    force: true,
    maxRetries: 3,
    retryDelay: 50,
  })
}
