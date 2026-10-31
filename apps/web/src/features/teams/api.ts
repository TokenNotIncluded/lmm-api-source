/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api, type ApiRequestConfig } from '@/lib/api'
import { withAuthenticatedCookieWrite } from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'

const id = z.number().int().safe().positive()
const version = id
export const teamSchema = z
  .object({
    id,
    account_id: id,
    created_by_user_id: id,
    version,
    name: z.string(),
    role: z.enum(['owner', 'admin', 'member']),
    membership_version: z.number().int().safe().nonnegative(),
    can_spend: z.boolean(),
  })
  .strict()
export const memberSchema = z
  .object({
    user_id: id,
    role: z.enum(['owner', 'admin', 'member']),
    can_spend: z.boolean(),
    membership_version: z.number().int().safe().nonnegative(),
  })
  .strict()
export const invitationSchema = z
  .object({
    id,
    team_id: id,
    recipient_user_id: id,
    role: z.enum(['admin', 'member']),
    can_spend: z.boolean(),
    status: z.enum([
      'pending',
      'accepted',
      'rejected',
      'withdrawn',
      'expired',
      'stale',
    ]),
    expires_at: z.number().int().safe(),
  })
  .strict()
export const keySchema = z
  .object({
    id,
    kind: z.string(),
    user_id: id,
    owner_account_id: id,
    expires_at: z.number().int().safe(),
    revoked: z.boolean(),
    created_at: z.number().int().safe(),
  })
  .strict()
export type Team = z.infer<typeof teamSchema>
export type Member = z.infer<typeof memberSchema>
export type Invitation = z.infer<typeof invitationSchema>
export type TeamScope = { userId: number; sessionId: string }
export class TeamFailure extends Error {
  constructor(
    readonly kind:
      | 'session_changed'
      | 'forbidden'
      | 'conflict'
      | 'invalid'
      | 'unavailable'
      | 'unknown'
  ) {
    super(kind)
  }
}
export function currentTeamScope(scope: TeamScope) {
  const auth = useAuthStore.getState().auth
  return auth.user?.id === scope.userId && auth.session?.sid === scope.sessionId
}
async function request<T>(
  scope: TeamScope,
  method: 'GET' | 'POST' | 'PATCH' | 'DELETE',
  path: string,
  schema: z.ZodType<T>,
  data?: unknown,
  signal?: AbortSignal,
  status = 200
): Promise<T> {
  if (!currentTeamScope(scope) || signal?.aborted) {
    throw new TeamFailure('session_changed')
  }
  const options: ApiRequestConfig = {
    authScope: scope,
    signal,
    disableDuplicate: true,
    skipBusinessError: true,
    skipErrorHandler: true,
    timeout: 10000,
  }
  try {
    const dispatch = (config: ApiRequestConfig) =>
      api.request<unknown>({ ...options, ...config, url: path, method, data })
    const result =
      method === 'GET'
        ? await dispatch({})
        : await withAuthenticatedCookieWrite(dispatch, false)
    if (!currentTeamScope(scope) || signal?.aborted) {
      throw new TeamFailure('session_changed')
    }
    if (result.status !== status) {
      throw new TeamFailure(method === 'GET' ? 'unavailable' : 'unknown')
    }
    const parsed = schema.safeParse(result.data)
    if (!parsed.success) {
      throw new TeamFailure(method === 'GET' ? 'unavailable' : 'unknown')
    }
    return parsed.data
  } catch (error) {
    if (!currentTeamScope(scope) || signal?.aborted) {
      throw new TeamFailure('session_changed')
    }
    if (error instanceof TeamFailure) throw error
    const response =
      typeof error === 'object' && error !== null && 'response' in error
        ? error.response
        : undefined
    const status =
      typeof response === 'object' && response !== null && 'status' in response
        ? response.status
        : undefined
    throw new TeamFailure(
      status === 401 || status === 403
        ? 'forbidden'
        : status === 409
          ? 'conflict'
          : status === 400
            ? 'invalid'
            : method === 'GET'
              ? 'unavailable'
              : 'unknown'
    )
  }
}
const pathId = (value: number) => id.parse(value)
const cursor = (value: number) =>
  z.number().int().safe().nonnegative().parse(value)
const empty = z.union([z.undefined(), z.literal('')])
export const listTeams = (scope: TeamScope, after = 0, signal?: AbortSignal) =>
  request(
    scope,
    'GET',
    `/core/v1/teams?after=${cursor(after)}`,
    z.array(teamSchema).max(100),
    undefined,
    signal
  )
