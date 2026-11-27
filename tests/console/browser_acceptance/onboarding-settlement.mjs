// SPDX-License-Identifier: AGPL-3.0-or-later
// Invoked by the SQLx fixture. Business requests always go to its real Core listener.
import assert from 'node:assert/strict'
import http from 'node:http'
import { readFile } from 'node:fs/promises'
import { extname, resolve, sep } from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href)
const web = 'http://127.0.0.1:41781'
const core = new URL(process.env.BROWSER_CORE_URL)
assert.equal(core.hostname, '127.0.0.1')
const dist = resolve(process.env.BROWSER_WEB_DIST)
const root = '/core/v1/admin/routing'
const config = root + '/configuration'
const channel = root + '/channels/1/models'
const model = 'browser-supplier-new'
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }
const front = http.createServer(async (req, res) => {
  const url = new URL(req.url, web)
  if (/^\/(api|core|v1|health)(\/|$)/.test(url.pathname)) {
    const upstream = http.request(new URL(url.pathname + url.search, core), {
      method: req.method, headers: req.headers,
    }, reply => {
      res.writeHead(reply.statusCode, reply.headers)
      reply.pipe(res)
    })
    upstream.on('error', () => { res.writeHead(502); res.end() })
    req.pipe(upstream)
    return
  }
  if (req.method !== 'GET' && req.method !== 'HEAD') { res.writeHead(405); res.end(); return }
  try {
    const asset = resolve(dist, '.' + decodeURIComponent(url.pathname))
    assert.ok(asset === dist || asset.startsWith(dist + sep))
    const file = req.headers.accept?.includes('text/html') ? resolve(dist, 'index.html') : asset
    const bytes = await readFile(file)
    res.writeHead(200, { 'content-type': mime[extname(file)] ?? 'application/octet-stream' })
    res.end(bytes)
  } catch { res.writeHead(404); res.end() }
})

async function session(browser, username, destination) {
  const context = await browser.newContext({ locale: 'en-US' })
  await context.route('**/*', route => new URL(route.request().url()).origin === web
    ? route.continue() : route.abort())
  const page = await context.newPage()
  await page.goto(web + '/sign-in?redirect=' + encodeURIComponent(destination))
  await page.getByLabel('Username or Email', { exact: true }).fill(username)
  await page.getByLabel('Password', { exact: true }).fill(process.env.BROWSER_ACCOUNT_PASSWORD)
  const [response] = await Promise.all([
    page.waitForResponse(r => new URL(r.url()).pathname === '/api/user/login' && r.request().method() === 'POST'),
    page.getByRole('button', { name: 'Sign in', exact: true }).click(),
  ])
  assert.equal(response.status(), 200)
  const login = await response.json()
  assert.equal(login.success, true)
  await page.waitForURL(url => url.pathname.replace(/\/$/, '') === destination)
  return { page, token: login.data.access_token, user: login.data.user.id }
}

async function request(page, token, path, body) {
  // A same-origin fetch from Chromium; keep credentials and complete response bodies out of artifacts.
  const result = await page.evaluate(async ({ token, path, body }) => {
    const response = await fetch(path, {
      method: body === undefined ? 'GET' : 'POST',
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    })
    return { status: response.status, id: response.headers.get('x-request-id'), json: await response.json() }
  }, { token, path, body })
  assert.equal(result.status, 200, path)
  return result
}

async function clickReceipt(page, button, path, method) {
  const [response] = await Promise.all([
    page.waitForResponse(r => new URL(r.url()).pathname === path && r.request().method() === method),
    button.click(),
  ])
  assert.equal(response.status(), 200, path)
  const result = await response.json()
  assert.equal(result.success, true, path)
  return { data: result.data, input: response.request().postDataJSON() }
}

