/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { defaultStringifySearch } from '@tanstack/react-router'
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  ChevronDown,
  CircleCheck,
  CircleOff,
  Columns3,
  History,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { EntityQuickActions } from '@/components/entity-quick-actions'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Table,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { toIntlLocale } from '@/i18n/languages'
import { copyToClipboard } from '@/lib/copy-to-clipboard'

import type { NativeUsersScope } from './access'
import { NativeUserAccess } from './access-editor'
import { NativeBalanceAdjustment } from './adjustments-editor'
import type { NativeUser, NativeUsersQuery, NativeUsersSort } from './contract'
import { NativeDisplayNameEditor } from './profile-editor'
import { NativeUserSecurity } from './security-editor'
import { NativeUserUsage } from './user-usage'

import './user-list.css'

export type NativeUserColumn = 'email' | 'teams' | 'groups' | 'role' | 'created'

type Props = {
  initialExpandedId?: string
  users: NativeUser[]
  scope: NativeUsersScope
  query: NativeUsersQuery
  selected: ReadonlySet<string>
  columns: ReadonlySet<NativeUserColumn>
  onSelect: (id: string, checked: boolean) => void
  onSelectPage: (checked: boolean) => void
  onSort: (sort: NativeUsersSort) => void
  onGroupSelect: (group: string) => void
  onTeamSelect: (teamId: string) => void
  onUpdated: () => void
  onAccessDenied: () => void
}

