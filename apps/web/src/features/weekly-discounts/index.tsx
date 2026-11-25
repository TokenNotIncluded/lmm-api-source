/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Link } from '@tanstack/react-router'
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import { useStatus } from '@/hooks/use-status'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import {
  createWeeklyClient,
  type WeeklyDecisionDetail,
  type WeeklyDecisionSummary,
  type WeeklyHistory,
} from './api'
import { WeeklyDetailSession } from './detail-session'

const client = createWeeklyClient()
function ownerKey() {
  const { user, session } = useAuthStore.getState().auth
  return user && session?.sid ? String(user.id) + ':' + session.sid : null
}
function dateTime(stamp: number, language: string) {
  return new Intl.DateTimeFormat(toIntlLocale(language), {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZoneName: 'short',
  }).format(new Date(stamp * 1000))
}
function weekDate(stamp: number, language: string) {
  return new Intl.DateTimeFormat(toIntlLocale(language), {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    timeZone: 'UTC',
  }).format(new Date(stamp * 1000))
}
/** Deliberately construct the public fields; never lift a private code to history. */
function publicSummary(detail: WeeklyDecisionDetail): WeeklyDecisionSummary {
  return {
    id: detail.id,
    week_start: detail.week_start,
    expires_at: detail.expires_at,
    status: detail.status,
    discount_percent: detail.discount_percent,
    created_at: detail.created_at,
    claimed_at: detail.claimed_at,
    browser_path: detail.browser_path,
  }
}

export function WeeklyDiscounts({ selectedID }: { selectedID?: string }) {
  const auth = useAuthStore((state) => state.auth)
  const scope =
    auth.user && auth.session?.sid
      ? String(auth.user.id) + ':' + auth.session.sid
      : null
  const { t } = useTranslation()
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Weekly discounts')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <Button
          variant='ghost'
          className='h-auto min-h-12 max-w-full py-3 whitespace-normal'
          render={<Link to='/wallet' />}
        >
          {t('Wallet')}
        </Button>
      </SectionPageLayout.Actions>
      <SectionPageLayout.Content>
        <div className='mx-auto w-full max-w-4xl space-y-10 pb-12 sm:space-y-12'>
          {scope ? (
            <WeeklyDiscountAccess
              key={scope}
              scope={scope}
              selectedID={selectedID}
            />
          ) : (
            <p
              role='status'
              className='text-muted-foreground text-sm leading-7'
            >
              {t('Sign in again to view your private weekly discounts.')}
            </p>
          )}
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}

function WeeklyDiscountAccess({
  scope,
  selectedID,
}: {
  scope: string
  selectedID?: string
}) {
  const { status, capabilitiesReady, error, refetch } = useStatus()
  const { t } = useTranslation()
  if (error) {
    return (
      <div className='space-y-3'>
        <p role='alert'>{t('Could not refresh. Try again.')}</p>
        <Button variant='outline' onClick={() => void refetch()}>
          {t('Retry')}
        </Button>
      </div>
    )
  }
  if (
    !capabilitiesReady ||
    status?.backend_capabilities?.weekly_discounts !== true
  ) {
    return (
      <p role='status' className='text-muted-foreground text-sm leading-7'>
        {capabilitiesReady ? t('Capability unavailable') : t('Loading…')}
      </p>
    )
  }
  return (
    <>
      <p className='text-muted-foreground max-w-2xl text-sm leading-7'>
        {t(
          'Review your weekly decision, claim an offered code, and see past weeks.'
        )}
      </p>
      <OwnerDiscounts scope={scope} selectedID={selectedID} />
    </>
  )
}

