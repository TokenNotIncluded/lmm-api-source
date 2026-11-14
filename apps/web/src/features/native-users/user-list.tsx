/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  ChevronDown,
  CircleCheck,
  CircleOff,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { EntityQuickActions } from '@/components/entity-quick-actions'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Table,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { toIntlLocale } from '@/i18n/languages'

import type { NativeUsersScope } from './access'
import { NativeUserAccess } from './access-editor'
import { NativeBalanceAdjustment } from './adjustments-editor'
import type { NativeUser, NativeUsersQuery, NativeUsersSort } from './contract'
import { NativeDisplayNameEditor } from './profile-editor'
import { NativeUserSecurity } from './security-editor'

type Props = {
  users: NativeUser[]
  scope: NativeUsersScope
  query: NativeUsersQuery
  selected: ReadonlySet<string>
  onSelect: (id: string, checked: boolean) => void
  onSelectPage: (checked: boolean) => void
  onSort: (sort: NativeUsersSort) => void
  onUpdated: () => void
  onAccessDenied: () => void
}

function UserRecord({
  user,
  scope,
  checked,
  onSelect,
  onUpdated,
  onAccessDenied,
}: Pick<Props, 'scope' | 'onSelect' | 'onUpdated' | 'onAccessDenied'> & {
  user: NativeUser
  checked: boolean
}) {
  const { t, i18n } = useTranslation()
  const [expanded, setExpanded] = useState(false)
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
  const name = user.username || user.id
  const wallet = user.wallet
  const detailsId = `native-user-details-${user.id}`

  return (
    <tbody
      data-native-user-id={user.id}
      className='border-border/50 block border-b py-4 last:border-0 lg:table-row-group lg:border-0 lg:py-0'
    >
      <TableRow
        data-state={checked ? 'selected' : undefined}
        className='grid grid-cols-[1.75rem_minmax(0,1fr)_minmax(0,1fr)_2.75rem] items-start gap-x-2 gap-y-4 rounded-lg border-0 px-1 py-2 lg:table-row lg:rounded-none lg:border-b lg:px-0 lg:py-0'
      >
        <TableCell className='col-start-1 row-start-1 px-1 py-2 lg:w-12 lg:px-4 lg:py-5'>
          <Checkbox
            checked={checked}
            onCheckedChange={(value) => onSelect(user.id, value)}
            aria-label={t('Select user {{name}}', { name })}
          />
        </TableCell>
        <TableCell className='col-span-2 col-start-2 row-start-1 min-w-0 p-0 whitespace-normal lg:px-3 lg:py-5'>
          <div className='min-w-0 space-y-1'>
            <p className='font-medium break-all'>
              <EntityQuickActions
                kind='username'
                value={user.username ?? ''}
                label={profile(user.username)}
              />
            </p>
            <p className='text-muted-foreground break-all'>
              {profile(user.email)}
            </p>
            <p className='text-muted-foreground font-mono text-xs break-all'>
              <span className='sr-only'>{t('User ID')}: </span>#{user.id}
            </p>
          </div>
        </TableCell>
        <TableCell className='col-start-2 row-start-3 min-w-0 p-0 whitespace-normal lg:px-3 lg:py-5'>
          <p className='text-muted-foreground mb-1 text-xs lg:hidden'>
            {t('Groups')}
          </p>
          <div className='space-y-1 break-all'>
            {user.groups.length === 0 ? (
              <span className='text-muted-foreground'>
                {t('No explicit group memberships')}
              </span>
            ) : (
              <p>{user.groups.join(' · ')}</p>
            )}
            <p className='text-muted-foreground text-xs'>
              {t('Service level')}: {user.service_level}
            </p>
          </div>
        </TableCell>
        <TableCell className='col-span-2 col-start-2 row-start-2 min-w-0 p-0 whitespace-normal lg:px-3 lg:py-5'>
          <div className='flex flex-wrap items-center gap-x-4 gap-y-1 lg:block lg:space-y-1'>
            <p
              className={`flex items-center gap-1.5 ${user.active ? '' : 'text-destructive'}`}
            >
              {user.active ? (
                <CircleCheck
                  className='text-success size-3.5'
                  aria-hidden='true'
                />
              ) : (
                <CircleOff className='size-3.5' aria-hidden='true' />
              )}
              {user.active ? t('Active user') : t('Disabled user')}
            </p>
            <p className='text-muted-foreground text-xs'>{role}</p>
          </div>
        </TableCell>
        <TableCell className='col-span-2 col-start-3 row-start-3 min-w-0 p-0 whitespace-normal lg:px-3 lg:py-5 lg:text-right'>
          <p className='text-muted-foreground mb-1 text-xs lg:hidden'>
            {t('Spendable wallet')}
          </p>
          <p className='font-mono font-medium break-all'>
            {wallet.status === 'available'
              ? wallet.wallet_units
              : t('Unavailable')}
          </p>
          {wallet.status === 'available' && (
            <p className='text-muted-foreground mt-1 text-xs'>
              {t('Core credits')}
            </p>
          )}
        </TableCell>
        <TableCell className='col-start-4 row-start-1 p-0 lg:px-2 lg:py-5'>
          <Button
            variant='ghost'
            size='icon'
            type='button'
            className='min-h-11 min-w-11'
            aria-label={t('User details')}
            aria-expanded={expanded}
            aria-controls={detailsId}
            onClick={() => setExpanded((value) => !value)}
          >
            <ChevronDown
              className={expanded ? 'rotate-180' : undefined}
              aria-hidden='true'
            />
          </Button>
        </TableCell>
      </TableRow>
      <tr className={expanded ? 'block lg:table-row' : 'hidden'}>
        <td colSpan={6} className='block px-3 pt-3 pb-6 lg:table-cell lg:px-15'>
          <div id={detailsId} className='space-y-6'>
            <div className='flex flex-wrap gap-2 [&_button]:min-h-11 sm:[&_button]:min-h-9'>
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
                      onUpdated={onUpdated}
                      onAccessDenied={onAccessDenied}
                    />
                  )}
                </>
              )}
            </div>
            <div className='grid gap-x-8 gap-y-6 sm:grid-cols-2 xl:grid-cols-4'>
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
                    {t('Developer access')}
                  </dt>
                  <dd className='mt-1'>
                    {user.developer_access_granted
                      ? t('Granted')
                      : t('Not granted')}
                  </dd>
                </div>
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

