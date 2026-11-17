/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  AxiosError,
  type AxiosResponse,
  type InternalAxiosRequestConfig,
} from 'axios'

import type { Member, Team } from '@/features/teams/api'
import { validCreditLimit, type Budget } from '@/features/teams/budget-api'
import { useAuthStore } from '@/stores/auth-store'

type ReviewTrace = {
  synthetic: true
  calls: { method: string; path: string; status: number }[]
}
declare global {
  interface Window {
    __LMM_TEAM_PROFILE_REVIEW__?: ReviewTrace
  }
}

const start = Date.UTC(2026, 10, 1) / 1000
const accountBudget = (
  id: string,
  accountId: number | null,
  userId: number | null,
  issuer: number,
  scope: Budget['scope'],
  canEdit: boolean
): Budget => ({
  id,
  scope,
  account_id: accountId,
  user_id: userId,
  issuer_user_id: issuer,
  period: 'month',
  revision: 1,
  limit_credits: '5000000',
  consumed_credits: '1250000',
  reserved_credits: '250000',
  unlimited: false,
  remaining_credits: '3500000',
  window_start: start,
  window_end: start + 2592000,
  can_edit: canEdit,
})

// Only the explicitly enabled loopback console gallery installs this adapter.
// These in-memory records establish layout and interaction evidence, not server authorization.
export function createTeamBudgetReviewFixture() {
  const enabled =
    ['127.0.0.1', 'localhost', '[::1]'].includes(window.location.hostname) &&
    ['console_review', 'team_profile_review'].some(
      (flag) => new URLSearchParams(window.location.search).get(flag) === '1'
    )

  let identity = ''
  let teams: Team[] = []
  let members = new Map<number, Member[]>()
  let budgets = new Map<number | 'self', Budget[]>()
  let nextTeamId = 20
  let nextBudgetId = 1000
  let trace: ReviewTrace = { synthetic: true, calls: [] }

  return (config: InternalAxiosRequestConfig): AxiosResponse | undefined => {
    const location = window.location
    if (!enabled) return undefined
    const url = new URL(config.url ?? '', location.origin)
    if (url.origin !== location.origin || url.username || url.password) {
      return undefined
    }
    const path = url.pathname
    if (
      path !== '/core/v1/budgets' &&
      path !== '/core/v1/invites' &&
      !/^\/core\/v1\/teams(?:\/|$)/.test(path)
    ) {
      return undefined
    }
    const auth = useAuthStore.getState().auth
    if (!auth.user || !auth.session?.sid) return undefined
    const userId = auth.user.id
    const currentIdentity = `${userId}:${auth.session.sid}`
    if (currentIdentity !== identity) {
      identity = currentIdentity
      teams = [
        {
          id: 12,
          account_id: 24,
          created_by_user_id: userId,
          version: 1,
          name: 'Synthetic research team',
          role: 'owner',
          membership_version: 1,
          can_spend: true,
        },
        {
          id: 13,
          account_id: 25,
          created_by_user_id: 9002,
          version: 1,
          name: 'Synthetic observer team',
          role: 'member',
          membership_version: 1,
          can_spend: false,
        },
      ]
      members = new Map([
        [
          12,
          [
            {
              user_id: userId,
              username: auth.user.username ?? 'synthetic-user',
              display_name: 'Synthetic owner',
              role: 'owner',
              can_spend: true,
              membership_version: 1,
            },
            {
              user_id: 4202,
              username: 'synthetic-member',
              display_name: 'Synthetic teammate',
              role: 'member',
              can_spend: true,
              membership_version: 1,
            },
          ],
        ],
        [
          13,
          [
            {
              user_id: 9002,
              username: 'synthetic-manager',
              display_name: 'Synthetic manager',
              role: 'owner',
              can_spend: true,
              membership_version: 1,
            },
            {
              user_id: userId,
              username: auth.user.username ?? 'synthetic-user',
              display_name: 'Synthetic observer',
              role: 'member',
              can_spend: false,
              membership_version: 1,
            },
          ],
        ],
      ])
      budgets = new Map<number | 'self', Budget[]>([
        ['self', [accountBudget('101', null, userId, userId, 'self', true)]],
        [
          12,
          [
            accountBudget('1201', 24, null, userId, 'account', true),
            accountBudget('1202', 24, 4202, userId, 'member', true),
          ],
        ],
        [
          13,
          [
            accountBudget('1301', 25, null, 9002, 'account', false),
            accountBudget('1302', 25, userId, 9002, 'member', false),
          ],
        ],
      ])
      nextTeamId = 20
      nextBudgetId = 1000
      trace = { synthetic: true, calls: [] }
      window.__LMM_TEAM_PROFILE_REVIEW__ = trace
    }
    const method = (config.method ?? 'get').toUpperCase()
    const response = (data: unknown, status = 200): AxiosResponse => {
      trace.calls.push({ method, path, status })
      return {
        data: structuredClone(data),
        status,
        statusText: 'Synthetic team review',
        headers: {},
        config,
      }
    }
    const fail = (status: number): never => {
      const result = response(
        { code: 'SYNTHETIC_TEAM_REVIEW_REJECTED' },
        status
      )
      throw new AxiosError(
        'Synthetic team review rejected this operation',
        undefined,
        config,
        undefined,
        result
      )
    }
    const body: Record<string, unknown> =
      typeof config.data === 'string'
        ? JSON.parse(config.data)
        : (config.data ?? {})

    if (path === '/core/v1/teams') {
      if (method === 'GET') {
        const after = Number(url.searchParams.get('after') ?? 0)
        return response(teams.filter((team) => team.id > after).slice(0, 100))
      }
      if (method === 'POST') {
        if (
          typeof body.name !== 'string' ||
          !body.name.trim() ||
          body.name.length > 100
        ) {
          return fail(400)
        }
        const id = nextTeamId++
        teams.push({
          id,
          account_id: id + 100,
          created_by_user_id: userId,
          version: 1,
          name: body.name.trim(),
          role: 'owner',
          membership_version: 1,
          can_spend: true,
        })
        members.set(id, [
          {
            user_id: userId,
            username: auth.user.username ?? 'synthetic-user',
            role: 'owner',
            can_spend: true,
            membership_version: 1,
          },
        ])
        budgets.set(id, [])
        return response({ id }, 201)
      }
      return fail(405)
    }
    if (path === '/core/v1/invites') {
      return method === 'GET' ? response([]) : fail(405)
    }
    const match =
      /^\/core\/v1\/teams\/(\d+)\/(members|invites|keys|budgets)$/.exec(path)
    const team = match
      ? teams.find((item) => item.id === Number(match[1]))
      : undefined
    if (match && !team) return fail(404)
    if (match && team && match[2] !== 'budgets') {
      if (method !== 'GET') return fail(405)
      if (match[2] === 'members') return response(members.get(team.id) ?? [])
      return response([])
    }
    if (path !== '/core/v1/budgets' && !match) return fail(404)
    const key = team?.id ?? 'self'
    const current = budgets.get(key) ?? []
    if (method === 'GET') return response({ version: 1, items: current })
    if (method !== 'PUT') return fail(405)
    if (team?.role === 'member') return fail(403)
    const scope = team ? body.scope : 'self'
    const target =
      scope === 'self' ? userId : scope === 'member' ? body.user_id : null
    if (
      !['self', 'account', 'member'].includes(String(scope)) ||
      !['day', 'month'].includes(String(body.period)) ||
      typeof body.limit_credits !== 'string' ||
      !validCreditLimit(body.limit_credits) ||
      (scope === 'member' &&
        (!team ||
          !members.get(team.id)?.some((member) => member.user_id === target)))
    ) {
      return fail(400)
    }
    const existing = current.find(
      (item) =>
        item.scope === scope &&
        item.user_id === target &&
        item.period === body.period &&
        item.issuer_user_id === userId
    )
    if ((existing?.revision ?? null) !== body.expected_revision) {
      return fail(409)
    }
    const budget: Budget = {
      ...(existing ??
        accountBudget(
          String(nextBudgetId++),
          team?.account_id ?? null,
          target as number | null,
          userId,
          scope as Budget['scope'],
          true
        )),
      unlimited: false,
      remaining_credits: '0',
      period: body.period as Budget['period'],
      limit_credits: body.limit_credits,
      revision: (existing?.revision ?? 0) + 1,
      consumed_credits: existing?.consumed_credits ?? '0',
      reserved_credits: existing?.reserved_credits ?? '0',
    }
    const remaining =
      BigInt(budget.limit_credits) -
      BigInt(budget.consumed_credits) -
      BigInt(budget.reserved_credits)
    budget.remaining_credits = String(remaining > 0n ? remaining : 0n)
    budget.window_end =
      budget.window_start + (budget.period === 'day' ? 86400 : 2592000)
    budgets.set(
      key,
      existing
        ? current.map((item) => (item.id === budget.id ? budget : item))
        : [...current, budget]
    )
    return response(budget)
  }
}
