// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { promisify } from 'node:util'

const run = promisify(execFile)

export async function runAccountBrowser(chromium, session, request, web, scenario) {
  const authHome = await mkdtemp(join(tmpdir(), 'lmm-account-browser-'))
  let browser
  try {
    const nss = join(authHome, '.pki', 'nssdb')
    await mkdir(nss, { recursive: true, mode: 0o700 })
    await run('certutil', ['-N', '--empty-password', '-d', 'sql:' + nss])
    await run('certutil', ['-A', '-d', 'sql:' + nss, '-n', 'Runner consent provider', '-t', 'C,,',
      '-i', scenario.bindingProvider.certificateFile])
    browser = await chromium.launch({ headless: true, env: {
      ...process.env, HOME: authHome, XDG_CONFIG_HOME: join(authHome, '.config'),
    } })
    return await verifyBrowserAccounts(browser, session, request, web, scenario)
  } finally {
    await browser?.close()
    await rm(authHome, { force: true, recursive: true })
  }
}

async function signedInOwner(browser, session, request, identifier, providerOrigin) {
  const login = await session(browser, identifier, '/profile', {}, providerOrigin)
  const self = await request(login.page, login.identity, '/api/user/self')
  assert.equal(self.json.success, true)
  assert.equal(self.json.data.id, login.user)
  return login
}

async function verifyBrowserAccounts(browser, session, request, web, scenario) {
  const provider = scenario.bindingProvider
  const origin = new URL(provider.authorizationOrigin)
  assert.equal(origin.protocol, 'https:')
  assert.equal(origin.hostname, '127.0.0.2')
  assert.notEqual(origin.hostname, new URL(web).hostname)
  assert.equal(origin.origin, provider.authorizationOrigin)
  const label = 'Linked sign-in accounts'
  const linkPath = '/api/user/account-links'
  const providerKey = 'federated:' + provider.id
  const pathFor = phase => linkPath + '/' + providerKey + '/' + phase
  const owned = row => row.active === true && row.issuer === 'urn:lmm:federated:' + provider.id && row.subject === provider.subject
  let stage = 'username sign-in'
  const navigation = { consent_status: null, callback_observed: false, finish_status: null }
  try {
    const username = await signedInOwner(browser, session, request, 'browserpayer', origin.origin)
    const owner = username.user
    assert.equal(owner, 7)
    const usernameSession = username.identity.sid
    await username.page.context().close()

    stage = 'verified email sign-in'
    const email = await signedInOwner(browser, session, request, scenario.loginEmail, origin.origin)
    assert.equal(email.user, owner)
    assert.notEqual(email.identity.sid, usernameSession)
    const loginEvidence = { username_status: 200, email_status: 200, self_status: 200, user_id: owner, distinct_sessions: true }

    await writeFile(join(dirname(process.env.BROWSER_RECEIPT), 'email-login.json'), JSON.stringify(loginEvidence, null, 2))

    stage = 'binding page and unbound provider'
    const { page } = email
    await page.goto(web + '/profile?tab=account')
    const region = page.getByRole('region', { name: label })
    await region.getByText(provider.name, { exact: true }).waitFor()
    const initial = await request(page, email.identity, linkPath)
    assert.equal(initial.json.success, true)
    const available = initial.json.data.providers.filter(row => row.provider === providerKey && row.can_bind === true)
    assert.equal(available.length, 1)
    assert.equal(initial.json.data.links.filter(owned).length, 0)
    const bind = region.getByRole('button', { name: 'Bind', exact: true })
    assert.equal(await bind.count(), 1)

    const observed = (response, path, method) => {
      const url = new URL(response.url())
      return url.origin === web && decodeURIComponent(url.pathname) === path && response.request().method() === method
    }
    stage = 'authenticated binding start and consent page'
    const [popup, begin] = await Promise.all([
      page.waitForEvent('popup'),
      page.waitForResponse(response => observed(response, pathFor('begin'), 'POST')),
      bind.click(),
    ])
    assert.equal(begin.status(), 200)
    await popup.waitForURL(url => url.origin === origin.origin)
    const consent = popup.getByRole('button', { name: 'Allow account link', exact: true })
    await consent.waitFor()

    stage = 'callback and durable binding confirmation'
    // Observe the production callback navigation in the popup. The account
    // page receives the result and issues its own authenticated finish call.
    const [, , finish] = await Promise.all([
      popup.waitForResponse(response => {
        const url = new URL(response.url())
        return url.origin === origin.origin && url.pathname === '/consent' && response.request().method() === 'POST'
      }).then(response => {
        navigation.consent_status = response.status()
        assert.equal(response.status(), 303)
      }),
      popup.waitForURL(url => url.origin === web && url.pathname === '/oauth/' + provider.slug,
        { waitUntil: 'commit' }).then(() => { navigation.callback_observed = true }),
      page.waitForResponse(response => observed(response, pathFor('finish'), 'POST')).then(response => {
        navigation.finish_status = response.status()
        return response
      }),
      consent.click(),
    ])
    assert.equal(finish.status(), 200)
    assert.equal((await finish.json()).success, true)
    await region.getByText(provider.subject, { exact: true }).waitFor()
    await region.getByRole('button', { name: 'Unbind', exact: true }).waitFor()
    await page.context().close()

    stage = 'fresh password session and persisted linked identity'
    const fresh = await signedInOwner(browser, session, request, 'browserpayer', origin.origin)
    assert.equal(fresh.user, owner)
    assert.notEqual(fresh.identity.sid, email.identity.sid)
    await fresh.page.goto(web + '/profile?tab=account')
    const reopened = fresh.page.getByRole('region', { name: label })
    await reopened.getByText(provider.subject, { exact: true }).waitFor()
    await reopened.getByRole('button', { name: 'Unbind', exact: true }).waitFor()
    const saved = await request(fresh.page, fresh.identity, linkPath)
    assert.equal(saved.json.success, true)
    assert.equal(saved.json.data.links.filter(owned).length, 1)
    await fresh.page.context().close()
    return { email_login: loginEvidence, external_binding: {
      user_id: owner, provider_kind: 'federated', begin_status: 200,
      consent_observed: true, consent_redirect_status: navigation.consent_status,
      callback_observed: navigation.callback_observed, finish_status: navigation.finish_status,
      readback_status: saved.status, fresh_session_readback: true, active_subject_matches: 1,
    } }
  } catch {
    // Never export bearer values, signed redirect queries or Playwright traces.
    throw new Error('Browser account acceptance failed during ' + stage + ' ' + JSON.stringify(navigation))
  }
}