export function NativeUserList({
  users,
  query,
  selected,
  onSelectPage,
  onSort,
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
    <div>
      <label className='text-muted-foreground flex min-h-11 items-center gap-3 px-2 text-sm lg:hidden'>
        <Checkbox
          checked={allSelected}
          indeterminate={someSelected && !allSelected}
          onCheckedChange={onSelectPage}
        />
        {t('Select current page')}
      </label>
      <Table className='block table-fixed lg:table'>
        <TableHeader className='hidden lg:table-header-group'>
          <TableRow className='hover:bg-transparent'>
            <TableHead className='w-12'>
              <Checkbox
                checked={allSelected}
                indeterminate={someSelected && !allSelected}
                onCheckedChange={onSelectPage}
                aria-label={t('Select current page')}
              />
            </TableHead>
            <TableHead className='w-[30%] px-3' aria-sort={sort('username')}>
              {sortHeading('username', t('Username'))}
            </TableHead>
            <TableHead className='w-[23%] px-3' aria-sort={sort('groups')}>
              {sortHeading('groups', t('Groups'))}
            </TableHead>
            <TableHead className='w-[20%] px-3' aria-sort={sort('status')}>
              {sortHeading('status', t('Account'))}
            </TableHead>
            <TableHead className='px-3 text-right whitespace-normal'>
              {t('Spendable wallet')}
            </TableHead>
            <TableHead className='w-14'>
              <span className='sr-only'>{t('User details')}</span>
            </TableHead>
          </TableRow>
        </TableHeader>
        {users.map((user) => (
          <UserRecord
            key={user.id}
            user={user}
            checked={selected.has(user.id)}
            {...props}
          />
        ))}
      </Table>
    </div>
  )
}
