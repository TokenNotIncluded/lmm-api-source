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
import { Share01Icon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { Link } from '@tanstack/react-router'
import { ArrowUpRight, KeyRound, ShieldCheck, Users } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button, buttonVariants } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { UserAvatar } from '@/components/user-avatar'
import { walletProjection } from '@/features/wallet/lib/wallet-projection'
import { formatCompactNumber, formatNumber } from '@/lib/format'
import { getRoleLabel } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { useProfileActivity } from '../hooks/use-profile-activity'
import { getDisplayName } from '../lib'
import {
  buildProfileDailyUsage,
  buildProfileUsageSummary,
  type ProfileDailyUsage,
} from '../lib/activity'
import type { UserProfile } from '../types'
import { ProfileActivity } from './profile-activity'
import { ProfileShareDialog } from './profile-share-dialog'

interface ProfileHeaderProps {
  profile: UserProfile | null
  loading: boolean
}
interface ProfileStat {
  label: string
  value: string
  loading?: boolean
}

function ProfileStatRail({ stats }: { stats: ProfileStat[] }) {
  return (
    <dl
      data-testid='profile-activity-stats'
      className='grid grid-cols-2 gap-x-5 gap-y-7 sm:grid-cols-3 xl:grid-cols-5'
    >
      {stats.map((stat) => (
        <div key={stat.label} className='min-w-0 space-y-2'>
          <dt className='text-muted-foreground text-xs leading-5'>
            {stat.label}
          </dt>
          <dd className='text-foreground text-xl font-medium break-words tabular-nums sm:text-2xl'>
            {stat.loading ? (
              <Skeleton className='h-7 w-20 rounded-md' />
            ) : (
              stat.value
            )}
          </dd>
        </div>
      ))}
    </dl>
  )
}

function ProfileOverviewSkeleton({ days }: { days: ProfileDailyUsage[] }) {
  const { t } = useTranslation()
  const stats = [
    'Tokens in the past year',
    'Peak daily tokens',
    'API Requests',
    'Current streak',
    'Longest streak this year',
  ].map((label) => ({ label: t(label), value: '', loading: true }))
  return (
    <section
      data-testid='profile-overview-skeleton'
      className='space-y-10 py-5 sm:py-7'
    >
      <div className='flex items-center gap-5'>
        <Skeleton className='size-20 shrink-0 rounded-full' />
        <div className='space-y-3'>
          <Skeleton className='h-8 w-48 max-w-full' />
          <Skeleton className='h-5 w-32' />
        </div>
      </div>
      <Skeleton className='h-44 w-full rounded-2xl' />
      <ProfileStatRail stats={stats} />
      <ProfileActivity
        days={days}
        activeDays={0}
        loading
        error={null}
        onRetry={() => undefined}
      />
    </section>
  )
}

