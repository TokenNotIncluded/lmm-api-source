// SPDX-License-Identifier: AGPL-3.0-or-later
// Invoked by the SQLx fixture. Business requests always go to its real Core listener.
import assert from 'node:assert/strict'
import { runAccountBrowser } from './identity-account.mjs'
import { captureConsolePages } from './core-page-captures.mjs'
import { capturePaidLogViews } from './usage-log-captures.mjs'
import { checkTerminalLog, exerciseTerminal } from './terminal-acceptance.mjs'
import http from 'node:http'
import { readFile } from 'node:fs/promises'
import { extname, resolve, sep } from 'node:path'
import { pathToFileURL } from 'node:url'

const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href)
const web = 'http://127.0.0.1:41781'
const core = new URL(process.env.BROWSER_CORE_URL)
assert.equal(core.hostname, '127.0.0.1')
const extensions = new URL(process.env.BROWSER_EXTENSIONS_URL)
assert.equal(extensions.protocol, 'http:')
assert.equal(extensions.hostname, '127.0.0.1')
assert.equal(extensions.href, extensions.origin + '/')
const pluginPath = '/api/extensions/plugins'
const optionsPath = '/api/option/'
const extensionRoutes = new Set([
  pluginPath,
  '/api/site/navigation', '/api/release-notes/latest', '/api/hero-sms/capabilities', '/api/user/payments/channels',
])
const dist = resolve(process.env.BROWSER_WEB_DIST)
const root = '/core/v1/admin/routing'
const config = root + '/configuration'
const scenario = JSON.parse(process.env.BROWSER_SCENARIO)
const { model, sibling, group, otherGroup } = scenario
assert.equal(new Set([model, sibling, group, otherGroup]).size, 4)
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }
const front = http.createServer(async (req, res) => {
  const url = new URL(req.url, web)
  if (/^\/(api|core|v1|health)(\/|$)/.test(url.pathname)) {
    const owner = extensionRoutes.has(url.pathname) ? extensions : core
    const upstream = http.request(new URL(url.pathname + url.search, owner), {
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

async function session(browser, username, destination, contextOptions = {}, consentOrigin) {
  const context = await browser.newContext({ locale: 'en-US', ...contextOptions })
  await context.route('**/*', route => {
    const origin = new URL(route.request().url()).origin
    return origin === web || origin === consentOrigin ? route.continue() : route.abort()
  })
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
  const user = login.data.user.id
  const sid = login.data.session.sid
  assert.match(sid, /^[a-f0-9]{64}$/)
  return { page, identity: { sid, user }, user }
}

async function request(page, identity, path, body, method = body === undefined ? 'GET' : 'POST', expectedStatus = 200) {
  const result = await page.evaluate(async ({ identity, path, body, method }) => {
    async function currentBearer() {
      if (typeof identity === 'string') {
        if (!path.startsWith('/v1/')) throw new Error('API key used outside the model API')
        return identity
      }
      if (!/^\/(?:api|core)\//.test(path)) throw new Error('Invalid dashboard request path')
      const proof = await fetch('/api/user/auth/reconcile', {
        method: 'POST', credentials: 'same-origin', mode: 'same-origin', redirect: 'error',
        cache: 'no-store', headers: { 'X-Auth-Session': identity.sid },
      })
      const value = await proof.json()
      const bundle = value.data?.bundle
      if (proof.status !== 200 || value.success !== true || value.data?.status !== 'current'
        || bundle?.session?.current !== true || bundle?.session?.sid !== identity.sid
        || bundle?.user?.id !== identity.user || bundle?.token_type !== 'Bearer'
        || typeof bundle.access_token !== 'string') {
        throw new Error('The current browser session could not be verified')
      }
      return bundle.access_token
    }
    const send = bearer => fetch(path, {
      method, mode: 'same-origin', credentials: 'same-origin', redirect: 'error',
      headers: { Authorization: 'Bearer ' + bearer, 'Content-Type': 'application/json',
        ...(typeof identity === 'string' ? {} : { 'X-LMM-Credit-Unit': '500000' }) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    })
    const bearer = await currentBearer()
    let response = await send(bearer)
    let rotation = 'none'
    if (typeof identity !== 'string' && method === 'GET' && response.status === 401) {
      // Reload can rotate the current session after its read-only proof. Retry only
      // after proving a different bearer belongs to the same user and session.
      const current = await currentBearer()
      if (current !== bearer) {
        rotation = 'same-session credential rotated'
        await response.body?.cancel()
        response = await send(current)
      } else {
        rotation = 'same credential rejected'
      }
    }
    return { status: response.status, id: response.headers.get('x-request-id'), json: await response.json(), rotation }
  }, { identity, path, body, method })
  assert.equal(result.status, expectedStatus, path + ' (' + result.rotation + ')')
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
  const { page, identity, user } = await session(browser, 'browseradmin', '/channels')
  assert.equal(user, 8)
  const plugins = await request(page, identity, pluginPath)
  assert.equal(plugins.json.success, true)
  const enabledPlugins = plugins.json.data.filter(plugin => plugin.installed && plugin.enabled).map(plugin => plugin.name)
  for (const name of ['community', 'operations']) assert.ok(enabledPlugins.includes(name), name)
  // The host retains the payment adapter outside the marketplace plugin lifecycle.
  assert.equal(plugins.json.data.some(plugin => plugin.name === 'payments'), false)
  const options = await request(page, identity, optionsPath)
  assert.equal(options.json.success, true)
  const settings = new Map(options.json.data.map(entry => [entry.key, entry.value]))
  assert.equal(settings.get('ServerAddress'), web)
  assert.equal(settings.get('USDExchangeRate'), '7.25')
  const read = async () => (await request(page, identity, config)).json.data
  // Install two fresh enabled groups in this disposable Core via its ordinary CAS.
  const seed = await read()
  assert.ok(!seed.routing.groups.some(g => [group, otherGroup].includes(g.name)))
  const proposal = {
    expected_version: seed.version,
    routing: { ...seed.routing, version: seed.version.routing + 1,
      groups: [...seed.routing.groups, ...[group, otherGroup].map(name => ({ name, enabled: true }))] },
    prices: { ...seed.prices, version: seed.version.pricing + 1 }, bindings: seed.bindings,
  }
  const seedPreview = await request(page, identity, config + '/preview', proposal)
  assert.equal(seedPreview.json.success, true)
  const seeded = await request(page, identity, config,
    { ...proposal, confirmation_digest: seedPreview.json.data.confirmation_digest }, 'PUT')
  assert.equal(seeded.json.success, true)
  await page.reload()
  await page.getByRole('button', { name: 'Model onboarding', exact: true }).waitFor()
  const initial = await read()
  assert.equal(initial.routing.upstreams.length, 1)
  const channelId = initial.routing.upstreams[0].id
  const channel = root + '/channels/' + channelId + '/models'
  const rules = c => c.prices.rules.filter(r => r.model === model)
  const routes = c => c.routing.routes.filter(r => r.model === model)
  const active = c => c.routing.routes.some(r => r.enabled && r.model === model)
  assert.equal(active(initial), false)
  assert.equal(initial.bindings.official_prices?.models?.[model], undefined)
  await page.getByRole('button', { name: 'Model onboarding', exact: true }).click()
  const dialog = page.getByRole('dialog')
  const button = name => dialog.getByRole('button', { name, exact: true })
  const seen = await clickReceipt(page, button('Sync models'), channel + '/synchronize', 'POST')
  assert.ok(seen.data.models.includes(model))
  assert.ok(seen.data.models.includes(sibling))
  assert.equal(seen.data.prices_inferred, false)
  await dialog.getByRole('checkbox', { name: model, exact: true }).check()
  const saved = await clickReceipt(page, button('Save pending models'), channel + '/pending', 'PUT')
  assert.equal(saved.data.routes_published, false)
  const row = dialog.getByRole('row').filter({ has: page.getByRole('checkbox', { name: 'Review model ' + model, exact: true }) })
  await row.getByText('Pending pricing', { exact: true }).waitFor()
  const pending = await read()
  assert.deepEqual(pending.bindings.discoveries[channelId].pending_models, [model])
  assert.deepEqual(pending.routing.routes, initial.routing.routes)
  assert.deepEqual(pending.prices.rules, initial.prices.rules)
  assert.equal(pending.bindings.official_prices?.models?.[model], undefined)
  await row.getByRole('button', { name: 'Pricing', exact: true }).click()
  const price = dialog.getByRole('region', { name: 'Official base price' })
  await price.getByRole('heading', { name: model, exact: true }).waitFor()
  await price.locator('label:has(> span:text-is("Source")) select').selectOption('openrouter')
  assert.equal(await price.locator('label:has(> span:text-is("Channel")) select').inputValue(), String(channelId))
  const fetched = await clickReceipt(page, button('Fetch upstream prices'), channel + '/prices/preview', 'POST')
  const quote = fetched.data.quote
  assert.deepEqual(quote.source, { kind: 'channel', channel_id: channelId, upstream_model: model })
  assert.equal(quote.model, model)
  assert.equal(quote.rates.input, '2')
  assert.equal(quote.rates.output, '4')
  const editor = price.locator('textarea')
  await page.waitForFunction(id => Array.from(document.querySelectorAll('textarea')).some(t => t.value.includes('channel/' + id)), channelId)
  const toml = await editor.inputValue()
  for (const pattern of [new RegExp(String.raw`source\s*=\s*["']channel/${channelId}["']`), /source_unit\s*=\s*["']USD\/token["']/, /^input\s*=\s*2000000$/m, /^output\s*=\s*4000000$/m]) assert.match(toml, pattern)
  const preview = await clickReceipt(page, button('Preview changes'), config + '/preview', 'POST')
  assert.equal(preview.data.saved, false)
  const priced = await clickReceipt(page, button('Publish prices'), config, 'PUT')
  assert.equal(priced.input.confirmation_digest, preview.data.confirmation_digest)
  await row.getByText('Pending activation', { exact: true }).waitFor()
  const publishedPrice = await read()
  assert.equal(active(publishedPrice), false)
  assert.equal(publishedPrice.bindings.official_prices.models[model].rates.input, 2000000)
  assert.deepEqual(rules(publishedPrice), [])
  const enabledGroups = publishedPrice.routing.groups.filter(g => g.enabled).map(g => g.name)
  assert.equal(await row.getByText('Missing group prices:', { exact: false }).count(), 0)
  assert.equal(await row.getByText('Missing official base price:', { exact: false }).count(), 0)
  await row.getByRole('checkbox', { name: 'Review model ' + model, exact: true }).check()
  await button('Review selected routes').click()
  const review = dialog.getByRole('region', { name: 'Review selected routes', exact: true })
  for (const name of enabledGroups) assert.equal(await review.getByRole('checkbox', { name, exact: true }).isChecked(), false)
  await review.getByRole('checkbox', { name: group, exact: true }).check()
  for (const [name, value] of [['Maximum input tokens', '100'], ['Maximum output tokens', '20'], ['Contract revision', 'browser-v1']]) {
    await review.getByLabel(model + ' · ' + name, { exact: true }).fill(value)
  }
  const route = await clickReceipt(page, button('Preview selected routes'), config + '/preview', 'POST')
  assert.equal(route.data.saved, false)
  assert.deepEqual(route.input.expected_version, publishedPrice.version)
  assert.deepEqual(routes(route.data).map(r => r.group), [group])
  assert.deepEqual(rules(route.data).map(r => r.group), [group])
  const beforeConfirm = await read()
  assert.deepEqual(beforeConfirm.version, publishedPrice.version)
  assert.deepEqual(routes(beforeConfirm), [])
  assert.deepEqual(rules(beforeConfirm), [])
  const published = await clickReceipt(page, button('Publish selected routes'), config, 'PUT')
  assert.deepEqual(published.input, { ...route.input, confirmation_digest: route.data.confirmation_digest })
  await row.getByText('Enabled', { exact: true }).waitFor()
  await page.reload()
  await page.getByRole('button', { name: 'Model onboarding', exact: true }).click()
  await row.getByText('Enabled', { exact: true }).waitFor()
  assert.ok(enabledGroups.includes(otherGroup))
  assert.equal(await row.getByText('Missing group prices:', { exact: false }).count(), 0)
  const final = await read()
  assert.deepEqual(routes(final), routes(route.data))
  assert.deepEqual(rules(final), rules(route.data))
  assert.deepEqual(final.bindings.official_prices.models[model], publishedPrice.bindings.official_prices.models[model])
  assert.deepEqual(final.routing.routes.filter(r => r.model !== model), publishedPrice.routing.routes)
  assert.deepEqual(final.prices.rules.filter(r => r.model !== model), publishedPrice.prices.rules)
  const [bound] = routes(final)
  assert.equal(bound.enabled, true)
  assert.equal(bound.targets.length, 1)
  assert.equal(bound.targets[0].upstream_model, model)
  assert.equal(bound.targets[0].upstream_id, channelId)
  await page.context().close()
  return { model, group, other_group: otherGroup, channel_id: channelId, version: final.version,
    shell: { plugins_status: plugins.status, options_status: options.status,
      enabled_plugins: enabledPlugins, display_exchange_rate: settings.get('USDExchangeRate') },
    price_digest: preview.data.confirmation_digest, route_digest: route.data.confirmation_digest }

}

async function stream(page, key) {
  // Consume the full SSE response inside the authenticated browser. Only bounded outcome fields leave it.
  return page.evaluate(async ({ key, model }) => {
    const reply = await fetch('/v1/chat/completions', { method: 'POST', signal: AbortSignal.timeout(30000),
      headers: { Authorization: 'Bearer ' + key, 'Content-Type': 'application/json' },
      body: JSON.stringify({ model, stream: true, stream_options: { include_usage: true },
        messages: [{ role: 'user', content: 'browser proof' }], max_tokens: 20 }) })
    if (!reply.ok || !reply.headers.get('content-type')?.startsWith('text/event-stream')) {
      throw new Error('Unexpected stream response: HTTP ' + reply.status)
    }
    const wire = await reply.text()
    if (wire.length > 65536) throw new Error('Unexpected stream size')
    const frames = wire.replaceAll('\r\n', '\n').split('\n\n').flatMap(block => {
      const data = block.split('\n').filter(line => line.startsWith('data:')).map(line => line.slice(5).trimStart())
      return data.length ? [data.join('\n')] : []
    })
    const terminal = frames.pop()
    if (terminal !== '[DONE]' || frames.includes('[DONE]')) throw new Error('Missing or duplicate SSE terminal')
    const events = frames.map(frame => JSON.parse(frame))
    if (events.some(event => event.error)) throw new Error('Unexpected SSE error event')
    const choices = events.flatMap(event => event.choices ?? [])
    return { id: reply.headers.get('x-request-id'), done: true,
      contentMatches: choices.map(choice => choice.delta?.content ?? '').join('') === 'browser fixture answer',
      stopCount: choices.filter(choice => choice.finish_reason === 'stop').length,
      usage: events.filter(event => event.usage).map(event => [event.usage.prompt_tokens, event.usage.completion_tokens]) }
  }, { key, model })
}

async function settle(browser) {
  const { page, identity, user } = await session(browser, 'browserpayer', '/wallet')
  assert.equal(user, 7)
  const restricted = []
  for (const path of [pluginPath, optionsPath]) {
    const anonymous = await fetch(web + path)
    assert.equal(anonymous.status, 401, path)
    await anonymous.body?.cancel()
    const ordinary = await request(page, identity, path, undefined, 'GET', 403)
    restricted.push({ path, anonymous: anonymous.status, ordinary: ordinary.status })
  }
  const publicNavigation = await fetch(web + '/api/site/navigation')
  assert.equal(publicNavigation.status, 200)
  const navigation = await publicNavigation.json()
  assert.equal(navigation.success, true)
  assert.ok(Number.isSafeInteger(navigation.data.revision) && navigation.data.revision > 0)
  const anonymousRelease = await fetch(web + '/api/release-notes/latest')
  assert.equal(anonymousRelease.status, 401)
  await anonymousRelease.body?.cancel()
  const release = await request(page, identity, '/api/release-notes/latest')
  assert.equal(release.json.success, true)
  assert.equal(release.json.data, null)
  const channelPath = '/api/user/payments/channels'
  const anonymousChannels = await fetch(web + channelPath)
  assert.equal(anonymousChannels.status, 401)
  await anonymousChannels.body?.cancel()
  const checkout = await request(page, identity, channelPath)
  assert.equal(checkout.json.success, true)
  assert.deepEqual(checkout.json.data.channels, [])
  assert.equal(checkout.json.data.available, false)
  assert.equal(checkout.json.data.pancake_portal_available, false)
  const wallet = async () => {
    const { json: { data } } = await request(page, identity, '/api/user/self')
    assert.equal(data.id, user)
    assert.equal(data.financial_projection_available, true)
    assert.equal(data.wallet_reserved_quota, 0)
    return data.wallet_spendable_quota
  }
  const before = await wallet()
  assert.equal(before, 1000)
  await page.locator('#wallet-balance').getByText(/^1,?000\s+Credits$/i).first().waitFor()
  const created = await request(page, identity, '/api/token/', {
    name: 'Browser settlement proof', remain_quota: 500, unlimited_quota: false,
    expired_time: Math.floor(Date.now() / 1000) + 600, one_time_reveal: true,
    group, model_limits_enabled: true, model_limits: model,
    auto_groups: [], cross_group_retry: false, allow_ips: '127.0.0.1/32',
  })
  const key = created.json.data
  assert.ok(key.key.startsWith('lmmk_'))
  const catalog = await request(page, key.key, '/v1/models')
  assert.deepEqual(catalog.json.data.map(record => record.id), [model])
  const paid = await stream(page, key.key)
  assert.equal(paid.contentMatches, true)
  assert.equal(paid.stopCount, 1)
  assert.deepEqual(paid.usage, [[10, 20]])
  assert.ok(paid.id)
  const after = await wallet()
  assert.equal(after, before - 50)
  await page.getByRole('button', { name: 'Refresh account status' }).click()
  await page.locator('#wallet-balance').getByText(/^950\s+Credits$/i).first().waitFor()
  const { json: { data } } = await request(page, identity, '/api/log/self?request_id=' + encodeURIComponent(paid.id))
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
  return {
    request_id: paid.id, user, key_id: key.id, before, after, settled: 50, stream_terminal: paid.done,
    shell: { restricted, navigation_revision: navigation.data.revision, anonymous_release: anonymousRelease.status, signed_in_release: release.status },
    checkout: { anonymous_status: anonymousChannels.status, signed_in_status: checkout.status,
      available: checkout.json.data.available, channels: checkout.json.data.channels.length,
      portal_available: checkout.json.data.pancake_portal_available },
  }
}

await new Promise((ok, fail) => front.once('error', fail).listen(41781, '127.0.0.1', ok))
let browser
try {
  if (scenario.bindingProvider) {
    const proof = await runAccountBrowser(chromium, session, request, web, scenario)
    process.stdout.write(JSON.stringify(proof))
  } else {
    browser = await chromium.launch({ headless: true })
    const terminal = await exerciseTerminal(browser, session, request, { model, sibling })
    const publication = await onboard(browser)
    const payment = await settle(browser)
    terminal.log = await checkTerminalLog(browser,session,payment.request_id)
    const screenshots = await capturePaidLogViews(browser, session, payment, web)
    const pages = await captureConsolePages(browser, session, web, payment, publication)
    process.stdout.write(JSON.stringify({ publication, payment, terminal, screenshots, pages }))
  }
} finally {
  await browser?.close()
  front.closeAllConnections()
  await new Promise(ok => front.close(ok))
}
