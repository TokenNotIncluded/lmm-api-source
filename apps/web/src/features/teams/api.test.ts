/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { AxiosError, getAdapter } from 'axios'

import { api } from '@/lib/api'
import {
  applyAuthBundle,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'

import {
  createTeam,
  issueTeamKey,
  listTeams,
  removeMember,
  respondToInvite,
  TeamFailure,
  updateMember,
  type Invitation,
  type TeamScope,
} from './api'

const adapter = api.defaults.adapter
const refreshAdapter = getAdapter(adapter)
const previousAuth = useAuthStore.getState().auth
const scope: TeamScope = { userId: 501, sessionId: 'synthetic-team-session' }
function login(sid = scope.sessionId, expiresAt = 2000000000) {
  applyAuthBundle(
    {
      access_token: 'synthetic-team-bearer',
      token_type: 'Bearer',
      access_expires_at: expiresAt,
      user: { id: scope.userId, role: 1, username: 'synthetic-member' },
      session: {
        sid,
        current: true,
        login_method: 'password',
        ip: '127.0.0.1',
        user_agent: 'synthetic',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2000000000,
      },
    },
    false
  )
}
afterEach(() => {
  api.defaults.adapter = adapter
  setDevelopmentAuthRefreshAdapter(refreshAdapter)
  useAuthStore.setState({ auth: previousAuth })
})

test('team reads require the current session and reject incompatible or unsafe DTOs', async () => {
  login()
  for (const data of [
    { success: true, data: [] },
    [
      {
        id: Number.MAX_SAFE_INTEGER + 1,
        account_id: 1,
        created_by_user_id: 1,
        version: 1,
        name: 'Team',
        role: 'owner',
        membership_version: 0,
        can_spend: true,
      },
    ],
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      data,
      headers: {},
      status: 200,
      statusText: 'OK',
    })
    await assert.rejects(
      listTeams(scope),
      (error) => error instanceof TeamFailure && error.kind === 'unavailable'
    )
  }
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.url, '/core/v1/teams?after=0')
    assert.deepEqual(config.authScope, scope)
    login('replacement-team-session')
    return { config, data: [], headers: {}, status: 200, statusText: 'OK' }
  }
  await assert.rejects(listTeams(scope), /session_changed/)
  await assert.rejects(listTeams(scope), /session_changed/)
  assert.equal(calls, 1)
})

test('member writes bind the selected membership generation without replay', async () => {
  login()
  const member = {
    user_id: 502,
    role: 'member' as const,
    can_spend: true,
    membership_version: 9,
  }
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(`${config.method} ${config.url}`)
    assert.deepEqual(config.authScope, scope)
    if (config.method === 'patch') {
      assert.deepEqual(JSON.parse(String(config.data)), {
        role: 'admin',
        can_spend: false,
        expected_version: 9,
      })
    }
    throw Object.assign(new Error('stale membership'), {
      response: { status: 409 },
    })
  }
  await assert.rejects(
    updateMember(scope, 101, member, 'admin', false),
    /conflict/
  )
  await assert.rejects(removeMember(scope, 101, member), /conflict/)
  assert.deepEqual(seen, [
    'patch /core/v1/teams/101/members/502',
    'delete /core/v1/teams/101/members/502?version=9',
  ])
})

test('team key issuance charges only the selected team and returns a one-time secret', async () => {
  login()
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/core/v1/keys')
    assert.deepEqual(JSON.parse(String(config.data)), {
      owner: { kind: 'team', id: 101 },
      ttl_seconds: 2592000,
      funding_policy: {
        version: 1,
        account_order: [{ kind: 'team', id: 101 }],
      },
    })
    return {
      config,
      data: { id: 702, secret: 'synthetic-never-live-key' },
      headers: {},
      status: 201,
      statusText: 'Created',
    }
  }
  assert.equal(
    (await issueTeamKey(scope, 101)).secret,
    'synthetic-never-live-key'
  )
})

