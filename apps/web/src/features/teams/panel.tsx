/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useAuthStore } from '@/stores/auth-store'

import * as teams from './api'

const selectClass =
  'border-input bg-background min-h-11 rounded-md border px-3 py-2 text-sm'
type Pending = { label: string; detail: string; run: () => Promise<unknown> }

export function TeamsPanel() {
  const auth = useAuthStore((state) => state.auth)
  if (!auth.user || !auth.session?.sid) return null
  const scope = { userId: auth.user.id, sessionId: auth.session.sid }
  return <Workspace key={`${scope.userId}:${scope.sessionId}`} scope={scope} />
}

function Workspace({ scope }: { scope: teams.TeamScope }) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const prefix = ['native-teams', scope.userId, scope.sessionId] as const
  const [teamId, setTeamId] = useState<number>()
  const [after, setAfter] = useState(0)
  const [inboxAfter, setInboxAfter] = useState(0)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const [pending, setPending] = useState<Pending>()
  const write = useRef(false)
  const list = useQuery({
    queryKey: [...prefix, 'teams', after],
    queryFn: ({ signal }) => teams.listTeams(scope, after, signal),
    retry: false,
  })
  const inbox = useQuery({
    queryKey: [...prefix, 'invitations', inboxAfter],
    queryFn: ({ signal }) =>
      teams.listInvitations(scope, undefined, inboxAfter, signal),
    retry: false,
  })
  const selected =
    list.data?.find((team) => team.id === teamId) ?? list.data?.[0]

  async function run(action: () => Promise<unknown>) {
    if (write.current || !teams.currentTeamScope(scope)) return
    write.current = true
    setBusy(true)
    setNotice('')
    try {
      await action()
      if (!teams.currentTeamScope(scope)) return
      setPending(undefined)
      toast.success(t('Update succeeded'))
      await client.invalidateQueries({ queryKey: prefix })
    } catch (error) {
      if (!teams.currentTeamScope(scope)) return
      const kind = error instanceof teams.TeamFailure ? error.kind : 'unknown'
      setPending(undefined)
      setNotice(
        t(
          kind === 'unknown'
            ? 'Save result is unknown. Check the original operation before continuing.'
            : kind === 'conflict'
              ? 'Reload the page to try again.'
              : 'Request failed'
        )
      )
      await client.invalidateQueries({ queryKey: prefix })
    } finally {
      write.current = false
      setBusy(false)
    }
  }
  return (
    <section className='space-y-8' aria-label={t('Teams')}>
      <header className='flex flex-wrap items-center justify-between gap-3'>
        <h2 className='text-base font-semibold'>{t('My teams')}</h2>
        <Button
          disabled={busy || list.isPending}
          onClick={() =>
            void run(async () => {
              const created = await teams.createTeam(scope)
              setTeamId(created.id)
              setAfter(0)
            })
          }
        >
          {t('Create team')}
        </Button>
      </header>
      {notice && (
        <Alert role='alert'>
          <AlertDescription>{notice}</AlertDescription>
        </Alert>
      )}
      {list.isPending ? (
        <p role='status'>{t('Loading')}</p>
      ) : list.isError ? (
        <Alert role='alert'>
          <AlertDescription>{t('Request failed')}</AlertDescription>
        </Alert>
      ) : (
        <>
          <div className='flex flex-wrap items-center gap-3'>
            <Label htmlFor='selected-team'>{t('Team')}</Label>
            <select
              id='selected-team'
              className={selectClass}
              value={selected?.id ?? ''}
              onChange={(event) => setTeamId(Number(event.target.value))}
            >
              {!list.data?.length && <option value=''>{t('No data')}</option>}
              {list.data?.map((team) => (
                <option key={team.id} value={team.id}>
                  {team.name} · {team.id}
                </option>
              ))}
            </select>
            <Pager
              after={after}
              last={list.data?.at(-1)?.id}
              full={list.data?.length === 100}
              change={setAfter}
            />
          </div>
          {selected && (
            <TeamDetail
              key={`${selected.id}:${selected.version}:${selected.membership_version}`}
              scope={scope}
              team={selected}
              busy={busy}
              run={run}
              confirm={setPending}
              prefix={prefix}
            />
          )}
        </>
      )}
      <section className='space-y-3 border-t pt-6'>
        <h3 className='font-medium'>{t('Invitations')}</h3>
        {inbox.isPending ? (
          <p role='status'>{t('Loading')}</p>
        ) : inbox.isError ? (
          <p role='alert'>{t('Request failed')}</p>
        ) : (
          <>
            <InviteRows
              items={inbox.data ?? []}
              busy={busy}
              onAction={(invite, action) =>
                void run(() => teams.respondToInvite(scope, invite, action))
              }
            />
            <Pager
              after={inboxAfter}
              last={inbox.data?.at(-1)?.id}
              full={inbox.data?.length === 100}
              change={setInboxAfter}
            />
          </>
        )}
      </section>
      <Dialog
        open={Boolean(pending)}
        onOpenChange={(open) => {
          if (!open && !busy) setPending(undefined)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{pending?.label}</DialogTitle>
            <DialogDescription>{pending?.detail}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant='outline'
              disabled={busy}
              onClick={() => setPending(undefined)}
            >
              {t('Cancel')}
            </Button>
            <Button
              disabled={busy || !pending}
              onClick={() => pending && void run(pending.run)}
            >
              {t('Confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}

function Pager({
  after,
  last,
  full,
  change,
}: {
  after: number
  last?: number
  full: boolean
  change: (value: number) => void
}) {
  const { t } = useTranslation()
  if (!after && !full) return null
  return (
    <div className='flex gap-2'>
      <Button
        size='sm'
        variant='outline'
        disabled={!after}
        onClick={() => change(0)}
      >
        {t('First page')}
      </Button>
      <Button
        size='sm'
        variant='outline'
        disabled={!full || !last}
        onClick={() => last && change(last)}
      >
        {t('Next page')}
      </Button>
    </div>
  )
}

function InviteRows({
  items,
  busy,
  outgoing = false,
  onAction,
}: {
  items: teams.Invitation[]
  busy: boolean
  outgoing?: boolean
  onAction: (
    invite: teams.Invitation,
    action: 'accept' | 'reject' | 'withdraw'
  ) => void
}) {
  const { t } = useTranslation()
  const statuses = {
    pending: t('Pending'),
    accepted: t('Accepted'),
    rejected: t('Rejected'),
    withdrawn: t('Withdrawn'),
    expired: t('Expired'),
    stale: t('Unavailable'),
  }
  if (!items.length) {
    return <p className='text-muted-foreground text-sm'>{t('No data')}</p>
  }
  return (
    <div className='divide-y'>
      {items.map((invite) => (
        <div
          key={invite.id}
          className='flex flex-wrap items-center justify-between gap-3 py-3 text-sm'
        >
          <div className='space-y-1'>
            <p>
              {t('Team')} {invite.team_id} · {t('User ID')}{' '}
              {invite.recipient_user_id}
            </p>
            <p className='text-muted-foreground'>
              {t(invite.role === 'admin' ? 'Administrator' : 'Member')} ·{' '}
              {statuses[invite.status]} ·{' '}
              {new Date(invite.expires_at * 1000).toLocaleString()}
            </p>
          </div>
          {invite.status === 'pending' && (
            <div className='flex gap-2'>
              {outgoing ? (
                <Button
                  size='sm'
                  variant='outline'
                  disabled={busy}
                  onClick={() => onAction(invite, 'withdraw')}
                >
                  {t('Withdraw')}
                </Button>
              ) : (
                <>
                  <Button
                    size='sm'
                    disabled={busy}
                    onClick={() => onAction(invite, 'accept')}
                  >
                    {t('Accept')}
                  </Button>
                  <Button
                    size='sm'
                    variant='outline'
                    disabled={busy}
                    onClick={() => onAction(invite, 'reject')}
                  >
                    {t('Reject')}
                  </Button>
                </>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  )
}

function TeamDetail({
  scope,
  team,
  busy,
  run,
  confirm,
  prefix,
}: {
  scope: teams.TeamScope
  team: teams.Team
  busy: boolean
  run: (action: () => Promise<unknown>) => Promise<void>
  confirm: (value: Pending) => void
  prefix: readonly ['native-teams', number, string]
}) {
  const { t } = useTranslation()
  const [name, setName] = useState(team.name)
  const [recipient, setRecipient] = useState('')
  const [role, setRole] = useState<'admin' | 'member'>('member')
  const [canSpend, setCanSpend] = useState(false)
  const [memberAfter, setMemberAfter] = useState(0)
  const [inviteAfter, setInviteAfter] = useState(0)
  const [keyAfter, setKeyAfter] = useState(0)
  const [secret, setSecret] = useState('')
  const manager = team.role !== 'member'
  const members = useQuery({
    queryKey: [...prefix, team.id, 'members', memberAfter],
    queryFn: ({ signal }) =>
      teams.listMembers(scope, team.id, memberAfter, signal),
    retry: false,
  })
  const invitations = useQuery({
    queryKey: [...prefix, team.id, 'invites', inviteAfter],
    queryFn: ({ signal }) =>
      teams.listInvitations(scope, team.id, inviteAfter, signal),
    enabled: manager,
    retry: false,
  })
  const keys = useQuery({
    queryKey: [...prefix, team.id, 'keys', keyAfter],
    queryFn: ({ signal }) => teams.listKeys(scope, team.id, keyAfter, signal),
    retry: false,
  })
  const recipientId = Number(recipient)
  function invite(event: FormEvent) {
    event.preventDefault()
    if (!Number.isSafeInteger(recipientId) || recipientId <= 0) return
    void run(async () => {
      await teams.inviteMember(scope, team.id, recipientId, role, canSpend)
      setRecipient('')
    })
  }
  return (
    <div className='space-y-7'>
      <div className='flex flex-wrap items-center gap-3'>
        <Badge variant='secondary'>
          {t(
            team.role === 'owner'
              ? 'Owner'
              : team.role === 'admin'
                ? 'Administrator'
                : 'Member'
          )}
        </Badge>
        {team.can_spend && (
          <Badge variant='outline'>{t('Allow spending')}</Badge>
        )}
      </div>
      {manager && (
        <form
          className='flex flex-wrap items-end gap-3'
          onSubmit={(event) => {
            event.preventDefault()
            void run(() => teams.renameTeam(scope, team, name))
          }}
        >
          <div className='min-w-48 flex-1 space-y-2'>
            <Label htmlFor='team-name'>{t('Team name')}</Label>
            <Input
              id='team-name'
              maxLength={100}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </div>
          <Button disabled={busy || !name.trim() || name.trim() === team.name}>
            {t('Save')}
          </Button>
        </form>
      )}
      <section className='space-y-3'>
        <h3 className='font-medium'>{t('Members')}</h3>
        {members.isPending ? (
          <p role='status'>{t('Loading')}</p>
        ) : members.isError ? (
          <p role='alert'>{t('Request failed')}</p>
        ) : (
          <>
            <div className='divide-y'>
              {members.data?.map((member) => (
                <MemberRow
                  key={`${member.user_id}:${member.membership_version}`}
                  member={member}
                  scope={scope}
                  team={team}
                  busy={busy}
                  run={run}
                  confirm={confirm}
                />
              ))}
            </div>
            <Pager
              after={memberAfter}
              last={members.data?.at(-1)?.user_id}
              full={members.data?.length === 100}
              change={setMemberAfter}
            />
          </>
        )}
      </section>
      {manager && (
        <section className='space-y-4 border-t pt-6'>
          <h3 className='font-medium'>{t('Invite member')}</h3>
          <form onSubmit={invite} className='flex flex-wrap items-end gap-3'>
            <div className='space-y-2'>
              <Label htmlFor='invite-user'>{t('User ID')}</Label>
              <Input
                id='invite-user'
                inputMode='numeric'
                pattern='[1-9][0-9]*'
                required
                value={recipient}
                onChange={(event) => setRecipient(event.target.value)}
              />
            </div>
            <div className='space-y-2'>
              <Label htmlFor='invite-role'>{t('Role')}</Label>
              <select
                id='invite-role'
                className={selectClass}
                value={role}
                onChange={(event) =>
                  setRole(event.target.value as 'admin' | 'member')
                }
              >
                <option value='member'>{t('Member')}</option>
                {team.role === 'owner' && (
                  <option value='admin'>{t('Administrator')}</option>
                )}
              </select>
            </div>
            <Label className='flex min-h-11 items-center gap-2'>
              <Checkbox checked={canSpend} onCheckedChange={setCanSpend} />
              {t('Allow spending')}
            </Label>
            <Button
              disabled={
                busy || !Number.isSafeInteger(recipientId) || recipientId <= 0
              }
            >
              {t('Invite member')}
            </Button>
          </form>
          {invitations.isError ? (
            <p role='alert'>{t('Request failed')}</p>
          ) : (
            <InviteRows
              items={invitations.data ?? []}
              outgoing
              busy={busy}
              onAction={(invite, action) =>
                void run(() => teams.respondToInvite(scope, invite, action))
              }
            />
          )}
          <Pager
            after={inviteAfter}
            last={invitations.data?.at(-1)?.id}
            full={invitations.data?.length === 100}
            change={setInviteAfter}
          />
        </section>
      )}
      <section className='space-y-3 border-t pt-6'>
        <div className='flex flex-wrap items-center justify-between gap-3'>
          <h3 className='font-medium'>{t('API keys')}</h3>
          {team.can_spend && (
            <Button
              size='sm'
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  const key = await teams.issueTeamKey(scope, team.id)
                  if (teams.currentTeamScope(scope)) setSecret(key.secret)
                })
              }
            >
              {t('Create API key')}
            </Button>
          )}
        </div>
        {secret && (
          <Alert>
            <AlertDescription>
              <p>
                {t(
                  'Copy this key now. It is shown only once and expires after 30 days.'
                )}
              </p>
              <code className='block py-3 break-all'>{secret}</code>
              <Button variant='outline' size='sm' onClick={() => setSecret('')}>
                {t('Close')}
              </Button>
            </AlertDescription>
          </Alert>
        )}
        {keys.isError ? (
          <p role='alert'>{t('Request failed')}</p>
        ) : (
          <div className='divide-y'>
            {keys.data?.map((key) => (
              <div
                key={key.id}
                className='flex flex-wrap items-center justify-between gap-3 py-3 text-sm'
              >
                <span>
                  #{key.id} · {t('User ID')} {key.user_id} ·{' '}
                  {new Date(key.expires_at * 1000).toLocaleString()}
                </span>
                {key.revoked ? (
                  <Badge variant='outline'>{t('Revoked')}</Badge>
                ) : (
                  <Button
                    size='sm'
                    variant='outline'
                    disabled={busy}
                    onClick={() =>
                      confirm({
                        label: t('Revoke'),
                        detail: `${t('API keys')} #${key.id}`,
                        run: () => teams.revokeTeamKey(scope, key.id),
                      })
                    }
                  >
                    {t('Revoke')}
                  </Button>
                )}
              </div>
            ))}
          </div>
        )}
        <Pager
          after={keyAfter}
          last={keys.data?.at(-1)?.id}
          full={keys.data?.length === 100}
          change={setKeyAfter}
        />
      </section>
      {team.role === 'owner' && (
        <Button
          variant='outline'
          disabled={busy}
          onClick={() =>
            confirm({
              label: t('Close team'),
              detail: t(
                'Closing this team immediately revokes all team access. Its records are retained.'
              ),
              run: () => teams.closeTeam(scope, team),
            })
          }
        >
          {t('Close team')}
        </Button>
      )}
    </div>
  )
}

function MemberRow({
  member,
  scope,
  team,
  busy,
  run,
  confirm,
}: {
  member: teams.Member
  scope: teams.TeamScope
  team: teams.Team
  busy: boolean
  run: (action: () => Promise<unknown>) => Promise<void>
  confirm: (value: Pending) => void
}) {
  const { t } = useTranslation()
  const [role, setRole] = useState<'admin' | 'member'>(
    member.role === 'admin' ? 'admin' : 'member'
  )
  const [spending, setSpending] = useState(member.can_spend)
  const self = member.user_id === scope.userId
  const editable =
    !self &&
    member.role !== 'owner' &&
    (team.role === 'owner' ||
      (team.role === 'admin' && member.role === 'member'))
  return (
    <div className='flex flex-wrap items-center justify-between gap-3 py-4 text-sm'>
      <span className='font-mono'>
        {t('User ID')} {member.user_id}
      </span>
      {editable ? (
        <div className='flex flex-wrap items-center gap-3'>
          <select
            aria-label={`${t('Role')} ${member.user_id}`}
            className={selectClass}
            value={role}
            disabled={busy}
            onChange={(event) =>
              setRole(event.target.value as 'admin' | 'member')
            }
          >
            <option value='member'>{t('Member')}</option>
            {team.role === 'owner' && (
              <option value='admin'>{t('Administrator')}</option>
            )}
          </select>
          <Label className='flex min-h-11 items-center gap-2'>
            <Checkbox
              checked={spending}
              disabled={busy}
              onCheckedChange={setSpending}
            />
            {t('Allow spending')}
          </Label>
          <Button
            size='sm'
            disabled={
              busy || (role === member.role && spending === member.can_spend)
            }
            onClick={() =>
              void run(() =>
                teams.updateMember(scope, team.id, member, role, spending)
              )
            }
          >
            {t('Save')}
          </Button>
        </div>
      ) : (
        <span>
          {t(
            member.role === 'owner'
              ? 'Owner'
              : member.role === 'admin'
                ? 'Administrator'
                : 'Member'
          )}
        </span>
      )}
      <div className='flex flex-wrap gap-2'>
        {team.role === 'owner' && !self && (
          <Button
            size='sm'
            variant='outline'
            disabled={busy}
            onClick={() =>
              confirm({
                label: t('Transfer ownership'),
                detail: `${team.name} → ${member.user_id}`,
                run: () => teams.transferTeam(scope, team, member.user_id),
              })
            }
          >
            {t('Transfer ownership')}
          </Button>
        )}
        {(editable || (self && member.role !== 'owner')) && (
          <Button
            size='sm'
            variant='outline'
            disabled={busy}
            onClick={() =>
              confirm({
                label: t(self ? 'Leave team' : 'Remove member'),
                detail: `${team.name} · ${member.user_id}`,
                run: () => teams.removeMember(scope, team.id, member),
              })
            }
          >
            {t(self ? 'Leave team' : 'Remove')}
          </Button>
        )}
      </div>
    </div>
  )
}