function UserRecord({
  user,
  scope,
  initialExpandedId,
  checked,
  columns,
  onGroupSelect,
  onTeamSelect,
  onSelect,
  onUpdated,
  onAccessDenied,
}: Pick<
  Props,
  | 'scope'
  | 'initialExpandedId'
  | 'onSelect'
  | 'onUpdated'
  | 'onAccessDenied'
  | 'onGroupSelect'
  | 'onTeamSelect'
> & {
  user: NativeUser
  checked: boolean
  columns: ReadonlySet<NativeUserColumn>
}) {
  const { t, i18n } = useTranslation()
  const [expanded, setExpanded] = useState(initialExpandedId === user.id)
  const profile = (value: string | null) =>
    value === null ? t('Unavailable') : value || '—'
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
  const name = user.username || user.id
  const wallet = user.wallet
  const historyOnly = user.identity_origin === 'history_only'
  const suspended = !user.active && user.account_active
  const number = (value: string) =>
    new Intl.NumberFormat(toIntlLocale(i18n.language)).format(BigInt(value))
  const detailsId = `native-user-details-${user.id}`

  return (
    <tbody data-native-user-id={user.id} className='native-user-record'>
      <TableRow
        data-state={checked ? 'selected' : undefined}
        className='native-user-row'
      >
        <TableCell className='native-user-select'>
          <Checkbox
            title={t('Select for CSV export')}
            checked={checked}
            onCheckedChange={(value) => onSelect(user.id, value)}
            aria-label={t('Select user {{name}}', { name })}
          />
        </TableCell>
        <TableCell className='native-user-id font-mono text-xs'>
          <button
            type='button'
            className='focus-visible:outline-ring flex min-h-11 max-w-full items-center rounded text-left hover:underline focus-visible:outline-2 sm:min-h-8'
            title={user.id}
            aria-label={[t('Copy'), t('User ID'), user.id].join(' ')}
            onClick={async () => {
              const copied = await copyToClipboard(user.id)
              if (copied) toast.success(t('Copied'))
              else toast.error(t('Failed to copy'))
            }}
          >
            <span className='truncate'>#{user.id}</span>
          </button>
        </TableCell>
        <TableCell className='native-user-identity'>
          <div className='flex min-w-0 items-center gap-3'>
            <span
              className='bg-muted text-muted-foreground flex size-9 shrink-0 items-center justify-center rounded-full text-xs font-medium'
              aria-hidden='true'
            >
              {Array.from(user.display_name || user.username || user.id)
                .slice(0, 1)
                .join('')
                .toUpperCase()}
            </span>
            <div className='min-w-0'>
              <p
                className='truncate font-medium'
                title={user.username || undefined}
              >
                <EntityQuickActions
                  kind='username'
                  value={user.username ?? ''}
                  label={profile(user.username)}
                />
              </p>
              {user.display_name && user.display_name !== user.username && (
                <p
                  className='text-muted-foreground mt-1 truncate text-xs'
                  title={user.display_name}
                >
                  {user.display_name}
                </p>
              )}
            </div>
          </div>
        </TableCell>
        {columns.has('email') && (
          <TableCell className='native-user-email'>
            <p className='native-user-field-label'>{t('Email')}</p>
            <p
              className='text-muted-foreground truncate'
              title={user.email || undefined}
            >
              {profile(user.email)}
            </p>
          </TableCell>
        )}
        <TableCell className='native-user-status'>
          <p
            className={`flex items-center gap-1.5 text-xs ${suspended ? 'text-destructive' : ''}`}
          >
            {historyOnly ? (
              <History
                className='text-muted-foreground size-3.5 shrink-0'
                aria-hidden='true'
              />
            ) : user.active ? (
              <CircleCheck
                className='text-success size-3.5 shrink-0'
                aria-hidden='true'
              />
            ) : (
              <CircleOff className='size-3.5 shrink-0' aria-hidden='true' />
            )}
            {historyOnly
              ? t('History only')
              : user.active
                ? t('Active user')
                : suspended
                  ? t('Disabled user')
                  : t('Sign-in inactive')}
          </p>
          {!user.account_active && (
            <p className='text-muted-foreground mt-1 text-xs'>
              {historyOnly
                ? t('Login identity not migrated')
                : t('Inactive account')}
            </p>
          )}
        </TableCell>
        {columns.has('role') && (
          <TableCell className='native-user-role'>
            <p className='text-xs'>{role}</p>
          </TableCell>
        )}
        {columns.has('teams') && (
          <TableCell className='native-user-teams'>
            <p className='native-user-field-label'>{t('Teams')}</p>
            {user.teams.length === 0 ? (
              <span className='text-muted-foreground text-xs'>
                {t('Personal account')}
              </span>
            ) : (
              <div className='flex min-w-0 items-center gap-1.5'>
                <button
                  type='button'
                  className='focus-visible:outline-ring min-w-0 truncate rounded text-left text-sm hover:underline focus-visible:outline-2'
                  title={user.teams[0].name}
                  aria-label={t('Filter by team {{name}}', {
                    name: user.teams[0].name,
                  })}
                  onClick={() => onTeamSelect(user.teams[0].id)}
                >
                  {user.teams[0].name ||
                    t('Team #{{id}}', { id: user.teams[0].id })}
                </button>
                {user.teams.length > 1 && (
                  <Button
                    variant='ghost'
                    size='sm'
                    className='h-7 shrink-0 px-1.5 text-xs'
                    aria-label={t('View all team memberships')}
                    aria-expanded={expanded}
                    aria-controls={detailsId}
                    onClick={() => setExpanded(true)}
                  >
                    +{user.teams.length - 1}
                  </Button>
                )}
              </div>
            )}
          </TableCell>
        )}
        {columns.has('groups') && (
          <TableCell className='native-user-groups'>
            <p className='native-user-field-label'>{t('Routing groups')}</p>
            {user.groups.length === 0 ? (
              <span className='text-muted-foreground'>—</span>
            ) : (
              <div className='flex min-w-0 items-center gap-1.5'>
                <button
                  type='button'
                  className='focus-visible:outline-ring min-w-0 truncate rounded text-left text-xs hover:underline focus-visible:outline-2'
                  title={user.groups[0]}
                  aria-label={t('Filter by group {{group}}', {
                    group: user.groups[0],
                  })}
                  onClick={() => onGroupSelect(user.groups[0])}
                >
                  {user.groups[0]}
                </button>
                {user.groups.length > 1 && (
                  <Button
                    variant='ghost'
                    size='sm'
                    className='h-7 shrink-0 px-1.5 text-xs'
                    aria-label={t('View group memberships ({{count}})', {
                      count: user.groups.length,
                    })}
                    aria-expanded={expanded}
                    aria-controls={detailsId}
                    onClick={() => setExpanded(true)}
                  >
                    +{user.groups.length - 1}
                  </Button>
                )}
              </div>
            )}
          </TableCell>
        )}
        <TableCell className='native-user-wallet'>
          <p className='native-user-field-label'>{t('Spendable wallet')}</p>
          <p className='font-medium break-all tabular-nums'>
            {wallet.status === 'available' ? number(wallet.wallet_units) : '—'}
          </p>
          <p className='text-muted-foreground mt-1 text-xs'>
            {wallet.status === 'available'
              ? t('Core credits')
              : t('Wallet unavailable')}
          </p>
        </TableCell>
        {columns.has('created') && (
          <TableCell className='native-user-created'>
            <p className='native-user-field-label'>
              {t('Core account created')}
            </p>
            <time
              className='text-muted-foreground text-xs'
              dateTime={new Date(
                user.core_account_created_at_unix_ms
              ).toISOString()}
              title={time(user.core_account_created_at_unix_ms)}
            >
              {new Intl.DateTimeFormat(toIntlLocale(i18n.language), {
                dateStyle: 'medium',
              }).format(user.core_account_created_at_unix_ms)}
            </time>
          </TableCell>
        )}
        <TableCell className='native-user-actions'>
          <Button
            variant='ghost'
            size='sm'
            type='button'
            className='min-h-11 gap-1 sm:min-h-9'
            aria-label={t('User details')}
            aria-expanded={expanded}
            aria-controls={detailsId}
            onClick={() => setExpanded((value) => !value)}
          >
            {t('Manage')}
            <ChevronDown
              className={expanded ? 'rotate-180' : undefined}
              aria-hidden='true'
            />
          </Button>
        </TableCell>
      </TableRow>
      <tr className='native-user-expanded-row' hidden={!expanded}>
        <td colSpan={6 + columns.size}>
          <div id={detailsId} className='space-y-5'>
            <header className='flex flex-wrap items-start justify-between gap-4'>
              <div className='min-w-0 space-y-1'>
                <h3 className='font-medium break-all'>
                  {user.display_name || user.username || user.id}
                </h3>
                <p className='text-muted-foreground text-sm break-all'>
                  {user.username && `@${user.username} · `}#{user.id}
                </p>
              </div>
              <div className='flex flex-wrap gap-2 [&_button]:min-h-11 sm:[&_button]:min-h-9'>
                {user.username && (
                  <Button
                    variant='ghost'
                    size='sm'
                    className='min-h-11 sm:min-h-9'
                    render={
                      <a
                        href={`/usage-logs/common${defaultStringifySearch({
                          username: user.username,
                          source: 'native',
                          scope: 'all',
                        })}`}
                      />
                    }
                  >
                    {t('View logs')}
                  </Button>
                )}
                {user.platform_role === 'user' && (
                  <>
                    <NativeDisplayNameEditor
                      scope={scope}
                      userId={user.id}
                      onUpdated={onUpdated}
                      onAccessDenied={onAccessDenied}
                    />
                    <NativeUserAccess
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
                    {wallet.status === 'available' && (
                      <NativeBalanceAdjustment
                        scope={scope}
                        userId={user.id}
                        wallet={wallet}
                        onUpdated={onUpdated}
                        onAccessDenied={onAccessDenied}
                      />
                    )}
                  </>
                )}
              </div>
            </header>
            {!user.active && (
              <p className='text-muted-foreground text-sm'>
                {t(
                  'This account is inactive. Viewing its history does not enable sign-in or spending.'
                )}
              </p>
            )}
            <section className='space-y-3' aria-label={t('Team memberships')}>
              <h3 className='font-medium'>{t('Team memberships')}</h3>
              {user.teams.length === 0 ? (
                <p className='text-muted-foreground text-sm'>
                  {t('No team memberships')}
                </p>
              ) : (
                <ul className='grid gap-3 @min-[40rem]/users:grid-cols-2'>
                  {user.teams.map((team) => (
                    <li
                      key={team.id}
                      className='bg-background/70 flex flex-wrap items-center justify-between gap-3 rounded-xl px-4 py-3'
                    >
                      <div className='min-w-0'>
                        <Button
                          variant='link'
                          className='h-auto max-w-full p-0 text-left break-words whitespace-normal'
                          onClick={() => onTeamSelect(team.id)}
                          aria-label={t('Filter by team {{name}}', {
                            name: team.name,
                          })}
                        >
                          {team.name || t('Team #{{id}}', { id: team.id })}
                        </Button>
                        <p className='text-muted-foreground mt-1 text-xs'>
                          {t('Team #{{id}}', { id: team.id })} ·{' '}
                          {team.role === 'owner'
                            ? t('Owner')
                            : team.role === 'admin'
                              ? t('Administrator')
                              : t('Member')}
                        </p>
                      </div>
                      <span
                        className={
                          team.can_spend
                            ? 'text-success text-xs'
                            : 'text-muted-foreground text-xs'
                        }
                      >
                        {team.can_spend
                          ? t('Spending enabled')
                          : t('Spending disabled')}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
            {expanded && (
              <NativeUserUsage
                scope={scope}
                userId={user.id}
                onAccessDenied={onAccessDenied}
              />
            )}

            <div className='space-y-2 text-sm'>
              <h3 className='font-medium'>{t('Routing groups')}</h3>
              <div className='flex flex-wrap gap-2'>
                {user.groups.length === 0 ? (
                  <span className='text-muted-foreground'>
                    {t('No explicit group memberships')}
                  </span>
                ) : (
                  user.groups.map((group) => (
                    <Button
                      key={group}
                      size='sm'
                      variant='outline'
                      className='h-auto min-h-9 max-w-full break-all whitespace-normal'
                      aria-label={t('Filter by group {{group}}', { group })}
                      onClick={() => onGroupSelect(group)}
                    >
                      {group}
                    </Button>
                  ))
                )}
              </div>
            </div>
            <div className='grid gap-x-8 gap-y-5 sm:grid-cols-2 xl:grid-cols-4'>
              <dl className='min-w-0 space-y-3 text-sm'>
                <div>
                  <dt className='text-muted-foreground'>{t('Display name')}</dt>
                  <dd className='mt-1 break-all'>
                    {profile(user.display_name)}
                  </dd>
                </div>
                {!user.profile_available && (
                  <p className='text-muted-foreground'>
                    {t('Profile unavailable')}
                  </p>
                )}
                <div>
                  <dt className='text-muted-foreground'>
                    {t('Account status')}
                  </dt>
                  <dd className='mt-1'>
                    {user.account_active ? t('Active') : t('Inactive')}
                  </dd>
                </div>
              </dl>
              <dl className='min-w-0 space-y-3 text-sm'>
                <div>
                  <dt className='text-muted-foreground'>
                    {t('Core account created')}
                  </dt>
                  <dd className='mt-1'>
                    {time(user.core_account_created_at_unix_ms)}
                  </dd>
                </div>
                <div>
                  <dt className='text-muted-foreground'>
                    {t('Observed registration time')}
                  </dt>
                  <dd className='mt-1'>{time(user.registered_at_unix_ms)}</dd>
                </div>
              </dl>
              <div className='min-w-0 space-y-3 text-sm'>
                <h3 className='font-medium'>{t('Wallet (Core credits)')}</h3>
                {wallet.status === 'unavailable' ? (
                  <p>{t('Unavailable')}</p>
                ) : (
                  <dl className='space-y-2'>
                    <div>
                      <dt className='text-muted-foreground'>
                        {t('Economic position')}
                      </dt>
                      <dd
                        className={`font-mono break-all ${wallet.available_units.startsWith('-') ? 'text-destructive' : ''}`}
                      >
                        {wallet.available_units}
                      </dd>
                    </div>
                    <div>
                      <dt className='text-muted-foreground'>
                        {t('Reserved funds')}
                      </dt>
                      <dd className='font-mono break-all'>
                        {wallet.reserved_units}
                      </dd>
                    </div>
                    <div>
                      <dt className='text-muted-foreground'>
                        {t('Debt magnitude')}
                      </dt>
                      <dd
                        className={`font-mono break-all ${wallet.debt_units !== '0' ? 'text-destructive' : ''}`}
                      >
                        {wallet.debt_units}
                      </dd>
                    </div>
                  </dl>
                )}
              </div>
              <dl className='min-w-0 space-y-3 text-sm'>
                <div>
                  <dt className='text-muted-foreground'>
                    {t('Personal account ID')}
                  </dt>
                  <dd className='mt-1 font-mono break-all'>
                    {user.personal_account_id}
                  </dd>
                </div>
                <div>
                  <dt className='text-muted-foreground'>
                    {t('Authentication version')}
                  </dt>
                  <dd className='mt-1 font-mono break-all'>
                    {user.auth_version}
                  </dd>
                </div>
              </dl>
            </div>
            <p className='text-muted-foreground max-w-3xl text-xs'>
              {t(
                'Currency and money basis are unavailable. Amounts are exact Core credit units.'
              )}
            </p>
          </div>
        </td>
      </tr>
    </tbody>
  )
}

export function NativeUserColumns({
  columns,
  onColumnsChange,
}: {
  columns: ReadonlySet<NativeUserColumn>
  onColumnsChange: (columns: ReadonlySet<NativeUserColumn>) => void
}) {
  const { t } = useTranslation()
  const optionalColumns: [NativeUserColumn, string][] = [
    ['email', t('Email')],
    ['teams', t('Teams')],
    ['groups', t('Routing groups')],
    ['role', t('Role')],
    ['created', t('Core account created')],
  ]
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button variant='ghost' size='sm' className='min-h-11 sm:min-h-9' />
        }
      >
        <Columns3 aria-hidden='true' />
        {t('Columns')}
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end'>
        {optionalColumns.map(([key, label]) => (
          <DropdownMenuCheckboxItem
            key={key}
            checked={columns.has(key)}
            onCheckedChange={(checked) => {
              const next = new Set(columns)
              if (checked) next.add(key)
              else next.delete(key)
              onColumnsChange(next)
            }}
          >
            {label}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function NativeUserList({
  users,
  query,
  selected,
  onSelectPage,
  onSort,
  columns,
  ...props
}: Props) {
  const { t } = useTranslation()
  const allSelected =
    users.length > 0 && users.every((user) => selected.has(user.id))
  const someSelected = users.some((user) => selected.has(user.id))
  const sortHeading = (key: NativeUsersSort, label: string) => {
    const active = query.sort_by === key
    const Icon = active
      ? query.sort_order === 'asc'
        ? ArrowUp
        : ArrowDown
      : ArrowUpDown
    return (
      <Button
        type='button'
        variant='ghost'
        size='sm'
        className='-ml-3 font-medium'
        onClick={() => onSort(key)}
      >
        {label}
        <Icon className='size-3.5 opacity-60' aria-hidden='true' />
      </Button>
    )
  }
  const sort = (key: NativeUsersSort) =>
    query.sort_by !== key
      ? 'none'
      : query.sort_order === 'asc'
        ? 'ascending'
        : 'descending'
  return (
    <div className='native-users-list'>
      <label className='native-users-mobile-select text-muted-foreground min-h-11 items-center gap-3 px-2 text-sm'>
        <Checkbox
          checked={allSelected}
          indeterminate={someSelected && !allSelected}
          onCheckedChange={onSelectPage}
        />
        {t('Select current page')} · {t('CSV export')}
      </label>
      <Table className='native-users-table'>
        <TableHeader className='native-users-heading'>
          <TableRow className='hover:bg-transparent'>
            <TableHead className='native-user-select'>
              <Checkbox
                checked={allSelected}
                indeterminate={someSelected && !allSelected}
                onCheckedChange={onSelectPage}
                title={t('Select for CSV export')}
                aria-label={t('Select current page')}
              />
            </TableHead>
            <TableHead className='native-user-id' aria-sort={sort('id')}>
              {sortHeading('id', t('ID'))}
            </TableHead>
            <TableHead
              className='native-user-identity'
              aria-sort={sort('username')}
            >
              {sortHeading('username', t('Username'))}
            </TableHead>
            {columns.has('email') && (
              <TableHead className='native-user-email'>{t('Email')}</TableHead>
            )}
            <TableHead
              className='native-user-status'
              aria-sort={sort('status')}
            >
              {sortHeading('status', t('Status'))}
            </TableHead>
            {columns.has('role') && (
              <TableHead className='native-user-role' aria-sort={sort('role')}>
                {sortHeading('role', t('Role'))}
              </TableHead>
            )}
            {columns.has('teams') && (
              <TableHead className='native-user-teams'>{t('Teams')}</TableHead>
            )}
            {columns.has('groups') && (
              <TableHead
                className='native-user-groups'
                aria-sort={sort('groups')}
              >
                {sortHeading('groups', t('Routing groups'))}
              </TableHead>
            )}
            <TableHead className='native-user-wallet text-right whitespace-normal'>
              {t('Spendable wallet')}
            </TableHead>
            {columns.has('created') && (
              <TableHead
                className='native-user-created'
                aria-sort={sort('core_created_at')}
              >
                {sortHeading('core_created_at', t('Created'))}
              </TableHead>
            )}
            <TableHead className='native-user-actions'>
              <span className='sr-only'>{t('User details')}</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        {users.map((user) => (
          <UserRecord
            key={user.id}
            user={user}
            checked={selected.has(user.id)}
            columns={columns}
            {...props}
          />
        ))}
      </Table>
    </div>
  )
}