test('invitation acceptance verifies the returned team, and empty mutations require 204', async () => {
  login()
  const invitation: Invitation = {
    id: 301,
    team_id: 101,
    recipient_user_id: 501,
    role: 'member',
    can_spend: false,
    status: 'pending',
    expires_at: 2000000000,
  }
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return {
      config,
      data: config.url?.endsWith('/accept')
        ? { team_id: 999 }
        : { success: true },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await assert.rejects(respondToInvite(scope, invitation, 'accept'), /unknown/)
  await assert.rejects(respondToInvite(scope, invitation, 'reject'), /unknown/)
  assert.equal(calls, 2)
})

test('lost create responses and late authentication replacement never report success or retry', async () => {
  login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('response lost')
  }
  await assert.rejects(createTeam(scope), /unknown/)
  assert.equal(calls, 1)
  api.defaults.adapter = async (config) => {
    login('replacement-session-after-write')
    return {
      config,
      data: { id: 101 },
      headers: {},
      status: 201,
      statusText: 'Created',
    }
  }
  await assert.rejects(createTeam(scope), /session_changed/)
})

function refreshedBundle() {
  const auth = useAuthStore.getState().auth
  return {
    access_token: 'refreshed-team-bearer',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: auth.user,
    session: auth.session,
  }
}

for (const method of ['read', 'write']) {
  test(`team ${method} refreshes expired credentials before dispatch`, async () => {
    login(scope.sessionId, 1)
    const order: string[] = []
    setDevelopmentAuthRefreshAdapter(async (config) => {
      order.push('refresh')
      assert.equal(config.headers.get('X-Auth-Session'), scope.sessionId)
      return {
        config,
        data: { success: true, data: refreshedBundle() },
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    })
    api.defaults.adapter = async (config) => {
      order.push('request')
      assert.equal(
        config.headers.get('Authorization'),
        'Bearer refreshed-team-bearer'
      )
      assert.deepEqual(config.authScope, scope)
      return {
        config,
        data: method === 'read' ? [] : { id: 101 },
        headers: {},
        status: method === 'read' ? 200 : 201,
        statusText: 'OK',
      }
    }
    if (method === 'read') await listTeams(scope)
    else await createTeam(scope)
    assert.deepEqual(order, ['refresh', 'request'])
  })

  test(`team ${method} handles 401 without replaying mutations`, async () => {
    login()
    let calls = 0
    let refreshes = 0
    setDevelopmentAuthRefreshAdapter(async (config) => {
      refreshes++
      return {
        config,
        data: { success: true, data: refreshedBundle() },
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    })
    api.defaults.adapter = async (config) => {
      calls++
      if (calls === 1) {
        throw new AxiosError('expired', 'ERR_BAD_REQUEST', config, undefined, {
          config,
          data: {},
          headers: {},
          status: 401,
          statusText: 'Unauthorized',
        })
      }
      assert.equal(
        config.headers.get('Authorization'),
        'Bearer refreshed-team-bearer'
      )
      return { config, data: [], headers: {}, status: 200, statusText: 'OK' }
    }
    if (method === 'read') await listTeams(scope)
    else await assert.rejects(createTeam(scope), /forbidden/)
    assert.equal(calls, method === 'read' ? 2 : 1)
    assert.equal(refreshes, 1)
    assert.equal(
      useAuthStore.getState().auth.accessToken,
      'refreshed-team-bearer'
    )
  })
}

test('team mutation cannot follow a replacement session during refresh', async () => {
  login(scope.sessionId, 1)
  const oldBundle = refreshedBundle()
  let calls = 0
  setDevelopmentAuthRefreshAdapter(async (config) => {
    login('replacement-team-session')
    return {
      config,
      data: { success: true, data: oldBundle },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  })
  api.defaults.adapter = async (config) => {
    calls++
    return {
      config,
      data: { id: 101 },
      headers: {},
      status: 201,
      statusText: 'Created',
    }
  }
  await assert.rejects(createTeam(scope), /session_changed/)
  assert.equal(calls, 0)
  assert.equal(
    useAuthStore.getState().auth.session?.sid,
    'replacement-team-session'
  )
})