export function ProfileHeader({ profile, loading }: ProfileHeaderProps) {
  const { t, i18n } = useTranslation()
  const canManageKeys = useAuthStore(
    (state) => state.auth.user?.permissions?.api_key_management === true
  )
  const [shareOpen, setShareOpen] = useState(false)
  const activityQuery = useProfileActivity(
    profile?.created_time,
    Boolean(profile) && !loading
  )
  const days = useMemo(
    () => buildProfileDailyUsage(activityQuery.data ?? [], activityQuery.range),
    [activityQuery.data, activityQuery.range]
  )
  const summary = useMemo(() => buildProfileUsageSummary(days), [days])
  if (loading) return <ProfileOverviewSkeleton days={days} />
  if (!profile) return null

  const locale = i18n.resolvedLanguage || i18n.language
  const displayName = getDisplayName(profile)
  const wallet = walletProjection(profile)
  const activityLoading = activityQuery.isPending && !activityQuery.data
  const activityError = activityQuery.data ? null : activityQuery.error
  const streakSuffix = summary.currentStreakCapped ? '+' : ''
  const requests =
    profile.usage_projection_available === true &&
    Number.isSafeInteger(profile.request_count) &&
    profile.request_count >= 0
      ? profile.request_count
      : null
  const stats: ProfileStat[] = [
    {
      label: t('Tokens in the past year'),
      value:
        summary.totalTokens === null
          ? t('Unavailable')
          : formatCompactNumber(summary.totalTokens, locale),
      loading: activityLoading,
    },
    {
      label: t('Peak daily tokens'),
      value:
        summary.peakDailyTokens === null
          ? t('Unavailable')
          : formatCompactNumber(summary.peakDailyTokens, locale),
      loading: activityLoading,
    },
    {
      label: t('API Requests'),
      value:
        requests === null
          ? t('Unavailable')
          : formatCompactNumber(requests, locale),
    },
    {
      label: t('Current streak'),
      value: `${formatNumber(summary.currentStreak, locale)}${streakSuffix} ${t('days')}`,
      loading: activityLoading,
    },
    {
      label: t('Longest streak this year'),
      value: `${formatNumber(summary.longestStreak, locale)} ${t('days')}`,
      loading: activityLoading,
    },
  ]
  if (activityError) for (const index of [0, 1, 3, 4]) stats[index].value = '—'

  return (
    <section
      data-testid='profile-overview'
      className='space-y-10 py-5 sm:space-y-12 sm:py-7'
      aria-labelledby='profile-display-name'
    >
      <div className='flex flex-col gap-6 sm:flex-row sm:items-center sm:justify-between'>
        <div className='flex min-w-0 items-center gap-4 sm:gap-5'>
          <UserAvatar
            name={profile.username || displayName}
            email={profile.email}
            alt={displayName}
            className='size-16 shrink-0 rounded-full sm:size-20'
            fallbackClassName='text-2xl'
            gravatarSize={160}
          />
          <div className='min-w-0 space-y-2'>
            <h1
              id='profile-display-name'
              className='text-foreground !font-sans text-2xl font-semibold tracking-tight break-words sm:text-3xl'
            >
              {displayName}
            </h1>
            <div className='text-muted-foreground flex flex-wrap items-center gap-x-3 gap-y-1 text-sm'>
              <span className='break-all'>@{profile.username}</span>
              <span>{getRoleLabel(profile.role)}</span>
              <span className='font-mono text-xs'>
                {t('User ID')}: {profile.id}
              </span>
            </div>
          </div>
        </div>
        <Button
          className='min-h-11 w-fit'
          render={<Link to='/profile' search={{ tab: 'account' }} />}
        >
          {t('Account & budget')}
          <ArrowUpRight className='size-4' aria-hidden='true' />
        </Button>
      </div>

      <div className='bg-card grid gap-7 rounded-2xl p-5 sm:p-7 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]'>
        <div className='min-w-0 space-y-4'>
          <div className='flex flex-wrap items-center justify-between gap-2'>
            <h2 className='text-sm font-medium'>{t('Personal wallet')}</h2>
            <span className='text-muted-foreground flex items-center gap-2 text-xs'>
              <span
                className={
                  profile.status === 1
                    ? 'bg-success size-1.5 rounded-full'
                    : 'bg-destructive size-1.5 rounded-full'
                }
                aria-hidden='true'
              />
              {profile.status === 1 ? t('Enabled') : t('Disabled')}
            </span>
          </div>
          <p
            data-testid='profile-wallet-balance'
            className='text-3xl font-medium tracking-tight break-words tabular-nums'
          >
            {wallet.available === null ? (
              t('Unavailable')
            ) : (
              <>
                {formatNumber(wallet.available, locale)}{' '}
                <span className='text-muted-foreground text-base font-normal'>
                  {t('Credits')}
                </span>
              </>
            )}
          </p>
          <p className='text-muted-foreground max-w-prose text-xs leading-relaxed'>
            {t(
              'Personal and team balances are managed separately. Set spending limits for each account.'
            )}
          </p>
          <Link
            to='/wallet'
            className={buttonVariants({
              variant: 'link',
              className: 'h-auto min-h-9 px-0',
            })}
          >
            {t('Open wallet')}
            <ArrowUpRight className='size-4' aria-hidden='true' />
          </Link>
        </div>
        <nav
          aria-label={t('Account shortcuts')}
          className='grid content-start gap-2 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2'
        >
          <Link
            to='/profile'
            search={{ tab: 'teams' }}
            className='hover:bg-muted focus-visible:outline-ring flex min-h-14 items-center gap-3 rounded-xl px-3 py-3 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-2'
          >
            <Users
              className='text-muted-foreground size-4 shrink-0'
              aria-hidden='true'
            />
            <span>{t('Teams')}</span>
            <ArrowUpRight
              className='text-muted-foreground ml-auto size-4 shrink-0'
              aria-hidden='true'
            />
          </Link>
          <Link
            to='/profile'
            search={{ tab: 'security' }}
            className='hover:bg-muted focus-visible:outline-ring flex min-h-14 items-center gap-3 rounded-xl px-3 py-3 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-2'
          >
            <ShieldCheck
              className='text-muted-foreground size-4 shrink-0'
              aria-hidden='true'
            />
            <span>{t('Security')}</span>
            <ArrowUpRight
              className='text-muted-foreground ml-auto size-4 shrink-0'
              aria-hidden='true'
            />
          </Link>
          {canManageKeys && (
            <Link
              to='/keys'
              className='hover:bg-muted focus-visible:outline-ring flex min-h-14 items-center gap-3 rounded-xl px-3 py-3 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-2'
            >
              <KeyRound
                className='text-muted-foreground size-4 shrink-0'
                aria-hidden='true'
              />
              <span>{t('API Keys')}</span>
              <ArrowUpRight
                className='text-muted-foreground ml-auto size-4 shrink-0'
                aria-hidden='true'
              />
            </Link>
          )}
          <Link
            to='/profile/share'
            className='hover:bg-muted focus-visible:outline-ring flex min-h-14 items-center gap-3 rounded-xl px-3 py-3 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-2'
          >
            <HugeiconsIcon
              icon={Share01Icon}
              size={16}
              className='text-muted-foreground shrink-0'
              aria-hidden='true'
            />
            <span>{t('Create GitHub badge')}</span>
            <ArrowUpRight
              className='text-muted-foreground ml-auto size-4 shrink-0'
              aria-hidden='true'
            />
          </Link>
        </nav>
      </div>

      <div className='space-y-6'>
        <header className='flex flex-wrap items-center justify-between gap-3'>
          <h2 className='text-base font-semibold'>{t('Your activity')}</h2>
          <Button variant='ghost' onClick={() => setShareOpen(true)}>
            <HugeiconsIcon icon={Share01Icon} strokeWidth={2} />
            {t('Share profile')}
          </Button>
        </header>
        <ProfileStatRail stats={stats} />
        {!activityLoading && !activityError && (
          <div className='text-muted-foreground space-y-1 text-xs leading-relaxed'>
            <p>
              {t('Images')}: {formatNumber(summary.totalImages, locale)}
              {' · '}
              {t('Platform sponsored requests')}:{' '}
              {formatNumber(summary.sponsoredRequests, locale)}
            </p>
            {summary.tokenCountScope === 'settled_text_requests' && (
              <p>{t('Tokens cover settled text requests only.')}</p>
            )}
          </div>
        )}
      </div>
      <ProfileActivity
        days={days}
        activeDays={summary.activeDays}
        loading={activityLoading}
        error={activityError}
        onRetry={() => void activityQuery.refetch()}
      />
      <ProfileShareDialog
        open={shareOpen}
        onOpenChange={setShareOpen}
        profile={profile}
        days={days}
        summary={summary}
        activityAvailable={Boolean(activityQuery.data)}
      />
    </section>
  )
}