async function onboard(browser) {
  const { page, token, user } = await session(browser, 'browseradmin', '/channels')
  assert.equal(user, 8)
  const read = async () => (await request(page, token, config)).json.data
  const initial = await read()
  const active = c => c.routing.routes.some(r => r.enabled && r.model === model)
  assert.equal(active(initial), false)
  assert.equal(initial.bindings.official_prices?.models?.[model], undefined)
  await page.getByRole('button', { name: 'Model onboarding', exact: true }).click()
  const dialog = page.getByRole('dialog')
  const button = name => dialog.getByRole('button', { name, exact: true })
  const seen = await clickReceipt(page, button('Sync models'), channel + '/synchronize', 'POST')
  assert.ok(seen.data.models.includes(model))
  assert.ok(seen.data.models.includes('browser-expensive-sibling'))
  assert.equal(seen.data.prices_inferred, false)
  await dialog.getByRole('checkbox', { name: model, exact: true }).check()
  const saved = await clickReceipt(page, button('Save pending models'), channel + '/pending', 'PUT')
  assert.equal(saved.data.routes_published, false)
  const row = dialog.getByRole('listitem').filter({ has: page.getByRole('checkbox', { name: model, exact: true }) })
  await row.getByText('Pending pricing', { exact: true }).waitFor()
  const pending = await read()
  assert.deepEqual(pending.bindings.discoveries['1'].pending_models, [model])
  assert.deepEqual(pending.routing.routes, initial.routing.routes)
  assert.deepEqual(pending.prices.rules, initial.prices.rules)
  assert.equal(pending.bindings.official_prices?.models?.[model], undefined)
  await row.getByRole('button', { name: 'Pricing', exact: true }).click()
  const price = dialog.getByRole('region', { name: 'Official base price' })
  await price.getByRole('heading', { name: model, exact: true }).waitFor()
  await price.locator('label:has(> span:text-is("Source")) select').selectOption('openrouter')
  assert.equal(await price.locator('label:has(> span:text-is("Channel")) select').inputValue(), '1')
  const fetched = await clickReceipt(page, button('Fetch upstream prices'), channel + '/prices/preview', 'POST')
  const quote = fetched.data.quote
  assert.deepEqual(quote.source, { kind: 'channel', channel_id: 1, upstream_model: model })
  assert.equal(quote.model, model)
  assert.equal(quote.rates.input, '2')
  assert.equal(quote.rates.output, '4')
  const editor = price.locator('textarea')
  await page.waitForFunction(() => Array.from(document.querySelectorAll('textarea')).some(t => t.value.includes('channel/1')))
  const toml = await editor.inputValue()
  for (const pattern of [/source\s*=\s*["']channel\/1["']/, /source_unit\s*=\s*["']USD\/token["']/, /^input\s*=\s*2000000$/m, /^output\s*=\s*4000000$/m]) assert.match(toml, pattern)
  const preview = await clickReceipt(page, button('Preview changes'), config + '/preview', 'POST')
  assert.equal(preview.data.saved, false)
  const priced = await clickReceipt(page, button('Publish prices'), config, 'PUT')
  assert.equal(priced.input.confirmation_digest, preview.data.confirmation_digest)
  await row.getByText('Pending activation', { exact: true }).waitFor()
  const publishedPrice = await read()
  assert.equal(active(publishedPrice), false)
  assert.equal(publishedPrice.bindings.official_prices.models[model].rates.input, 2000000)
  for (const [name, value] of [['Maximum input tokens', '100'], ['Maximum output tokens', '20'], ['Contract revision', 'browser-v1']]) {
    await dialog.getByLabel(name, { exact: true }).fill(value)
  }
  const route = await clickReceipt(page, button('Preview route'), root + '/models/preview', 'POST')
  assert.equal(active(await read()), false)
  const published = await clickReceipt(page, button('Publish route'), root + '/models', 'POST')
  assert.equal(published.input.confirmation_digest, route.data.confirmation_digest)
  await row.getByText('Enabled', { exact: true }).waitFor()
  const final = await read()
  const bound = final.routing.routes.find(r => r.model === model && r.enabled)
  assert.equal(bound.targets.length, 1)
  assert.equal(bound.targets[0].upstream_model, model)
  assert.equal(bound.targets[0].upstream_id, 1)
  await page.context().close()
  return { model, version: final.version, price_digest: preview.data.confirmation_digest, route_digest: route.data.confirmation_digest }
}

async function settle(browser) {
  const { page, token, user } = await session(browser, 'browserpayer', '/wallet')
  assert.equal(user, 7)
  const wallet = async () => {
    const { json: { data } } = await request(page, token, '/api/user/self')
    assert.equal(data.id, user)
    assert.equal(data.financial_projection_available, true)
    assert.equal(data.wallet_reserved_quota, 0)
    return data.wallet_spendable_quota
  }
  const before = await wallet()
  assert.equal(before, 1000)
  await page.locator('#wallet-balance').getByText(/^1,?000\s+Credits$/i).first().waitFor()
  const created = await request(page, token, '/api/token/', {
    name: 'Browser settlement proof', remain_quota: 500, unlimited_quota: false,
    expired_time: Math.floor(Date.now() / 1000) + 600, one_time_reveal: true,
    group: 'default', model_limits_enabled: true, model_limits: model,
    auto_groups: [], cross_group_retry: false, allow_ips: '127.0.0.1/32',
  })
  const key = created.json.data
  assert.ok(key.key.startsWith('lmmk_'))
  const paid = await request(page, key.key, '/v1/chat/completions', {
    model, messages: [{ role: 'user', content: 'browser proof' }], max_tokens: 20,
  })
  assert.equal(paid.json.choices[0].message.content, 'browser fixture answer')
  assert.ok(paid.id)
  const after = await wallet()
  assert.equal(after, before - 50)
  await page.getByRole('button', { name: 'Refresh account status' }).click()
  await page.locator('#wallet-balance').getByText(/^950\s+Credits$/i).first().waitFor()
  const { json: { data } } = await request(page, token, '/api/log/self?request_id=' + encodeURIComponent(paid.id))
  assert.equal(data.items.length, 1)
  const receipt = data.items[0]
  assert.equal(receipt.request_id, paid.id)
  assert.equal(receipt.user_id, user)
  assert.equal(receipt.token_id, key.id)
  assert.equal(receipt.model_name, model)
  assert.equal(receipt.state, 'settled')
  assert.equal(receipt.net_consumption_credits, 50)
  await page.goto(web + '/usage-logs/common?requestId=' + encodeURIComponent(paid.id))
  await page.getByRole('button', { name: 'View request details' }).first().click()
  await page.getByRole('dialog').getByText(paid.id, { exact: true }).first().waitFor()
  return { request_id: paid.id, user, key_id: key.id, before, after, settled: 50 }
}

await new Promise((ok, fail) => front.once('error', fail).listen(41781, '127.0.0.1', ok))
let browser
try {
  browser = await chromium.launch({ headless: true })
  const publication = await onboard(browser)
  const payment = await settle(browser)
  process.stdout.write(JSON.stringify({ publication, payment }))
} finally {
  await browser?.close()
  front.closeAllConnections()
  await new Promise(ok => front.close(ok))
}
