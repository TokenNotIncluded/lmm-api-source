/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  Copy,
  Download,
  RefreshCw,
  Search,
  SlidersHorizontal,
  Users,
} from 'lucide-react'
import { useContext, useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import {
  isCurrentNativeUsersScope,
  NativeUsersAccessBoundary,
  NativeUsersFailure,
  type NativeUsersScope,
} from './access'
import { readNativeUserGroups, readNativeUsers } from './api'
import {
  DEFAULT_NATIVE_USERS_QUERY,
  nativeUsersQuery,
  type NativeGroupsResponse,
  type NativeUsersQuery,
  type NativeUsersResponse,
} from './contract'
import { NativeCreateUser } from './create-editor'
import { nativeUsersCsv } from './export'
import { nativeQueryFromSearch, nativeUsersSearch } from './search'
import { NativeUserList, type NativeUserColumn } from './user-list'

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
    <div className='min-w-0 space-y-2'>
      <Label htmlFor={id}>{label}</Label>
      <select
        id={id}
        className='bg-muted/60 focus-visible:ring-ring min-h-11 w-full rounded-lg border border-transparent px-3 py-2 text-base focus-visible:ring-2 sm:text-sm'
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
      className='bg-muted/30 space-y-4 rounded-xl p-5'
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
  const [filtersOpen, setFiltersOpen] = useState(
    Boolean(
      initial.group || initial.role || initial.status || initial.service_level
    )
  )
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const [columns, setColumns] = useState<ReadonlySet<NativeUserColumn>>(
    new Set(['email', 'groups', 'role', 'created'])
  )
  const [feedback, setFeedback] = useState<
    'none' | 'copied' | 'copy_failed' | 'exported' | 'export_failed'
  >('none')
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

  const clearResults = () => {
    request.current.generation++
    request.current.controller?.abort()
    setSelected(new Set())
    setFeedback('none')
    setState({ status: 'loading' })
  }
  const refresh = () => {
    clearResults()
    setReload((value) => value + 1)
  }
  const applyQuery = (next: NativeUsersQuery) => {
    try {
      const validated = nativeUsersQuery(next)
      clearResults()
      setInvalidQuery(false)
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
  const copyFilters = async () => {
    if (!isCurrentNativeUsersScope(scope)) return
    const generation = request.current.generation
    const owns = () =>
      request.current.alive &&
      request.current.generation === generation &&
      isCurrentNativeUsersScope(scope)
    try {
      await navigator.clipboard.writeText(
        `${window.location.origin}/users${nativeUsersSearch(query)}`
      )
      if (owns()) setFeedback('copied')
    } catch {
      if (owns()) setFeedback('copy_failed')
    }
  }
  const exportUsers = () => {
    if (state.status !== 'ready' || !isCurrentNativeUsersScope(scope)) return
    const rows = selected.size
      ? state.data.items.filter((user) => selected.has(user.id))
      : state.data.items
    if (!rows.length) return
    try {
      const csv = nativeUsersCsv(rows, [
        t('User ID'),
        t('Profile available'),
        t('Username'),
        t('Display name'),
        t('Email'),
        t('Role'),
        t('User status'),
        t('Account status'),
        t('Service level'),
        t('Developer access'),
        t('Explicit group memberships'),
        t('Registration time (Unix ms)'),
        t('Account creation time (Unix ms)'),
        t('Wallet status'),
        t('Unit'),
        t('Economic position'),
        t('Spendable wallet'),
        t('Reserved funds'),
        t('Debt magnitude'),
      ])
      const url = URL.createObjectURL(
        new Blob([csv], { type: 'text/csv;charset=utf-8' })
      )
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = `users-page-${query.page}-${new Date().toISOString().slice(0, 10)}.csv`
      document.body.appendChild(anchor)
      try {
        anchor.click()
      } finally {
        anchor.remove()
        window.setTimeout(() => URL.revokeObjectURL(url), 1000)
      }
      setFeedback('exported')
    } catch {
      setFeedback('export_failed')
    }
  }
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
    <div className='@container/users w-full min-w-0 space-y-5'>
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <p className='text-muted-foreground max-w-xl text-sm'>
          {t('Find accounts, update access and review balances.')}
        </p>
        <div className='flex flex-wrap gap-1'>
          <Button
            type='button'
            variant='ghost'
            className='min-h-11'
            onClick={copyFilters}
          >
            <Copy aria-hidden='true' />
            {t('Copy filter link')}
          </Button>
          <Button
            type='button'
            variant='ghost'
            className='min-h-11'
            onClick={refresh}
            disabled={state.status === 'loading'}
          >
            <RefreshCw aria-hidden='true' />
            {t('Refresh users')}
          </Button>
        </div>
      </div>
      {state.status === 'ready' &&
        state.data.capabilities.implemented_write_actions.includes(
          'create'
        ) && (
          <div className='[&>section]:rounded-none [&>section]:border-0 [&>section]:p-0'>
            <NativeCreateUser
              key={`${scope.userId}:${scope.sessionId}`}
              scope={scope}
              onCreated={refresh}
              onAccessDenied={onAccessDenied}
            />
          </div>
        )}
      <form
        onSubmit={submit}
        className='space-y-5'
        aria-label={t('User filters')}
      >
        <div className='grid grid-cols-1 items-end gap-3 @min-[28rem]/users:grid-cols-2 @min-[64rem]/users:grid-cols-[minmax(0,1fr)_11rem_11rem_auto]'>
          <div className='col-span-full min-w-0 space-y-2 @min-[64rem]/users:col-span-1'>
            <Label htmlFor='native-user-keyword'>
              {t('Search username, display name, email or user ID')}
            </Label>
            <div className='relative'>
              <Search
                className='text-muted-foreground pointer-events-none absolute top-3.5 left-3 size-4'
                aria-hidden='true'
              />
              <Input
                id='native-user-keyword'
                type='search'
                className='min-h-11 pl-10'
                value={draft.keyword}
                onChange={(event) => set('keyword', event.target.value)}
              />
            </div>
          </div>
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
            id='native-user-role'
            label={t('Role')}
            value={draft.role}
            onChange={(value) => set('role', value as NativeUsersQuery['role'])}
            options={[
              { value: '', label: t('All roles') },
              { value: 'user', label: t('User') },
              { value: 'admin', label: t('Administrator') },
              { value: 'superadmin', label: t('Super administrator') },
            ]}
          />
          <Button
            type='submit'
            className='col-span-full min-h-11 @min-[64rem]/users:col-span-1'
          >
            {t('Apply filters')}
          </Button>
        </div>
        <details
          open={filtersOpen}
          onToggle={(event) => setFiltersOpen(event.currentTarget.open)}
        >
          <summary className='focus-visible:outline-ring inline-flex min-h-11 cursor-pointer list-none items-center gap-2 text-sm font-medium focus-visible:outline-2 [&::-webkit-details-marker]:hidden'>
            <SlidersHorizontal className='size-4' aria-hidden='true' />
            {t('Advanced filters')}
          </summary>
          <div className='bg-muted/25 mt-2 space-y-5 rounded-xl p-5'>
            <div className='grid gap-5 @min-[28rem]/users:grid-cols-2 @min-[64rem]/users:grid-cols-3'>
              <div className='space-y-2'>
                <Label htmlFor='native-user-group'>
                  {t('Exact group membership')}
                </Label>
                <Input
                  id='native-user-group'
                  className='min-h-11'
                  value={draft.group}
                  onChange={(event) => set('group', event.target.value)}
                />
              </div>
              <FieldSelect
                id='native-user-level'
                label={t('Service level')}
                value={draft.service_level}
                onChange={(value) =>
                  set(
                    'service_level',
                    value as NativeUsersQuery['service_level']
                  )
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
            <div className='flex flex-wrap gap-2'>
              <Button type='submit' className='min-h-11'>
                {t('Apply filters')}
              </Button>
              <Button
                type='button'
                variant='outline'
                className='min-h-11'
                onClick={() => setGroupsOpen((value) => !value)}
              >
                {groupsOpen
                  ? t('Hide group directory')
                  : t('Browse group directory')}
              </Button>
              <Button
                type='button'
                variant='ghost'
                className='min-h-11'
                onClick={recheck}
              >
                {t('Recheck server availability')}
              </Button>
            </div>
            <p className='text-muted-foreground text-xs'>
              {t(
                'Service level is the native account level, independent of legacy trust. Group names match exactly, including spaces.'
              )}
            </p>
          </div>
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
        <Button
          type='button'
          variant='ghost'
          className='min-h-11'
          onClick={() => {
            setFiltersOpen(false)
            applyQuery({ ...DEFAULT_NATIVE_USERS_QUERY })
          }}
        >
          {t('Clear filters')}
        </Button>
      </form>
      {groupsOpen && (
        <GroupsBrowser
          scope={scope}
          onSelect={(value) => {
            set('group', value)
            setFiltersOpen(true)
          }}
          onAccessDenied={onAccessDenied}
        />
      )}
      {feedback !== 'none' && (
        <p
          className={
            feedback.endsWith('failed')
              ? 'text-destructive text-sm'
              : 'text-muted-foreground text-sm'
          }
          role={feedback.endsWith('failed') ? 'alert' : 'status'}
        >
          {feedback === 'copied'
            ? t('Filter link copied.')
            : feedback === 'copy_failed'
              ? t('Could not copy the filter link.')
              : feedback === 'exported'
                ? t('CSV download started.')
                : t('Could not export users.')}
        </p>
      )}
      <section
        aria-label={t('User results')}
        aria-busy={state.status === 'loading'}
        className='space-y-4'
      >
        {state.status === 'loading' ? (
          <p role='status' className='text-muted-foreground py-12 text-center'>
            {t('Loading users...')}
          </p>
        ) : state.status === 'error' ? (
          <div className='space-y-4 py-4'>
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
              className='min-h-11'
              onClick={refresh}
            >
              {t('Retry users')}
            </Button>
          </div>
        ) : (
          <>
            <div className='flex flex-wrap items-center justify-between gap-3'>
              <p
                aria-live='polite'
                className='text-sm font-medium tabular-nums'
              >
                {t('Showing {{start}}–{{end}} of {{total}} users', {
                  start: state.data.items.length
                    ? (query.page - 1) * query.page_size + 1
                    : 0,
                  end: state.data.items.length
                    ? (query.page - 1) * query.page_size +
                      state.data.items.length
                    : 0,
                  total: state.data.total,
                })}
              </p>
              <div className='flex flex-wrap items-center gap-2'>
                {selected.size > 0 && (
                  <>
                    <span
                      className='text-muted-foreground text-sm'
                      aria-live='polite'
                    >
                      {t('Selected: {{selected}}', { selected: selected.size })}
                    </span>
                    <Button
                      type='button'
                      variant='ghost'
                      className='min-h-11'
                      onClick={() => setSelected(new Set())}
                    >
                      {t('Clear selection')}
                    </Button>
                  </>
                )}
                <Button
                  type='button'
                  variant='outline'
                  className='min-h-11'
                  disabled={!state.data.items.length}
                  onClick={exportUsers}
                >
                  <Download aria-hidden='true' />
                  {selected.size
                    ? t('Export selected')
                    : t('Export current page')}
                </Button>
              </div>
            </div>
            {state.data.items.length === 0 ? (
              <div className='flex flex-col items-center gap-3 py-14 text-center'>
                <Users
                  className='text-muted-foreground size-7'
                  aria-hidden='true'
                />
                <h3 className='font-medium'>{t('No users on this page.')}</h3>
                <p className='text-muted-foreground text-sm'>
                  {t('Try another search or clear the filters.')}
                </p>
              </div>
            ) : (
              <NativeUserList
                users={state.data.items}
                scope={scope}
                query={query}
                selected={selected}
                columns={columns}
                onColumnsChange={setColumns}
                onGroupSelect={(group) => {
                  setFiltersOpen(true)
                  applyQuery({ ...query, group, page: 1 })
                }}
                onSelect={(id, checked) =>
                  setSelected((previous) => {
                    const next = new Set(previous)
                    if (checked) next.add(id)
                    else next.delete(id)
                    return next
                  })
                }
                onSelectPage={(checked) =>
                  setSelected(
                    new Set(
                      checked ? state.data.items.map((user) => user.id) : []
                    )
                  )
                }
                onSort={(sort_by) =>
                  applyQuery({
                    ...query,
                    sort_by,
                    sort_order:
                      query.sort_by === sort_by && query.sort_order === 'asc'
                        ? 'desc'
                        : 'asc',
                    page: 1,
                  })
                }
                onUpdated={refresh}
                onAccessDenied={onAccessDenied}
              />
            )}
          </>
        )}
        <div className='flex flex-wrap items-center justify-between gap-3 pt-2'>
          <p className='text-muted-foreground text-sm tabular-nums'>
            {state.status === 'ready' &&
              t('Page {{page}} of {{pages}}', {
                page: query.page,
                pages: Math.max(
                  1,
                  Math.ceil(state.data.total / query.page_size)
                ),
              })}
          </p>
          <div className='flex flex-wrap gap-2'>
            <Button
              type='button'
              variant='ghost'
              className='min-h-11'
              disabled={query.page <= 1 || state.status === 'loading'}
              onClick={() => applyQuery({ ...query, page: query.page - 1 })}
            >
              {t('Previous user page')}
            </Button>
            <Button
              type='button'
              variant='ghost'
              className='min-h-11'
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
        </div>
      </section>
      <details className='text-muted-foreground text-xs'>
        <summary className='focus-visible:outline-ring inline-flex min-h-11 cursor-pointer items-center focus-visible:outline-2'>
          {t('About user data')}
        </summary>
        <div className='max-w-3xl space-y-2 pb-4'>
          <p>
            {t(
              'Open Manage to review account usage, imported counters and available actions. Empty fields are shown as a dash; unavailable data is never a zero balance.'
            )}
          </p>
          <p>
            {t(
              'CSV downloads contain only the current page or your selection. IDs and credit amounts are stored as text to preserve precision.'
            )}
          </p>
        </div>
      </details>
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