export const listMembers = (
  scope: TeamScope,
  teamId: number,
  after = 0,
  signal?: AbortSignal
) =>
  request(
    scope,
    'GET',
    `/core/v1/teams/${pathId(teamId)}/members?after=${cursor(after)}`,
    z.array(memberSchema).max(100),
    undefined,
    signal
  )
export const listInvitations = (
  scope: TeamScope,
  teamId?: number,
  after = 0,
  signal?: AbortSignal
) =>
  request(
    scope,
    'GET',
    `${teamId ? `/core/v1/teams/${pathId(teamId)}/invites` : '/core/v1/invites'}?after=${cursor(after)}`,
    z.array(invitationSchema).max(100),
    undefined,
    signal
  )
export const listKeys = (
  scope: TeamScope,
  teamId: number,
  after = 0,
  signal?: AbortSignal
) =>
  request(
    scope,
    'GET',
    `/core/v1/teams/${pathId(teamId)}/keys?after=${cursor(after)}`,
    z.array(keySchema).max(100),
    undefined,
    signal
  )
export const createTeam = (scope: TeamScope) =>
  request(
    scope,
    'POST',
    '/core/v1/teams',
    z.object({ id }).strict(),
    undefined,
    undefined,
    201
  )
export const renameTeam = (scope: TeamScope, team: Team, name: string) =>
  request(
    scope,
    'PATCH',
    `/core/v1/teams/${pathId(team.id)}`,
    empty,
    { name, expected_version: team.version },
    undefined,
    204
  )
export const inviteMember = (
  scope: TeamScope,
  teamId: number,
  recipient: number,
  role: 'admin' | 'member',
  canSpend: boolean
) =>
  request(
    scope,
    'POST',
    `/core/v1/teams/${pathId(teamId)}/invites`,
    z.object({ id, secret: z.string() }).strict(),
    {
      recipient_user_id: pathId(recipient),
      role,
      can_spend: canSpend,
      ttl_seconds: 86400,
    },
    undefined,
    201
  )
export const respondToInvite = (
  scope: TeamScope,
  invitation: Invitation,
  action: 'accept' | 'reject' | 'withdraw'
) =>
  action === 'accept'
    ? request(
        scope,
        'POST',
        `/core/v1/invites/${pathId(invitation.id)}/accept`,
        z.object({ team_id: z.literal(invitation.team_id) }).strict()
      )
    : request(
        scope,
        action === 'withdraw' ? 'DELETE' : 'POST',
        `/core/v1/invites/${pathId(invitation.id)}${action === 'withdraw' ? '' : '/reject'}`,
        empty,
        undefined,
        undefined,
        204
      )
export const updateMember = (
  scope: TeamScope,
  teamId: number,
  member: Member,
  role: 'admin' | 'member',
  canSpend: boolean
) =>
  request(
    scope,
    'PATCH',
    `/core/v1/teams/${pathId(teamId)}/members/${pathId(member.user_id)}`,
    empty,
    { role, can_spend: canSpend, expected_version: member.membership_version },
    undefined,
    204
  )
export const removeMember = (
  scope: TeamScope,
  teamId: number,
  member: Member
) =>
  request(
    scope,
    'DELETE',
    `/core/v1/teams/${pathId(teamId)}/members/${pathId(member.user_id)}?version=${version.parse(member.membership_version)}`,
    empty,
    undefined,
    undefined,
    204
  )
export const transferTeam = (scope: TeamScope, team: Team, userId: number) =>
  request(
    scope,
    'POST',
    `/core/v1/teams/${pathId(team.id)}/owner`,
    empty,
    { new_owner_user_id: pathId(userId), expected_version: team.version },
    undefined,
    204
  )
export const closeTeam = (scope: TeamScope, team: Team) =>
  request(
    scope,
    'DELETE',
    `/core/v1/teams/${pathId(team.id)}?version=${version.parse(team.version)}`,
    empty,
    undefined,
    undefined,
    204
  )
export const issueTeamKey = (scope: TeamScope, teamId: number) =>
  request(
    scope,
    'POST',
    '/core/v1/keys',
    z.object({ id, secret: z.string() }).strict(),
    {
      owner: { kind: 'team', id: pathId(teamId) },
      ttl_seconds: 2592000,
      funding_policy: {
        version: 1,
        account_order: [{ kind: 'team', id: teamId }],
      },
    },
    undefined,
    201
  )
export const revokeTeamKey = (scope: TeamScope, credentialId: number) =>
  request(
    scope,
    'DELETE',
    `/core/v1/credentials/${pathId(credentialId)}`,
    empty,
    undefined,
    undefined,
    204
  )