function OwnerDiscounts({
  scope,
  selectedID,
}: {
  scope: string
  selectedID?: string
}) {
  const { t, i18n } = useTranslation()
  const [listing, setListing] = useState<{
    history: WeeklyHistory | null
    loading: boolean
    failed: boolean
  }>({
    history: null,
    loading: false,
    failed: false,
  })
  const { history, loading, failed } = listing
  const request = useRef<AbortController | null>(null)
  const mounted = useRef(false)
  const current = useCallback(
    () => mounted.current && ownerKey() === scope,
    [scope]
  )
  const load = useCallback(
    async (beforeWeek?: number) => {
      if (request.current || !current()) {
        return
      }
      const abort = new AbortController()
      request.current = abort
      setListing((old) => ({ ...old, loading: true, failed: false }))
      try {
        const page = await client.history(abort.signal, beforeWeek)
        if (!current() || abort.signal.aborted) {
          return
        }
        setListing((old) => {
          if (beforeWeek === undefined || !old.history) {
            return { history: page, loading: false, failed: false }
          }
          // A new server week needs a fresh first page before showing "no request".
          if (old.history.current_utc_week !== page.current_utc_week) {
            return { history: null, loading: false, failed: true }
          }
          const known = new Set(
            old.history.decisions.map((decision) => decision.id)
          )
          if (page.decisions.some((decision) => known.has(decision.id))) {
            return { ...old, loading: false, failed: true }
          }
          return {
            history: {
              ...page,
              decisions: [...old.history.decisions, ...page.decisions],
            },
            loading: false,
            failed: false,
          }
        })
      } catch {
        if (current() && !abort.signal.aborted) {
          setListing((old) => ({ ...old, failed: true }))
        }
      } finally {
        if (request.current === abort) {
          request.current = null
          if (current()) {
            setListing((old) => ({ ...old, loading: false }))
          }
        }
      }
    },
    [current]
  )
  useEffect(() => {
    mounted.current = true
    void load()
    return () => {
      mounted.current = false
      request.current?.abort()
      request.current = null
    }
  }, [load])
  const observed = useCallback(
    (decision: WeeklyDecisionSummary) => {
      if (!current()) {
        return
      }
      setListing((old) =>
        old.history
          ? {
              ...old,
              history: {
                ...old.history,
                decisions: old.history.decisions.map((row) =>
                  row.id === decision.id ? decision : row
                ),
              },
            }
          : old
      )
    },
    [current]
  )
  const thisWeek = history?.decisions.find(
    (decision) => decision.week_start === history.current_utc_week
  )
  const activeID = selectedID ?? thisWeek?.id
  const past =
    history?.decisions.filter(
      (decision) => decision.week_start < history.current_utc_week
    ) ?? []
  const displayingOther = Boolean(selectedID && selectedID !== thisWeek?.id)
  return (
    <>
      <section aria-labelledby='weekly-current-title' className='space-y-6'>
        <div className='flex flex-wrap items-center justify-between gap-3'>
          <div className='space-y-1'>
            <h2
              id='weekly-current-title'
              className='text-xl font-semibold tracking-tight'
            >
              {t('This week')}
            </h2>
            {history && (
              <p className='text-muted-foreground text-sm'>
                {t('Week of {{date}} (UTC)', {
                  date: weekDate(history.current_utc_week, i18n.language),
                })}
              </p>
            )}
          </div>
          <Button
            variant='outline'
            className='h-auto min-h-12 max-w-full py-3 whitespace-normal'
            disabled={loading}
            onClick={() => void load()}
          >
            {loading ? t('Loading…') : t('Refresh history')}
          </Button>
        </div>
        {failed && (
          <p role='alert' className='text-destructive text-sm leading-7'>
            {t(
              'Weekly history could not be refreshed. Try again to see the current week.'
            )}
          </p>
        )}
        {!history && loading && (
          <p role='status' className='text-muted-foreground text-sm'>
            {t('Loading…')}
          </p>
        )}
        {history && !thisWeek && (
          <div className='bg-muted/40 space-y-3 rounded-2xl p-6 sm:p-8'>
            <h3 className='text-lg font-medium'>{t('No request this week')}</h3>
            <p className='text-muted-foreground max-w-2xl text-sm leading-7'>
              {t(
                'Ask the assistant to review this week’s discount. A decline also uses this week’s opportunity.'
              )}
            </p>
          </div>
        )}
        {displayingOther && thisWeek && (
          <Button
            variant='outline'
            className='h-auto min-h-12 max-w-full py-3 whitespace-normal'
            render={
              <Link to='/weekly-discounts' search={{ id: thisWeek.id }} />
            }
          >
            {t('View this week')}
          </Button>
        )}
        {activeID && (
          <div className='space-y-4'>
            {displayingOther && (
              <h3 className='text-lg font-medium'>{t('Selected decision')}</h3>
            )}
            <DecisionView
              key={scope + ':' + activeID}
              id={activeID}
              scope={scope}
              currentWeek={history?.current_utc_week ?? null}
              onObserved={observed}
            />
          </div>
        )}
      </section>
      <section aria-labelledby='weekly-history-title' className='space-y-5'>
        <div className='space-y-2'>
          <h2 id='weekly-history-title' className='text-lg font-semibold'>
            {t('Previous weeks')}
          </h2>
          <p className='text-muted-foreground text-sm leading-7'>
            {t(
              'Past decisions stay in history and do not create another opportunity this week.'
            )}
          </p>
        </div>
        {history && past.length === 0 && (
          <p className='text-muted-foreground py-4 text-sm'>
            {t('No earlier weekly decisions')}
          </p>
        )}
        <ul className='divide-border divide-y'>
          {past.map((decision) => (
            <li key={decision.id}>
              <Link
                to='/weekly-discounts'
                search={{ id: decision.id }}
                aria-current={activeID === decision.id ? 'page' : undefined}
                className='hover:bg-muted/40 focus-visible:ring-ring/40 flex min-h-20 flex-wrap items-center justify-between gap-3 rounded-lg px-3 py-4 outline-none focus-visible:ring-3'
              >
                <span className='min-w-0 space-y-1'>
                  <span className='block text-sm font-medium'>
                    {t('Week of {{date}} (UTC)', {
                      date: weekDate(decision.week_start, i18n.language),
                    })}
                  </span>
                  <span className='text-muted-foreground block text-sm'>
                    {decision.status === 'claimed'
                      ? t('Claimed')
                      : decision.status === 'declined'
                        ? t('Declined')
                        : t('Expired offer')}
                  </span>
                </span>
                {decision.discount_percent > 0 && (
                  <span className='text-sm tabular-nums'>
                    {t('{{percent}}% off', {
                      percent: decision.discount_percent,
                    })}
                  </span>
                )}
              </Link>
            </li>
          ))}
        </ul>
        {history?.next_before_week != null && (
          <Button
            variant='outline'
            className='h-auto min-h-12 max-w-full py-3 whitespace-normal'
            disabled={loading}
            onClick={() => void load(history.next_before_week ?? undefined)}
          >
            {loading ? t('Loading…') : t('Load earlier weeks')}
          </Button>
        )}
      </section>
    </>
  )
}

