// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from 'node:assert/strict'

const config = '/core/v1/admin/routing/configuration'

async function run(page, name, args) {
  await page
    .getByLabel('Script', { exact: true })
    .fill(name + ' ' + JSON.stringify(args))
  await page.getByRole('button', { name: 'Run', exact: true }).click()
  const log = page.getByRole('log')
  await log
    .locator('code')
    .filter({ hasText: new RegExp(name + ' · (?!running)') })
    .waitFor()
  const output = await log.locator('pre').innerText()
  // Only decoded allowlisted fields enter the workflow artifact.
  return JSON.parse(output)
}

export async function exerciseTerminal(browser, session, request, { sibling: target, model: incomplete }) {
  const { page, token } = await session(
    browser,
    'browseradmin',
    '/operations/terminal'
  )
  const read = async () => (await request(page, token, config)).json.data
  const initial = await read()
  assert.equal(initial.routing.upstreams.length, 1)
  const channelId = initial.routing.upstreams[0].id
  const existing = initial.prices.rules[0]?.model
  assert.ok(existing && existing !== target && existing !== incomplete)
  // Stub the public price feed alone. Every account and Core route uses the
  // regular browser transport, and the supplier catalog comes from its TCP fixture.
  await page.route('https://models.dev/api.json', (route) =>
    route.fulfill({
      contentType: 'application/json',
      headers: { 'access-control-allow-origin': 'http://127.0.0.1:41781' },
      body: JSON.stringify({
        fixture: {
          models: {
            [target]: {
              cost: { input: 2, output: 4, cache_read: 0, cache_write: 0 },
            },
            [incomplete]: { cost: { input: 1, output: 3 } },
          },
        },
      }),
    })
  )
  const sync = async () => {
    const preview = await run(page, 'catalog.preview', { channels: [channelId, channelId + 1] })
    assert.equal(preview.rows[1].status, 'blocked')
    assert.ok(preview.rows[0].missing_price.includes(target))
    const saved = await run(page, 'catalog.confirm', {
      preview: preview.preview,
      channels: [channelId],
    })
    assert.equal(saved.status, 'published')
    return saved
  }
  const synced = await sync()
  const before = await read()
  assert.deepEqual(synced.version, before.version)
  assert.ok(before.bindings.discoveries[channelId].models.includes(target))
  const proposal = await run(page, 'prices.preview', {
    provider: 'fixture',
    models: [target, incomplete, existing],
    request_rate: 0,
  })
  assert.deepEqual(
    proposal.rows.map((row) => row.status),
    ['ready', 'blocked', 'preserved']
  )
  assert.match(proposal.rows[1].reason, /cache_read/)
  assert.match(proposal.rows[1].reason, /cache_write/)
  assert.equal(proposal.rows[0].details.pricing_scope, 'official_base_only')
  assert.equal(proposal.rows[0].details.base.input, '2')
  assert.deepEqual((await read()).version, before.version)
  // Make the saved proposal stale using the normal confirmed catalog operation.
  await sync()
  const stale = await run(page, 'changes.confirm', {
    preview: proposal.preview,
    items: [target],
  })
  assert.equal(stale.status, 'conflict')
  assert.ok(stale.replacement.preview)
  assert.equal(
    (await read()).bindings.official_prices?.models?.[target],
    undefined
  )

  const payer = await session(browser, 'browserpayer', '/wallet')
  const denied = await payer.page.evaluate(
    async ({ token, path }) => {
      const response = await fetch(path, {
        headers: { Authorization: 'Bearer ' + token },
      })
      return response.status
    },
    { token: payer.token, path: config }
  )
  assert.equal(denied, 403)
  await payer.page.context().close()

  const saved = await run(page, 'changes.confirm', {
    preview: stale.replacement.preview,
    items: [target],
  })
  assert.equal(saved.status, 'published')
  const after = await read()
  assert.deepEqual(saved.version, after.version)
  assert.deepEqual(saved.readback, [
    { id: target, scope: 'official_base_only' },
  ])
  assert.equal(
    after.bindings.official_prices.models[target].rates.input,
    2_000_000
  )
  assert.equal(
    after.bindings.official_prices.models[target].source,
    'models.dev/fixture'
  )
  assert.equal(after.bindings.official_prices.models[incomplete], undefined)
  assert.deepEqual(after.routing.routes, before.routing.routes)
  assert.deepEqual(after.prices.rules, before.prices.rules)
  await page.context().close()
  return {
    sync_version: synced.version,
    expected_version: proposal.expected_version,
    price_version: saved.version,
    confirmation_digest: saved.confirmed_digest,
    price_model: target,
    scope: saved.readback[0].scope,
    rejected_model: incomplete,
    missing_fields: proposal.rows[1].reason,
    preserved: existing,
    denied_status: denied,
    conflict: stale.status,
    real_provider_calls: 0,
  }
}

export async function checkTerminalLog(browser, session, requestId) {
  const { page } = await session(
    browser,
    'browseradmin',
    '/operations/terminal'
  )
  const result = await run(page, 'logs.query', {
    request_id: requestId,
    settlement_status: 'settled',
  })
  assert.equal(result.items.length, 1)
  assert.equal(result.items[0].request_id, requestId)
  assert.equal(result.items[0].settlement_status, 'settled')
  await page.context().close()
  return { request_id: requestId, settlement_status: 'settled' }
}
