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
import { useQuery } from '@tanstack/react-query'
import {
  CalendarDays,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Sparkles,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Dialog } from '@/components/dialog'
import { Turnstile } from '@/components/turnstile'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { IconBadge } from '@/components/ui/icon-badge'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  TooltipProvider,
} from '@/components/ui/tooltip'
import { useWalletCurrency } from '@/hooks/use-wallet-currency'
import { toIntlLocale } from '@/i18n/languages'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

import { getCheckinStatus, performCheckin } from '../api'
import type { CheckinResponse } from '../checkin-schema'

interface CheckinCalendarCardProps {
  checkinEnabled: boolean
  turnstileEnabled: boolean
  turnstileSiteKey: string
}

export function CheckinCalendarCard(props: CheckinCalendarCardProps) {
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  return (
    <CheckinCalendar key={JSON.stringify([userId, sessionId])} {...props} />
  )
}

function CheckinCalendar({
  checkinEnabled,
  turnstileEnabled,
  turnstileSiteKey,
}: CheckinCalendarCardProps) {
  const { t, i18n } = useTranslation()
  const { formatExactQuota: formatQuota } = useWalletCurrency()
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const [selectedMonth, setSelectedMonth] = useState<string>()
  const [claim, setClaim] = useState<CheckinResponse | null>(null)
  const pendingOperation = useRef<{ day: string; key: string } | null>(null)
  const inFlight = useRef(false)
  const [checkinLoading, setCheckinLoading] = useState(false)
  const [turnstileModalVisible, setTurnstileModalVisible] = useState(false)
  const [turnstileWidgetKey, setTurnstileWidgetKey] = useState(0)
  const [initialLoaded, setInitialLoaded] = useState(false)
  const [collapsed, setCollapsed] = useState<boolean>(false)

  const intlLocale = toIntlLocale(i18n.resolvedLanguage || i18n.language)
  const calendarFormatters = useMemo(() => {
    const month = new Intl.DateTimeFormat(intlLocale, {
      month: 'long',
      year: 'numeric',
      timeZone: 'UTC',
    })
    const date = new Intl.DateTimeFormat(intlLocale, {
      day: 'numeric',
      month: 'long',
      year: 'numeric',
      timeZone: 'UTC',
    })
    const weekdayShort = new Intl.DateTimeFormat(intlLocale, {
      weekday: 'short',
      timeZone: 'UTC',
    })
    const weekdayLong = new Intl.DateTimeFormat(intlLocale, {
      weekday: 'long',
      timeZone: 'UTC',
    })

    return {
      date,
      month,
      weekDays: Array.from({ length: 7 }, (_, index) => {
        const referenceDate = new Date(Date.UTC(2024, 0, 7 + index))
        return {
          long: weekdayLong.format(referenceDate),
          short: weekdayShort.format(referenceDate),
        }
      }),
    }
  }, [intlLocale])

  // Fetch checkin status
  /* eslint-disable @tanstack/query/exhaustive-deps */
  const {
    data: checkinData,
    isLoading,
    isError,
    refetch,
  } = useQuery({
    queryKey: ['checkin-status', userId, sessionId, selectedMonth],
    queryFn: async () => {
      const res = await getCheckinStatus(selectedMonth)
      if (res.success) {
        return res.data
      }
      throw new Error(res.message || t('Failed to fetch checkin status'))
    },
    enabled: checkinEnabled,
    staleTime: 30000,
    refetchInterval: 60000,
  })
  /* eslint-enable @tanstack/query/exhaustive-deps */

  // Native days and the default month come from the server. The original Go
  // response has no calendar fields, so its existing browser fallback remains.
  const browserToday = new Date()
  const legacyToday = `${browserToday.getFullYear()}-${String(
    browserToday.getMonth() + 1
  ).padStart(2, '0')}-${String(browserToday.getDate()).padStart(2, '0')}`
  const serverToday =
    checkinData?.kind === 'native' ? checkinData.today : legacyToday
  const claimDay =
    claim?.kind === 'native' ? claim.record.checkin_date : claim?.checkin_date
  const todayString =
    claimDay && claimDay > serverToday ? claimDay : serverToday
  const defaultMonth =
    checkinData?.kind === 'native' && todayString === checkinData.today
      ? checkinData.month
      : todayString.slice(0, 7)
  const currentMonthStr = selectedMonth ?? defaultMonth
  const currentMonth = useMemo(
    () => new Date(`${currentMonthStr}-01T00:00:00Z`),
    [currentMonthStr]
  )
  const latestMonth = todayString.slice(0, 7)
  const canGoNext = currentMonthStr < latestMonth
  const monthLabel = calendarFormatters.month.format(currentMonth)
  const rewardRange = checkinData?.reward_range
  const claimedToday = claimDay === todayString
  const checkedToday =
    claimedToday || checkinData?.stats.checked_in_today === true
  const records = (checkinData?.stats.records ?? []).map(
    ({ checkin_date, quota_awarded }) => ({ checkin_date, quota_awarded })
  )
  const claimAmount =
    claim?.kind === 'native' ? claim.record.quota_awarded : claim?.quota_awarded
  if (
    claimDay?.startsWith(currentMonthStr) &&
    claimAmount !== undefined &&
    !records.some((record) => record.checkin_date === claimDay)
  ) {
    records.push({ checkin_date: claimDay, quota_awarded: claimAmount })
  }
  const checkinRecordsMap = Object.fromEntries(
    records.map((record) => [record.checkin_date, record.quota_awarded])
  )
  const monthlyQuota = records
    .reduce((sum, record) => sum + BigInt(record.quota_awarded), 0n)
    .toString()
  const todayAward = claimedToday
    ? claimAmount
    : checkinData?.kind === 'native'
      ? checkinData.today_record?.quota_awarded
      : checkinRecordsMap[todayString]

  useEffect(() => {
    if (initialLoaded) return
    if (isLoading) return
    if (!checkinData) return
    setCollapsed(checkedToday)
    setInitialLoaded(true)
  }, [checkinData, checkedToday, initialLoaded, isLoading])

  const shouldTriggerTurnstile = useCallback(
    (message?: string) => {
      if (!turnstileEnabled) return false
      if (typeof message !== 'string') return true
      return message.includes('Turnstile')
    },
    [turnstileEnabled]
  )

  const doCheckin = useCallback(
    async (token?: string) => {
      if (!token && turnstileEnabled) {
        if (!turnstileSiteKey) {
          toast.error(t('Turnstile is enabled but site key is empty.'))
          return
        }
        setTurnstileModalVisible(true)
        return
      }

      if (inFlight.current) return
      inFlight.current = true
      setCheckinLoading(true)
      try {
        let operationKey: string | undefined
        if (checkinData?.kind === 'native') {
          if (pendingOperation.current?.day !== todayString) {
            pendingOperation.current = {
              day: todayString,
              key: crypto.randomUUID(),
            }
          }
          operationKey = pendingOperation.current.key
        }
        const res = await performCheckin(token, operationKey)
        if (res.success) {
          setClaim(
            res.data.kind === 'legacy'
              ? {
                  ...res.data,
                  checkin_date: res.data.checkin_date ?? todayString,
                }
              : res.data
          )
          pendingOperation.current = null
          const amount =
            res.data.kind === 'native'
              ? res.data.record.quota_awarded
              : res.data.quota_awarded
          toast.success(
            res.data.kind === 'native' && res.data.already_claimed
              ? t('Checked in')
              : `${t('Check-in successful! Received')} ${formatQuota(amount)}`
          )
          void refetch()
          setTurnstileModalVisible(false)
        } else {
          if (!token && shouldTriggerTurnstile(res.message)) {
            if (!turnstileSiteKey) {
              toast.error(t('Turnstile is enabled but site key is empty.'))
              return
            }
            setTurnstileModalVisible(true)
            return
          }
          if (token && shouldTriggerTurnstile(res.message)) {
            setTurnstileWidgetKey((v) => v + 1)
          }
          toast.error(res.message || t('Check-in failed'))
        }
      } catch {
        void refetch()
        toast.error(t('Check-in failed'))
      } finally {
        inFlight.current = false
        setCheckinLoading(false)
      }
    },
    [
      refetch,
      shouldTriggerTurnstile,
      t,
      turnstileEnabled,
      turnstileSiteKey,
      formatQuota,
      checkinData?.kind,
      todayString,
    ]
  )

  const handleCheckinClick = useCallback(() => {
    if (turnstileEnabled) {
      if (!turnstileSiteKey) {
        toast.error(t('Turnstile is enabled but site key is empty.'))
        return
      }
      setTurnstileModalVisible(true)
      return
    }
    void doCheckin()
  }, [doCheckin, t, turnstileEnabled, turnstileSiteKey])

  const handlePrevMonth = () => {
    const previous = new Date(currentMonth)
    previous.setUTCMonth(previous.getUTCMonth() - 1)
    setSelectedMonth(previous.toISOString().slice(0, 7))
  }

  const handleNextMonth = () => {
    const next = new Date(currentMonth)
    next.setUTCMonth(next.getUTCMonth() + 1)
    const month = next.toISOString().slice(0, 7)
    if (month <= latestMonth) setSelectedMonth(month)
  }

  // Build calendar grid
  const calendarDays = useMemo(() => {
    const year = currentMonth.getUTCFullYear()
    const month = currentMonth.getUTCMonth()
    const firstDay = new Date(Date.UTC(year, month, 1))
    const lastDay = new Date(Date.UTC(year, month + 1, 0))
    const daysInMonth = lastDay.getUTCDate()
    const startDayOfWeek = firstDay.getUTCDay() // 0 = Sunday

    const days: Array<{ date: Date; isCurrentMonth: boolean }> = []

    // Fill leading empty days
    for (let i = 0; i < startDayOfWeek; i++) {
      const d = new Date(Date.UTC(year, month, -startDayOfWeek + i + 1))
      days.push({ date: d, isCurrentMonth: false })
    }

    // Fill current month days
    for (let i = 1; i <= daysInMonth; i++) {
      days.push({
        date: new Date(Date.UTC(year, month, i)),
        isCurrentMonth: true,
      })
    }

    // Fill trailing empty days to complete the grid
    const remaining = 7 - (days.length % 7)
    if (remaining < 7) {
      for (let i = 1; i <= remaining; i++) {
        days.push({
          date: new Date(Date.UTC(year, month + 1, i)),
          isCurrentMonth: false,
        })
      }
    }

    return days
  }, [currentMonth])

  if (!checkinEnabled) {
    return null
  }

  if (isLoading) {
    return (
      <Card data-card-hover='false' className='gap-0 overflow-hidden py-0'>
        <div className='p-6'>
          <div className='flex items-start justify-between gap-4'>
            <div className='flex items-center gap-3'>
              <Skeleton className='h-10 w-10 rounded-xl' />
              <div className='space-y-2'>
                <Skeleton className='h-5 w-32' />
                <Skeleton className='h-3 w-56' />
              </div>
            </div>
            <Skeleton className='h-9 w-28 rounded-md' />
          </div>
        </div>
      </Card>
    )
  }

  let checkinButtonLabel = t('Check in now')
  if (checkinLoading) {
    checkinButtonLabel = t('Loading...')
  } else if (checkedToday) {
    checkinButtonLabel = t('Checked in')
  }

  return (
    <TooltipProvider delay={100}>
      <Dialog
        open={turnstileModalVisible}
        onOpenChange={(open) => {
          setTurnstileModalVisible(open)
          if (!open) {
            setTurnstileWidgetKey((v) => v + 1)
          }
        }}
        title={t('Security Check')}
        contentClassName='sm:max-w-md'
        contentHeight='auto'
        bodyClassName='space-y-4'
      >
        <div className='text-muted-foreground text-sm'>
          {t('Please complete the security check to continue.')}
        </div>
        <div className='flex justify-center py-4'>
          <Turnstile
            key={turnstileWidgetKey}
            siteKey={turnstileSiteKey}
            onVerify={(token) => {
              const normalizedToken = token.trim()
              if (normalizedToken) void doCheckin(normalizedToken)
            }}
            onExpire={() => {
              setTurnstileWidgetKey((v) => v + 1)
            }}
          />
        </div>
      </Dialog>

      <Card data-card-hover='false' className='gap-0 overflow-hidden py-0'>
        {/* Header */}
        <div className='border-b p-4 sm:p-6'>
          <div className='flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4'>
            <button
              type='button'
              aria-expanded={!collapsed}
              className='focus-visible:ring-ring/50 flex min-w-0 flex-1 items-start gap-3 rounded-lg text-left whitespace-normal outline-none focus-visible:ring-3'
              onClick={() => setCollapsed((v) => !v)}
            >
              <IconBadge tone='neutral' size='lg' className='sm:size-11'>
                <CalendarDays
                  className='h-4 w-4 sm:h-5 sm:w-5'
                  strokeWidth={2}
                />
              </IconBadge>
              <div className='min-w-0 flex-1'>
                <div className='flex flex-wrap items-center gap-1.5 sm:gap-2'>
                  <h3 className='text-base font-semibold tracking-tight sm:text-lg'>
                    {t('Daily Check-in')}
                  </h3>
                  {checkedToday && (
                    <div className='console-status-success-badge inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-medium sm:gap-1.5 sm:px-2.5 sm:text-xs'>
                      <Sparkles className='h-2.5 w-2.5 sm:h-3 sm:w-3' />
                      {t('Checked in')}
                    </div>
                  )}
                  <span className='text-muted-foreground inline-flex items-center'>
                    {collapsed ? (
                      <ChevronDown className='h-4 w-4' />
                    ) : (
                      <ChevronUp className='h-4 w-4' />
                    )}
                  </span>
                </div>
                <p className='text-muted-foreground mt-1 line-clamp-2 text-xs sm:text-sm'>
                  {checkedToday && todayAward !== undefined
                    ? `${t('Today')} +${formatQuota(todayAward)}`
                    : t('Check in daily to receive random quota rewards')}
                </p>
                {rewardRange ? (
                  <p className='text-muted-foreground mt-1 text-[11px] sm:text-xs'>
                    {t('Reward range for level {{level}}: {{min}}–{{max}}', {
                      level: rewardRange.trust_level,
                      min: formatQuota(rewardRange.min_quota),
                      max: formatQuota(rewardRange.max_quota),
                    })}
                  </p>
                ) : null}
              </div>
            </button>
            <Button
              onClick={handleCheckinClick}
              disabled={
                checkinLoading ||
                checkedToday ||
                !checkinData?.enabled ||
                isError
              }
              size='sm'
              className='h-11 w-full shrink-0 sm:h-7 sm:w-auto sm:min-w-24'
            >
              {checkinLoading ? <Spinner data-icon='inline-start' /> : null}
              {checkinButtonLabel}
            </Button>
          </div>
        </div>

        {isError ? (
          <div role='alert' className='p-4 text-sm'>
            <p>{t('Failed to fetch checkin status')}</p>
            <Button variant='outline' onClick={() => void refetch()}>
              {t('Retry')}
            </Button>
          </div>
        ) : null}

        {!collapsed ? (
          <>
            {/* Stats */}
            <div className='grid grid-cols-3 gap-px border-b'>
              <div className='bg-card p-3 text-center sm:p-5'>
                <div className='text-base leading-tight font-semibold tracking-tight whitespace-nowrap tabular-nums sm:text-xl xl:text-base 2xl:text-lg'>
                  {checkinData?.stats?.total_checkins || 0}
                </div>
                <div className='text-muted-foreground mt-0.5 text-[10px] font-medium sm:mt-1 sm:text-xs'>
                  {t('Total check-ins')}
                </div>
              </div>
              <div className='bg-card p-3 text-center sm:p-5'>
                <div className='text-base leading-tight font-semibold tracking-tight whitespace-nowrap tabular-nums sm:text-xl xl:text-base 2xl:text-lg'>
                  {formatQuota(monthlyQuota)}
                </div>
                <div className='text-muted-foreground mt-0.5 text-[10px] font-medium sm:mt-1 sm:text-xs'>
                  {t('This month')}
                </div>
              </div>
              <div className='bg-card p-3 text-center sm:p-5'>
                <div className='text-base leading-tight font-semibold tracking-tight whitespace-nowrap tabular-nums sm:text-xl xl:text-base 2xl:text-lg'>
                  {formatQuota(checkinData?.stats.total_quota ?? '0')}
                </div>
                <div className='text-muted-foreground mt-0.5 text-[10px] font-medium sm:mt-1 sm:text-xs'>
                  {t('Total earned')}
                </div>
              </div>
            </div>

            {/* Calendar */}
            <div className='p-4 sm:p-6'>
              <div className='space-y-3 sm:space-y-4'>
                {/* Month navigation */}
                <div className='flex items-center justify-between'>
                  <h4 className='text-xs font-semibold sm:text-sm'>
                    {monthLabel}
                  </h4>
                  <div className='flex items-center gap-0.5 sm:gap-1'>
                    <Button
                      aria-label={t('Previous month')}
                      variant='ghost'
                      size='icon'
                      className='size-11 sm:size-8'
                      onClick={handlePrevMonth}
                    >
                      <ChevronLeft />
                    </Button>
                    <Button
                      aria-label={t('Next month')}
                      variant='ghost'
                      size='icon'
                      className='size-11 sm:size-8'
                      onClick={handleNextMonth}
                      disabled={!canGoNext}
                    >
                      <ChevronRight />
                    </Button>
                  </div>
                </div>

                {/* Calendar grid */}
                <div
                  role='grid'
                  aria-label={monthLabel}
                  className='grid grid-cols-7 gap-0.5 sm:gap-1'
                >
                  {/* Week day headers */}
                  {calendarFormatters.weekDays.map((day) => (
                    <div
                      key={day.long}
                      role='columnheader'
                      aria-label={day.long}
                      className='text-muted-foreground flex h-7 items-center justify-center text-[10px] font-medium sm:h-8 sm:text-xs'
                    >
                      {day.short}
                    </div>
                  ))}

                  {/* Calendar days */}
                  {calendarDays.map((dayObj) => {
                    const dateStr = `${dayObj.date.getUTCFullYear()}-${String(
                      dayObj.date.getUTCMonth() + 1
                    ).padStart(2, '0')}-${String(
                      dayObj.date.getUTCDate()
                    ).padStart(2, '0')}`
                    const isToday = dateStr === todayString
                    const quotaAwarded = checkinRecordsMap[dateStr]
                    const isCheckedIn = quotaAwarded !== undefined
                    const dayNum = dayObj.date.getUTCDate()
                    const dateLabel = calendarFormatters.date.format(
                      dayObj.date
                    )
                    const formattedAward = isCheckedIn
                      ? formatQuota(quotaAwarded)
                      : undefined
                    const accessibleLabel = dayObj.isCurrentMonth
                      ? [
                          dateLabel,
                          isCheckedIn ? t('Checked in') : null,
                          formattedAward ? `+${formattedAward}` : null,
                        ]
                          .filter(Boolean)
                          .join(', ')
                      : undefined

                    const dayCell = (
                      <div
                        key={dateStr}
                        role='gridcell'
                        aria-current={isToday ? 'date' : undefined}
                        aria-hidden={dayObj.isCurrentMonth ? undefined : true}
                        aria-label={accessibleLabel}
                        tabIndex={
                          isCheckedIn && dayObj.isCurrentMonth ? 0 : undefined
                        }
                        className={cn(
                          'relative flex h-9 w-full items-center justify-center rounded-lg px-0 text-xs font-medium outline-none sm:h-10 sm:text-sm',
                          !dayObj.isCurrentMonth &&
                            'text-muted-foreground/40 cursor-default',
                          isToday && 'bg-primary text-primary-foreground',
                          !isToday && isCheckedIn && 'font-semibold',
                          isCheckedIn &&
                            dayObj.isCurrentMonth &&
                            'cursor-help transition-colors hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/50',
                          isToday && isCheckedIn && 'hover:bg-primary/90'
                        )}
                      >
                        <span className='tabular-nums'>{dayNum}</span>
                        {isCheckedIn && !isToday && (
                          <span
                            aria-hidden='true'
                            className='bg-success absolute bottom-0.5 size-1 rounded-full sm:bottom-1'
                          />
                        )}
                      </div>
                    )

                    if (isCheckedIn && dayObj.isCurrentMonth) {
                      return (
                        <Tooltip key={dateStr}>
                          <TooltipTrigger render={dayCell} />
                          <TooltipContent>
                            <div className='text-xs'>
                              <div className='font-medium'>
                                {t('Checked in')}
                              </div>
                              <div className='text-muted-foreground mt-0.5'>
                                +{formattedAward}
                              </div>
                            </div>
                          </TooltipContent>
                        </Tooltip>
                      )
                    }

                    return dayCell
                  })}
                </div>

                {/* Footer hint */}
                <div className='text-muted-foreground border-t pt-3 text-center text-[11px] sm:pt-4 sm:text-xs'>
                  {t('You can only check in once per day')}
                </div>

                <div className='bg-muted/30 text-muted-foreground rounded-lg border p-3 text-xs'>
                  <ul className='list-disc space-y-1 pl-5'>
                    <li>
                      {t('Check in daily to receive random quota rewards')}
                    </li>
                    <li>
                      {t('Rewards will be added directly to your balance')}
                    </li>
                    <li>{t('Do not repeat check-in; only once per day')}</li>
                  </ul>
                </div>
              </div>
            </div>
          </>
        ) : null}
      </Card>
    </TooltipProvider>
  )
}