function DecisionView({
  id,
  scope,
  currentWeek,
  onObserved,
}: {
  id: string
  scope: string
  currentWeek: number | null
  onObserved: (decision: WeeklyDecisionSummary) => void
}) {
  const { t, i18n } = useTranslation()
  const [session] = useState(
    () => new WeeklyDetailSession(id, client, () => ownerKey() === scope)
  )
  const state = useSyncExternalStore(
    session.subscribe,
    session.getSnapshot,
    session.getSnapshot
  )
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null)
  const mounted = useRef(false)
  useEffect(() => {
    mounted.current = true
    void session.query()
    const clear = () => {
      session.clear()
      setCopied(null)
    }
    const visibility = () => {
      if (document.hidden) {
        clear()
      }
    }
    window.addEventListener('pagehide', clear)
    document.addEventListener('visibilitychange', visibility)
    return () => {
      mounted.current = false
      session.clear()
      window.removeEventListener('pagehide', clear)
      document.removeEventListener('visibilitychange', visibility)
    }
  }, [session])
  useEffect(() => {
    if (state.phase === 'ready' && state.inspection) {
      onObserved(publicSummary(state.inspection.decision))
    }
  }, [onObserved, state.inspection, state.phase])
  const decision = state.inspection?.decision
  const past =
    decision && currentWeek !== null && decision.week_start < currentWeek
  const claimable =
    state.phase === 'ready' && state.inspection?.claimable && !past
  async function copyCode() {
    const code = session.getSnapshot().inspection?.decision.code
    if (!code || ownerKey() !== scope) {
      return
    }
    try {
      await navigator.clipboard.writeText(code)
      if (
        mounted.current &&
        ownerKey() === scope &&
        session.getSnapshot().inspection?.decision.code === code
      ) {
        setCopied('yes')
      }
    } catch {
      if (mounted.current && ownerKey() === scope) {
        setCopied('failed')
      }
    }
  }
  return (
    <div
      className='bg-card min-w-0 space-y-7 rounded-2xl p-6 sm:p-8'
      aria-busy={state.phase === 'loading' || state.phase === 'claiming'}
    >
      {decision && (
        <>
          <div className='space-y-3'>
            <p className='text-muted-foreground text-sm'>
              {currentWeek !== null && decision.week_start === currentWeek
                ? t('This week’s decision')
                : t('Week of {{date}} (UTC)', {
                    date: weekDate(decision.week_start, i18n.language),
                  })}
            </p>
            <h3 className='text-3xl font-semibold tracking-tight tabular-nums sm:text-4xl'>
              {decision.status === 'declined'
                ? t('Declined')
                : t('{{percent}}% off', { percent: decision.discount_percent })}
            </h3>
            <p className='text-sm font-medium'>
              {decision.status === 'claimed'
                ? t('Claimed')
                : decision.status === 'declined'
                  ? t('The weekly decision was declined.')
                  : past
                    ? t('Expired offer')
                    : t('Awaiting claim')}
            </p>
          </div>
          <dl className='space-y-6 text-sm'>
            <div className='space-y-2'>
              <dt className='text-muted-foreground'>{t('Decision reason')}</dt>
              <dd className='leading-7 wrap-anywhere whitespace-pre-wrap'>
                {decision.reason}
              </dd>
            </div>
            <div className='space-y-2'>
              <dt className='text-muted-foreground'>
                {t('Claim window ends')}
              </dt>
              <dd>{dateTime(decision.expires_at, i18n.language)}</dd>
            </div>
            {decision.claimed_at !== null && (
              <div className='space-y-2'>
                <dt className='text-muted-foreground'>{t('Claimed at')}</dt>
                <dd>{dateTime(decision.claimed_at, i18n.language)}</dd>
              </div>
            )}
          </dl>
          {past && (
            <p className='text-muted-foreground text-sm leading-7'>
              {t('This record belongs to an earlier week.')}
            </p>
          )}
          {decision.code && (
            <div className='bg-muted/50 space-y-4 rounded-xl p-5'>
              <p className='text-sm font-medium'>
                {t('Your private discount code')}
              </p>
              <output
                className='block font-mono text-base leading-7 wrap-anywhere'
                aria-label={t('Your private discount code')}
              >
                {decision.code}
              </output>
              <div className='flex flex-wrap items-center gap-3'>
                <Button
                  variant='outline'
                  className='h-auto min-h-12 max-w-full py-3 whitespace-normal'
                  onClick={() => void copyCode()}
                >
                  {t('Copy code')}
                </Button>
                {copied && (
                  <span role='status' className='text-muted-foreground text-sm'>
                    {copied === 'yes'
                      ? t('Code copied')
                      : t('Copy failed. Select and copy the code.')}
                  </span>
                )}
              </div>
              <p className='text-muted-foreground text-xs leading-6'>
                {t(
                  'The page clears the code when you leave or switch sessions.'
                )}
              </p>
            </div>
          )}
        </>
      )}
      {(state.phase === 'idle' || state.phase === 'loading') && (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('Loading…')}
        </p>
      )}
      {state.phase === 'claiming' && (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('Claiming this offer…')}
        </p>
      )}
      {state.phase === 'unknown' && (
        <p role='status' className='text-sm leading-7'>
          {t(
            'The claim result is unknown. Check this original decision before doing anything else. No new claim is sent automatically.'
          )}
        </p>
      )}
      {state.phase === 'error' && (
        <p role='alert' className='text-sm leading-7'>
          {t(
            'This decision could not be loaded. Check it again using your signed-in session.'
          )}
        </p>
      )}
      {state.phase === 'cleared' && (
        <p role='status' className='text-muted-foreground text-sm leading-7'>
          {t(
            'The private preview was cleared. Load this decision again to view it.'
          )}
        </p>
      )}
      {state.phase === 'ready' &&
        decision?.status === 'offered' &&
        !claimable && (
          <p className='text-muted-foreground text-sm leading-7'>
            {t(
              'This offer cannot be claimed now. It may have expired or require the original signed-in session.'
            )}
          </p>
        )}
      <div className='flex flex-wrap gap-3'>
        {claimable && (
          <Button
            className='h-auto min-h-12 max-w-full px-6 py-3 whitespace-normal'
            onClick={() => void session.claim()}
          >
            {t('Claim discount code')}
          </Button>
        )}
        {['unknown', 'error', 'cleared'].includes(state.phase) && (
          <Button
            variant='outline'
            className='h-auto min-h-12 max-w-full py-3 whitespace-normal'
            onClick={() => void session.query()}
          >
            {state.phase === 'unknown'
              ? t('Check original decision')
              : t('Load this decision')}
          </Button>
        )}
      </div>
    </div>
  )
}
