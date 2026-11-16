/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
/*
Copyright (C) 2026 LIghtJUNction
*/
import { Link } from '@tanstack/react-router'
import { Loader2, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  isCurrentNativeUsersScope,
  NativeUsersFailure,
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { readNativeUser } from '@/features/native-users/api'
import {
  DEFAULT_NATIVE_USERS_QUERY,
  type NativeUserDetail,
} from '@/features/native-users/contract'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import { UserStatistics } from './user-statistics'

interface UserInfoDialogProps {
  userId: number | null
  open: boolean
  onOpenChange: (open: boolean) => void
}

type Load =
  | { status: 'loading' }
  | { status: 'ready'; detail: NativeUserDetail }
  | { status: 'error'; kind: NativeUsersFailure['kind'] }

function InfoItem({ label, value }: { label: string; value: string | number }) {
  return (
    <div className='min-w-0 space-y-1.5'>
      <dt className='text-muted-foreground text-xs'>{label}</dt>
      <dd className='text-sm font-medium break-words'>{value}</dd>
    </div>
  )
}

function UserSnapshot({
  userId,
  scope,
}: {
  userId: string
  scope: NativeUsersScope
}) {
  const { t, i18n } = useTranslation()
  const [state, setState] = useState<Load>({ status: 'loading' })
  const [reload, setReload] = useState(0)
  const { userId: actorId, sessionId, role } = scope

  useEffect(() => {
    const controller = new AbortController()
    const captured = { userId: actorId, sessionId, role }
    const owns = () =>
      !controller.signal.aborted && isCurrentNativeUsersScope(captured)
    void readNativeUser(captured, userId, controller.signal).then(
      (detail) => {
        if (owns()) setState({ status: 'ready', detail })
      },
      (error: unknown) => {
        if (owns()) {
          setState({
            status: 'error',
            kind:
              error instanceof NativeUsersFailure ? error.kind : 'unavailable',
          })
        }
      }
    )
    return () => controller.abort()
  }, [actorId, sessionId, role, userId, reload])

  const refresh = () => {
    setState({ status: 'loading' })
    setReload((value) => value + 1)
  }

  if (!isCurrentNativeUsersScope(scope)) return null
  if (state.status === 'loading') {
    return (
      <div
        className='text-muted-foreground flex items-center justify-center gap-3 py-12'
        role='status'
      >
        <Loader2 className='size-5 animate-spin' aria-hidden='true' />
        <span>{t('Loading users...')}</span>
      </div>
    )
  }
  if (state.status === 'error') {
    return (
      <div className='space-y-4 py-4'>
        <Alert role='alert'>
          <AlertDescription>
            {state.kind === 'forbidden'
              ? t(
                  'Administrator access or multi-factor verification is required.'
                )
              : t('Failed to fetch user information')}
          </AlertDescription>
        </Alert>
        <Button type='button' variant='outline' onClick={refresh}>
          {t('Retry')}
        </Button>
      </div>
    )
  }

  const { user, usage, statistics } = state.detail
  const number = (value: string) =>
    BigInt(value).toLocaleString(toIntlLocale(i18n.language))
  const wallet = user.wallet
  const profile = (value: string | null) =>
    value === null ? t('Unavailable') : value === '' ? t('Empty value') : value
  const date = (value: number | null) =>
    value === null
      ? t('Unavailable')
      : new Intl.DateTimeFormat(toIntlLocale(i18n.language), {
          dateStyle: 'medium',
          timeStyle: 'short',
        }).format(value)
  const accountRole = {
    user: t('User'),
    admin: t('Administrator'),
    superadmin: t('Super administrator'),
  }[user.platform_role]

  return (
    <div className='space-y-7 py-3'>
      <div className='min-w-0 space-y-1'>
        <p className='text-xl font-semibold tracking-tight break-all'>
          {profile(user.username)}
        </p>
        <p className='text-muted-foreground text-sm break-all'>
          {profile(user.email)}
        </p>
      </div>

      <section
        className='bg-muted/40 space-y-4 rounded-xl p-5'
        aria-label={t('Wallet (Core credits)')}
      >
        <h3 className='text-sm font-medium'>{t('Wallet (Core credits)')}</h3>
        {wallet.status === 'available' ? (
          <dl className='grid grid-cols-1 gap-x-6 gap-y-5 min-[380px]:grid-cols-2'>
            {[
              [t('Spendable wallet'), wallet.wallet_units],
              [t('Economic position'), wallet.available_units],
              [t('Reserved funds'), wallet.reserved_units],
              [t('Debt magnitude'), wallet.debt_units],
            ].map(([label, value]) => (
              <div key={label} className='min-w-0 space-y-1.5'>
                <dt className='text-muted-foreground text-xs'>{label}</dt>
                <dd className='font-mono text-xl font-medium break-all tabular-nums'>
                  {value}
                </dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className='text-muted-foreground text-sm'>{t('Unavailable')}</p>
        )}
        <p className='text-muted-foreground text-xs leading-relaxed'>
          {t(
            'Currency and money basis are unavailable. Amounts are exact Core credit units.'
          )}
        </p>
      </section>

      <section
        className='space-y-4'
        aria-label={t('Personal account usage')}
        data-testid='personal-user-usage'
      >
        <h3 className='text-sm font-medium'>{t('Personal account usage')}</h3>
        {usage.status === 'available' ? (
          <>
            <dl className='grid grid-cols-1 gap-x-6 gap-y-5 min-[380px]:grid-cols-2'>
              <div className='min-w-0 space-y-1.5 min-[380px]:col-span-2'>
                <dt className='text-muted-foreground text-xs'>
                  {t('Net consumption')}
                </dt>
                <dd className='flex flex-wrap items-baseline gap-x-2 gap-y-1'>
                  <span className='font-mono text-2xl font-medium break-all tabular-nums'>
                    {number(usage.net_consumption_units)}
                  </span>
                  <span className='text-muted-foreground text-xs'>
                    {t('Core credits')}
                  </span>
                </dd>
              </div>
              <InfoItem
                label={t('Settled requests')}
                value={number(usage.settled_requests)}
              />
              <InfoItem
                label={t('Pending settlement')}
                value={number(usage.pending_requests)}
              />
            </dl>
            <details className='text-sm'>
              <summary className='text-muted-foreground focus-visible:outline-ring min-h-11 cursor-pointer py-3 break-words focus-visible:outline-2'>
                {t('Usage breakdown')}
              </summary>
              <dl className='grid grid-cols-1 gap-x-6 gap-y-4 py-3 min-[380px]:grid-cols-2'>
                {[
                  [
                    t('Personal wallet consumption'),
                    usage.personal_wallet_consumption_units,
                  ],
                  [
                    t('Team wallet consumption'),
                    usage.team_wallet_consumption_units,
                  ],
                  [
                    t('Subscription consumption'),
                    usage.subscription_consumption_units,
                  ],
                  [
                    t('Sponsored consumption'),
                    usage.sponsored_consumption_units,
                  ],
                  [t('Gross consumption'), usage.gross_settled_units],
                  [t('Refunded consumption'), usage.refunded_units],
                ].map(([label, value]) => (
                  <InfoItem key={label} label={label} value={number(value)} />
                ))}
              </dl>
            </details>
          </>
        ) : (
          <p className='text-muted-foreground text-sm'>{t('Unavailable')}</p>
        )}
        <p className='text-muted-foreground text-xs leading-relaxed'>
          {t(
            'Personal-account requests may be paid by a personal wallet, team, subscription or sponsor. Imported history and team-owned API keys are excluded.'
          )}
        </p>
      </section>

      <UserStatistics statistics={statistics} />

      <dl className='grid grid-cols-1 gap-x-8 gap-y-5 sm:grid-cols-2'>
        <InfoItem
          label={t('Display name')}
          value={profile(user.display_name)}
        />
        <InfoItem label={t('Role')} value={accountRole} />
        <InfoItem
          label={t('User status')}
          value={user.active ? t('Active user') : t('Disabled user')}
        />
        <InfoItem
          label={t('Account status')}
          value={user.account_active ? t('Active') : t('Inactive')}
        />
        <InfoItem label={t('Service level')} value={user.service_level} />
        <InfoItem
          label={t('Developer access')}
          value={
            user.developer_access_granted ? t('Granted') : t('Not granted')
          }
        />
        <InfoItem
          label={t('Explicit group memberships')}
          value={
            user.groups.length
              ? user.groups.join(' · ')
              : t('No explicit group memberships')
          }
        />
        <InfoItem
          label={t('Observed registration time')}
          value={date(user.registered_at_unix_ms)}
        />
      </dl>

      <details className='text-sm'>
        <summary className='text-muted-foreground focus-visible:outline-ring inline-flex min-h-11 cursor-pointer items-center focus-visible:outline-2'>
          {t('User details')}
        </summary>
        <dl className='grid grid-cols-1 gap-x-8 gap-y-5 py-3 sm:grid-cols-2'>
          <InfoItem label={t('User ID')} value={user.id} />
          <InfoItem
            label={t('Personal account ID')}
            value={user.personal_account_id}
          />
          <InfoItem
            label={t('Authentication version')}
            value={user.auth_version}
          />
          <InfoItem
            label={t('Core account created')}
            value={date(user.core_account_created_at_unix_ms)}
          />
        </dl>
      </details>

      <div className='flex flex-wrap gap-2'>
        <Button
          render={
            <Link
              to='/users'
              search={{ ...DEFAULT_NATIVE_USERS_QUERY, keyword: user.id }}
            />
          }
          className='h-auto min-h-11 max-w-full py-2 text-center break-words whitespace-normal'
        >
          {t('Open user management')}
        </Button>
        <Button
          type='button'
          variant='ghost'
          onClick={refresh}
          className='min-h-11'
        >
          <RefreshCw className='size-4' aria-hidden='true' />
          {t('Refresh')}
        </Button>
      </div>
    </div>
  )
}

export function UserInfoDialog({
  userId,
  open,
  onOpenChange,
}: UserInfoDialogProps) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  const validUserId =
    userId !== null && Number.isSafeInteger(userId) && userId > 0

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t('User Information')}
      description={t(
        "View this account's profile, access, wallet and recorded activity."
      )}
      contentClassName='sm:max-w-xl'
      contentHeight='auto'
    >
      {open && scope && validUserId ? (
        <UserSnapshot
          key={`${scope.userId}:${scope.sessionId}:${scope.role}:${userId}`}
          userId={String(userId)}
          scope={scope}
        />
      ) : open ? (
        <Alert role='alert'>
          <AlertDescription>
            {!scope
              ? t(
                  'Administrator access or multi-factor verification is required.'
                )
              : t('No user information available')}
          </AlertDescription>
        </Alert>
      ) : null}
    </Dialog>
  )
}
