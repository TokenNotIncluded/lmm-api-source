/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  CircleCheck,
  CircleOff,
  ShieldCheck,
  UserRound,
  Wallet,
} from 'lucide-react'
import { useContext, useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { toIntlLocale } from '@/i18n/languages'

import {
  isCurrentNativeUsersScope,
  NativeUsersAccessBoundary,
  NativeUsersFailure,
  type NativeUsersScope,
} from './access'
import { NativeBalanceAdjustment } from './adjustments-editor'
import { readNativeUserGroups, readNativeUsers } from './api'
import {
  DEFAULT_NATIVE_USERS_QUERY,
  nativeUsersQuery,
  type NativeGroupsResponse,
  type NativeUser,
  type NativeUsersQuery,
  type NativeUsersResponse,
} from './contract'
import { NativeCreateUser } from './create-editor'
import { NativeDisplayNameEditor } from './profile-editor'
import { nativeQueryFromSearch } from './search'
import { NativeUserSecurity } from './security-editor'

type Props = { scope: NativeUsersScope; recheck: () => void; search: string }
type Load<T> =
  | { status: 'loading' }
  | { status: 'ready'; data: T }
  | { status: 'error'; kind: NativeUsersFailure['kind'] }
type SelectProps = {
  id: string
  label: string
  value: string
  options: { value: string; label: string }[]
  onChange: (value: string) => void
}

function FieldSelect({ id, label, value, options, onChange }: SelectProps) {
  return (
    <div className='space-y-2'>
      <Label htmlFor={id}>{label}</Label>
      <select
        id={id}
        className='border-input bg-background focus-visible:ring-ring min-h-11 w-full rounded-md border px-3 py-2 text-sm focus-visible:ring-2 sm:min-h-9'
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </div>
  )
}

function UserRecord({
  user,
  scope,
  onUpdated,
  onAccessDenied,
}: {
  user: NativeUser
  scope: NativeUsersScope
  onUpdated: () => void
  onAccessDenied: () => void
}) {
  const { t, i18n } = useTranslation()
  const profile = (value: string | null) =>
    value === null ? t('Unavailable') : value === '' ? t('Empty value') : value
  const time = (value: number | null) =>
    value === null
      ? t('Unavailable')
      : new Intl.DateTimeFormat(toIntlLocale(i18n.language), {
          dateStyle: 'medium',
          timeStyle: 'short',
        }).format(value)
  const role = {
    user: t('User'),
    admin: t('Administrator'),
    superadmin: t('Super administrator'),
  }[user.platform_role]
  const wallet = user.wallet
  return (
    <article
      data-native-user-id={user.id}
      className='native-user-record rounded-lg border p-4'
    >
      <div className='grid items-center gap-3 md:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)]'>
        <div className='min-w-0'>
          <h3 className='font-semibold break-all'>{profile(user.username)}</h3>
          <p className='text-muted-foreground text-sm break-all'>
            {profile(user.email)}
          </p>
          <p className='text-muted-foreground text-xs break-all'>
            {t('User ID')}: {user.id}
          </p>
        </div>
        <div className='flex flex-wrap gap-2'>
          <Badge variant={user.active ? 'success' : 'destructive'}>
            {user.active ? (
              <CircleCheck aria-hidden='true' />
            ) : (
              <CircleOff aria-hidden='true' />
            )}
            {user.active ? t('Active user') : t('Disabled user')}
          </Badge>
          <Badge variant={user.platform_role === 'user' ? 'info' : 'tertiary'}>
            {role}
          </Badge>
          <Badge variant='outline'>
            {t('Service level')}: {user.service_level}
          </Badge>
        </div>
        <div className='text-sm'>
          <p className='text-muted-foreground'>{t('Spendable wallet')}</p>
          <p className='font-mono font-semibold break-all'>
            {wallet.status === 'available'
              ? wallet.wallet_units
              : t('Unavailable')}
          </p>
        </div>
      </div>
      <div className='mt-3 flex flex-wrap items-center gap-2'>
        {user.platform_role === 'user' && (
          <div className='flex flex-wrap gap-2'>
            <NativeDisplayNameEditor
              scope={scope}
              userId={user.id}
              onUpdated={onUpdated}
              onAccessDenied={onAccessDenied}
            />
            <NativeUserSecurity
              scope={scope}
              userId={user.id}
              onUpdated={onUpdated}
              onAccessDenied={onAccessDenied}
            />
          </div>
        )}
        {user.platform_role === 'user' && wallet.status === 'available' && (
          <NativeBalanceAdjustment
            scope={scope}
            userId={user.id}
            onUpdated={onUpdated}
            onAccessDenied={onAccessDenied}
          />
        )}
      </div>
      <details className='mt-3 border-t pt-3'>
        <summary className='text-muted-foreground focus-visible:outline-ring cursor-pointer text-sm font-medium focus-visible:outline-2'>
          {t('User details')}
        </summary>
        <div className='mt-4 grid gap-5 md:grid-cols-2 xl:grid-cols-4'>
          <div className='min-w-0 space-y-2'>
            <h3 className='flex items-center gap-3 font-semibold break-all'>
              <span className='native-user-avatar' aria-hidden='true'>
                {user.platform_role === 'user' ? (
                  <UserRound size={20} />
                ) : (
                  <ShieldCheck size={20} />
                )}
              </span>
              {profile(user.username)}
            </h3>
            <dl className='space-y-1 text-sm break-all'>
              <div>
                <dt className='text-muted-foreground inline'>
                  {t('User ID')}:{' '}
                </dt>
                <dd className='inline font-mono'>{user.id}</dd>
              </div>
              <div>
                <dt className='text-muted-foreground inline'>
                  {t('Display name')}:{' '}
                </dt>
                <dd className='inline'>{profile(user.display_name)}</dd>
              </div>
              <div>
                <dt className='text-muted-foreground inline'>{t('Email')}: </dt>
                <dd className='inline'>{profile(user.email)}</dd>
              </div>
              {!user.profile_available && (
                <p className='text-muted-foreground'>
                  {t('Profile unavailable')}
                </p>
              )}
            </dl>
          </div>
          <div className='min-w-0 space-y-2 text-sm'>
            <h4 className='font-medium'>{t('Account')}</h4>
            <div className='flex flex-wrap gap-2'>
              <Badge
                variant={user.platform_role === 'user' ? 'info' : 'tertiary'}
              >
                {role}
              </Badge>
              <Badge variant={user.active ? 'success' : 'destructive'}>
                {user.active ? (
                  <CircleCheck aria-hidden='true' />
                ) : (
                  <CircleOff aria-hidden='true' />
                )}
                {user.active ? t('Active user') : t('Disabled user')}
              </Badge>
            </div>
            <dl className='space-y-1'>
              <div>
                <dt className='text-muted-foreground inline'>
                  {t('Account status')}:{' '}
                </dt>
                <dd className='inline'>
                  {user.account_active ? t('Active') : t('Inactive')}
                </dd>
              </div>
              <div>
                <dt className='text-muted-foreground inline'>
                  {t('Service level')}:{' '}
                </dt>
                <dd className='inline'>
                  <Badge variant='tertiary'>{user.service_level}</Badge>
                </dd>
              </div>
              <div>
                <dt className='text-muted-foreground inline'>
                  {t('Developer access')}:{' '}
                </dt>
                <dd className='inline'>
                  <Badge
                    variant={user.developer_access_granted ? 'info' : 'outline'}
                  >
                    {user.developer_access_granted
                      ? t('Granted')
                      : t('Not granted')}
                  </Badge>
                </dd>
              </div>
            </dl>
            <h4 className='font-medium'>{t('Explicit group memberships')}</h4>
            {user.groups.length === 0 ? (
              <p className='text-muted-foreground'>
                {t('No explicit group memberships')}
              </p>
            ) : (
              <ul className='flex flex-wrap gap-1'>
                {user.groups.map((group) => (
                  <li key={group} className='max-w-full'>
                    <Badge
                      variant='outline'
                      className='break-all whitespace-normal'
                    >
                      {group}
                    </Badge>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className='native-user-wallet min-w-0 space-y-2 text-sm'>
            <h4 className='flex items-center gap-2 font-medium'>
              <Wallet size={16} className='text-primary' aria-hidden='true' />
              {t('Wallet (Core credits)')}
            </h4>
            {wallet.status === 'unavailable' ? (
              <p>{t('Unavailable')}</p>
            ) : (
              <dl className='space-y-1 font-mono break-all'>
                <div>
                  <dt className='text-muted-foreground font-sans'>
                    {t('Economic position')}
                  </dt>
                  <dd
                    className={
                      wallet.available_units.startsWith('-')
                        ? 'text-destructive font-semibold'
                        : 'text-primary font-semibold'
                    }
                  >
                    {wallet.available_units}
                  </dd>
                </div>
                <div>
                  <dt className='text-muted-foreground font-sans'>
                    {t('Spendable wallet')}
                  </dt>
                  <dd>{wallet.wallet_units}</dd>
                </div>
                <div>
                  <dt className='text-muted-foreground font-sans'>
                    {t('Reserved funds')}
                  </dt>
                  <dd>{wallet.reserved_units}</dd>
                </div>
                <div>
                  <dt className='text-muted-foreground font-sans'>
                    {t('Debt magnitude')}
                  </dt>
                  <dd
                    className={
                      wallet.debt_units !== '0'
                        ? 'text-destructive font-semibold'
                        : undefined
                    }
                  >
                    {wallet.debt_units}
                  </dd>
                </div>
              </dl>
            )}
            <p className='text-muted-foreground'>
              {t(
                'Currency and money basis are unavailable. Amounts are exact Core credit units.'
              )}
            </p>
          </div>
          <dl className='min-w-0 space-y-2 text-sm break-all'>
            <div>
              <dt className='text-muted-foreground'>
                {t('Core account created')}
              </dt>
              <dd>{time(user.core_account_created_at_unix_ms)}</dd>
            </div>
            <div>
              <dt className='text-muted-foreground'>
                {t('Observed registration time')}
              </dt>
              <dd>{time(user.registered_at_unix_ms)}</dd>
            </div>
            <div>
              <dt className='text-muted-foreground'>
                {t('Personal account ID')}
              </dt>
              <dd className='font-mono'>{user.personal_account_id}</dd>
            </div>
            <div>
              <dt className='text-muted-foreground'>
                {t('Authentication version')}
              </dt>
              <dd className='font-mono'>{user.auth_version}</dd>
            </div>
          </dl>
        </div>
      </details>
    </article>
  )
}

function GroupsBrowser({
  scope,
  onSelect,
  onAccessDenied,
}: {
  scope: NativeUsersScope
  onSelect: (group: string) => void
  onAccessDenied: () => void
}) {
  const { t } = useTranslation()
  const [page, setPage] = useState(1)
  const [reload, setReload] = useState(0)
  const [state, setState] = useState<Load<NativeGroupsResponse>>({
    status: 'loading',
  })
  const request = useRef({
    alive: true,
    generation: 0,
    controller: undefined as AbortController | undefined,
  })
  const { userId, sessionId, role } = scope
  useEffect(() => {
    const holder = request.current
    holder.alive = true
    const generation = ++holder.generation
    const controller = new AbortController()
    holder.controller = controller
    const captured = { userId, sessionId, role }
    const owns = () =>
      holder.alive &&
      holder.generation === generation &&
      isCurrentNativeUsersScope(captured)
    setState({ status: 'loading' })
    void readNativeUserGroups(captured, page, controller.signal).then(
      (data) => {
        if (owns()) setState({ status: 'ready', data })
      },
      (error: unknown) => {
        if (owns()) {
          if (
            error instanceof NativeUsersFailure &&
            error.kind === 'forbidden'
          ) {
            onAccessDenied()
            return
          }
          setState({
            status: 'error',
            kind:
              error instanceof NativeUsersFailure ? error.kind : 'unavailable',
          })
        }
      }
    )
    return () => {
      holder.alive = false
      holder.generation++
      controller.abort()
    }
  }, [userId, sessionId, role, page, reload, onAccessDenied])
  const move = (next: number) => {
    request.current.generation++
    request.current.controller?.abort()
    setState({ status: 'loading' })
    setPage(next)
  }
  return (
    <section
      aria-label={t('Group directory')}
      className='space-y-3 rounded-lg border p-4'
      aria-busy={state.status === 'loading'}
    >
      <h3 className='font-medium'>{t('Group directory')}</h3>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Group choices are paginated independently. Membership does not grant administrator or spending authority.'
        )}
      </p>
      {state.status === 'loading' ? (
        <p role='status'>{t('Loading groups...')}</p>
      ) : state.status === 'error' ? (
        <Alert role='alert'>
          <AlertDescription>
            {t('Group directory is unavailable.')}
          </AlertDescription>
        </Alert>
      ) : (
        <>
          <p>
            {t('{{count}} groups · Page {{page}}', {
              count: state.data.total,
              page: state.data.page,
            })}
          </p>
          {state.data.items.length === 0 ? (
            <p>{t('No groups on this page.')}</p>
          ) : (
            <ul className='flex flex-wrap gap-2'>
              {state.data.items.map((group) => (
                <li key={group} className='max-w-full'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11 max-w-full break-all whitespace-normal sm:min-h-9'
                    onClick={() => onSelect(group)}
                  >
                    {group}
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      <div className='flex flex-wrap gap-2'>
        <Button
          type='button'
          variant='outline'
          className='min-h-11 sm:min-h-9'
          disabled={page <= 1 || state.status === 'loading'}
          onClick={() => move(page - 1)}
        >
          {t('Previous group page')}
        </Button>
        <Button
          type='button'
          variant='outline'
          className='min-h-11 sm:min-h-9'
          disabled={
            state.status !== 'ready' ||
            page >= 100000 ||
            page * 20 >= state.data.total
          }
          onClick={() => move(page + 1)}
        >
          {t('Next group page')}
        </Button>
        {state.status === 'error' && (
          <Button
            type='button'
            className='min-h-11 sm:min-h-9'
            variant='outline'
            onClick={() => {
              setState({ status: 'loading' })
              setReload((value) => value + 1)
            }}
          >
            {t('Retry group directory')}
          </Button>
        )}
      </div>
    </section>
  )
}

function UsersWorkspace({
  scope,
  recheck,
  initial,
  onAccessDenied,
}: Omit<Props, 'search'> & {
  initial: NativeUsersQuery
  onAccessDenied: () => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState(initial)
  const [query, setQuery] = useState(initial)
  const [state, setState] = useState<Load<NativeUsersResponse>>({
    status: 'loading',
  })
  const [invalidQuery, setInvalidQuery] = useState(false)
  const [groupsOpen, setGroupsOpen] = useState(false)
  const [reload, setReload] = useState(0)
  const request = useRef({
    alive: true,
    generation: 0,
    controller: undefined as AbortController | undefined,
  })
  const { userId, sessionId, role } = scope
  useEffect(() => {
    const holder = request.current
    holder.alive = true
    const generation = ++holder.generation
    const controller = new AbortController()
    holder.controller = controller
    const captured = { userId, sessionId, role }
    const owns = () =>
      holder.alive &&
      holder.generation === generation &&
      isCurrentNativeUsersScope(captured)
    setState({ status: 'loading' })
    void readNativeUsers(captured, query, controller.signal).then(
      (data) => {
        if (owns()) setState({ status: 'ready', data })
      },
      (error: unknown) => {
        if (owns()) {
          if (
            error instanceof NativeUsersFailure &&
            error.kind === 'forbidden'
          ) {
            onAccessDenied()
            return
          }
          setState({
            status: 'error',
            kind:
              error instanceof NativeUsersFailure ? error.kind : 'unavailable',
          })
        }
      }
    )
    return () => {
      holder.alive = false
      holder.generation++
      controller.abort()
    }
  }, [userId, sessionId, role, query, reload, onAccessDenied])
  const applyQuery = (next: NativeUsersQuery) => {
    try {
      const validated = nativeUsersQuery(next)
      request.current.generation++
      request.current.controller?.abort()
      setInvalidQuery(false)
      setState({ status: 'loading' })
      setQuery(validated)
      setDraft(validated)
    } catch {
      setInvalidQuery(true)
    }
  }
  const submit = (event: FormEvent) => {
    event.preventDefault()
    applyQuery({ ...draft, page: 1 })
  }
  const set = <Key extends keyof NativeUsersQuery>(
    key: Key,
    value: NativeUsersQuery[Key]
  ) => setDraft((previous) => ({ ...previous, [key]: value }))
  const sorts = {
    id: t('User ID'),
    username: t('Username'),
    display_name: t('Display name'),
    core_created_at: t('Core account created'),
    registered_at: t('Observed registration time'),
    groups: t('Explicit group memberships'),
    service_level: t('Service level'),
    role: t('Role'),
    status: t('User status'),
  }
  if (!isCurrentNativeUsersScope(scope)) return null
  return (
    <div className='space-y-5'>
      {state.status === 'ready' &&
        state.data.capabilities.implemented_write_actions.includes(
          'create'
        ) && (
          <NativeCreateUser
            key={`${scope.userId}:${scope.sessionId}`}
            scope={scope}
            onCreated={() => setReload((value) => value + 1)}
            onAccessDenied={onAccessDenied}
          />
        )}
      <form
        onSubmit={submit}
        className='space-y-4 rounded-lg border p-4'
        aria-label={t('User filters')}
      >
        <div>
          <div className='space-y-2'>
            <Label htmlFor='native-user-keyword'>
              {t('Search username, display name, email or user ID')}
            </Label>
            <Input
              id='native-user-keyword'
              className='min-h-11 sm:min-h-9'
              value={draft.keyword}
              onChange={(event) => set('keyword', event.target.value)}
            />
          </div>
        </div>
        <details
          open={Boolean(
            initial.group ||
            initial.role ||
            initial.status ||
            initial.service_level
          )}
        >
          <summary className='focus-visible:outline-ring cursor-pointer text-sm font-medium focus-visible:outline-2'>
            {t('Advanced filters')}
          </summary>
          <div className='mt-4 grid gap-4 sm:grid-cols-2 xl:grid-cols-4'>
            <div className='space-y-2'>
              <Label htmlFor='native-user-group'>
                {t('Exact group membership')}
              </Label>
              <Input
                id='native-user-group'
                className='min-h-11 sm:min-h-9'
                value={draft.group}
                onChange={(event) => set('group', event.target.value)}
              />
            </div>
            <FieldSelect
              id='native-user-role'
              label={t('Role')}
              value={draft.role}
              onChange={(value) =>
                set('role', value as NativeUsersQuery['role'])
              }
              options={[
                { value: '', label: t('All roles') },
                { value: 'user', label: t('User') },
                { value: 'admin', label: t('Administrator') },
                { value: 'superadmin', label: t('Super administrator') },
              ]}
            />
            <FieldSelect
              id='native-user-status'
              label={t('User status')}
              value={draft.status}
              onChange={(value) =>
                set('status', value as NativeUsersQuery['status'])
              }
              options={[
                { value: '', label: t('All user statuses') },
                { value: 'active', label: t('Active user') },
                { value: 'disabled', label: t('Disabled user') },
              ]}
            />
            <FieldSelect
              id='native-user-level'
              label={t('Service level')}
              value={draft.service_level}
              onChange={(value) =>
                set('service_level', value as NativeUsersQuery['service_level'])
              }
              options={[
                { value: '', label: t('All service levels') },
                ...['0', '1', '2', '3', '4'].map((value) => ({
                  value,
                  label: value,
                })),
              ]}
            />
            <FieldSelect
              id='native-user-sort'
              label={t('Sort by')}
              value={draft.sort_by}
              onChange={(value) =>
                set('sort_by', value as NativeUsersQuery['sort_by'])
              }
              options={Object.entries(sorts).map(([value, label]) => ({
                value,
                label,
              }))}
            />
            <FieldSelect
              id='native-user-order'
              label={t('Sort order')}
              value={draft.sort_order}
              onChange={(value) =>
                set('sort_order', value as NativeUsersQuery['sort_order'])
              }
              options={[
                { value: 'desc', label: t('Descending') },
                { value: 'asc', label: t('Ascending') },
              ]}
            />
            <FieldSelect
              id='native-user-page-size'
              label={t('Users per page')}
              value={String(draft.page_size)}
              onChange={(value) => set('page_size', Number(value))}
              options={[...new Set([draft.page_size, 20, 50, 100])]
                .sort((a, b) => a - b)
                .map((value) => ({
                  value: String(value),
                  label: String(value),
                }))}
            />
          </div>
          <p className='text-muted-foreground text-sm'>
            {t(
              'Service level is the native account level, independent of legacy trust. Group names match exactly, including spaces.'
            )}
          </p>
        </details>
        {invalidQuery && (
          <Alert role='alert'>
            <AlertDescription>
              {t(
                'These user filters are invalid. Check the supported values and input length.'
              )}
            </AlertDescription>
          </Alert>
        )}
        <div className='flex flex-wrap gap-2'>
          <Button type='submit' className='min-h-11 sm:min-h-9'>
            {t('Apply filters')}
          </Button>
          <Button
            type='button'
            variant='ghost'
            className='min-h-11 sm:min-h-9'
            onClick={() => applyQuery({ ...DEFAULT_NATIVE_USERS_QUERY })}
          >
            {t('Clear filters')}
          </Button>
          <Button
            type='button'
            variant='outline'
            className='min-h-11 sm:min-h-9'
            onClick={() => setGroupsOpen((value) => !value)}
          >
            {groupsOpen
              ? t('Hide group directory')
              : t('Browse group directory')}
          </Button>
          <Button
            type='button'
            variant='outline'
            className='min-h-11 sm:min-h-9'
            onClick={recheck}
          >
            {t('Recheck server availability')}
          </Button>
        </div>
      </form>
      {groupsOpen && (
        <GroupsBrowser
          scope={scope}
          onSelect={(value) => set('group', value)}
          onAccessDenied={onAccessDenied}
        />
      )}
      <section
        aria-label={t('User results')}
        aria-busy={state.status === 'loading'}
        className='space-y-4'
      >
        {state.status === 'loading' ? (
          <p role='status'>{t('Loading users...')}</p>
        ) : state.status === 'error' ? (
          <>
            <Alert role='alert'>
              <AlertDescription>
                {state.kind === 'forbidden'
                  ? t(
                      'Administrator access or multi-factor verification is required.'
                    )
                  : state.kind === 'invalid_response'
                    ? t('The server returned incompatible user data.')
                    : state.kind === 'limited'
                      ? t('User reads are rate limited. Try again later.')
                      : t(
                          'User data is unavailable. No previous results are displayed.'
                        )}
              </AlertDescription>
            </Alert>
            <Button
              type='button'
              variant='outline'
              className='min-h-11 sm:min-h-9'
              onClick={() => {
                setState({ status: 'loading' })
                setReload((value) => value + 1)
              }}
            >
              {t('Retry users')}
            </Button>
          </>
        ) : (
          <>
            <p aria-live='polite'>
              {t('{{count}} users · Page {{page}}', {
                count: state.data.total,
                page: state.data.page,
              })}
            </p>
            {state.data.items.length === 0 ? (
              <p>{t('No users on this page.')}</p>
            ) : (
              state.data.items.map((user) => (
                <UserRecord
                  key={user.id}
                  user={user}
                  scope={scope}
                  onUpdated={() => setReload((value) => value + 1)}
                  onAccessDenied={onAccessDenied}
                />
              ))
            )}
          </>
        )}
        <div className='flex flex-wrap gap-2'>
          <Button
            type='button'
            variant='outline'
            className='min-h-11 sm:min-h-9'
            disabled={query.page <= 1 || state.status === 'loading'}
            onClick={() => applyQuery({ ...query, page: query.page - 1 })}
          >
            {t('Previous user page')}
          </Button>
          <Button
            type='button'
            variant='outline'
            className='min-h-11 sm:min-h-9'
            disabled={
              state.status !== 'ready' ||
              query.page >= 100000 ||
              query.page * query.page_size >= state.data.total
            }
            onClick={() => applyQuery({ ...query, page: query.page + 1 })}
          >
            {t('Next user page')}
          </Button>
        </div>
      </section>
      <Alert>
        <AlertDescription>
          {t(
            'User management. Available actions are shown beside each user. Risk, usage, top-up, referral history and legacy trust information are unavailable in this view.'
          )}
        </AlertDescription>
      </Alert>
    </div>
  )
}

export function NativeUsersPage({ scope, recheck, search }: Props) {
  const { t } = useTranslation()
  const onAccessDenied = useContext(NativeUsersAccessBoundary)
  if (!onAccessDenied) {
    return (
      <Alert role='alert'>
        <AlertDescription>
          {t('A current administrator session is required.')}
        </AlertDescription>
      </Alert>
    )
  }
  const initial = nativeQueryFromSearch(search)
  if (!initial) {
    return (
      <Alert role='alert'>
        <AlertDescription className='space-y-3'>
          <p>
            {t(
              'This saved URL contains unsupported user filters or sorting. Clear it before loading users.'
            )}
          </p>
          <Button
            variant='outline'
            render={<a href='/users' />}
            className='min-h-11 sm:min-h-9'
          >
            {t('Clear saved user filters')}
          </Button>
        </AlertDescription>
      </Alert>
    )
  }
  return (
    <UsersWorkspace
      key={search}
      scope={scope}
      recheck={recheck}
      initial={initial}
      onAccessDenied={onAccessDenied}
    />
  )
}
