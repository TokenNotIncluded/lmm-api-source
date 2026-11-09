/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
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
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
  CommerceFailure,
  currentCommerceScope,
  formatCredits,
  readPending,
  requestKey,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { formatTimestamp } from './lib'
import {
  grantInvalidationSchema,
  invalidateNativeGrant,
  listNativeGrantRecords,
  pendingGrantInvalidationSchema,
  type NativeGrantRecord,
  type NativeGrantRecords,
  type PendingGrantInvalidation,
} from './native-admin-api'

const recoveryArea = 'subscription-invalidation'
const statusLabels = {
  active: 'Active',
  scheduled: 'Scheduled',
  expired: 'Expired',
  invalidated: 'Invalidated',
  refunded: 'Refunded',
} as const

export function NativeSubscriptionRecords({
  scope,
  userId,
  onChanged,
}: {
  scope: CommerceScope
  userId?: number
  onChanged?: () => void
}) {
  const { t, i18n } = useTranslation()
  const role = useAuthStore((state) => state.auth.user?.role ?? 0)
  const canInvalidate = role >= ROLE.SUPER_ADMIN
  const [data, setData] = useState<NativeGrantRecords | null>(null)
  const [page, setPage] = useState(1)
  const [query, setQuery] = useState('')
  const [search, setSearch] = useState('')
  const [status, setStatus] = useState('all')
  const [refresh, setRefresh] = useState(0)
  const [loading, setLoading] = useState(true)
  const [loadFailed, setLoadFailed] = useState(false)
  const [target, setTarget] = useState<NativeGrantRecord | null>(null)
  const [reason, setReason] = useState('')
  const [pending, setPending] = useState<PendingGrantInvalidation | null>(null)
  const [busy, setBusy] = useState(false)
  const [refused, setRefused] = useState(false)
  const [storageBlocked, setStorageBlocked] = useState(false)
  const [notice, setNotice] = useState('')
  const mounted = useRef(true)
  const running = useRef(false)
  const current = () => mounted.current && currentCommerceScope(scope)
  useEffect(() => {
    mounted.current = true
    try {
      const saved = readPending(
        scope,
        recoveryArea,
        pendingGrantInvalidationSchema
      )
      setPending(saved)
      if (saved) setReason(saved.request.reason)
    } catch {
      setStorageBlocked(true)
    }
    return () => {
      mounted.current = false
    }
    // The caller remounts this workspace on account/session/target changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  useEffect(() => {
    let active = true
    setLoading(true)
    setLoadFailed(false)
    setData(null)
    void listNativeGrantRecords(scope, { page, query, status, userId })
      .then(
        (result) => {
          if (active && currentCommerceScope(scope)) setData(result)
        },
        () => {
          if (active && currentCommerceScope(scope)) setLoadFailed(true)
        }
      )
      .finally(() => {
        if (active && currentCommerceScope(scope)) setLoading(false)
      })
    return () => {
      active = false
    }
  }, [scope, userId, page, query, status, refresh])
  function remember(command: PendingGrantInvalidation | null) {
    try {
      savePending(scope, recoveryArea, command)
      setPending(command)
      return true
    } catch {
      setStorageBlocked(true)
      return false
    }
  }
  async function invalidate(event: FormEvent) {
    event.preventDefault()
    if (running.current || !current() || !canInvalidate || storageBlocked) {
      return
    }
    let command = pending
    if (!command) {
      if (!target) return
      const request = grantInvalidationSchema.safeParse({
        operation_key: requestKey(),
        user_id: target.user_id,
        reason,
      })
      if (!request.success) {
        setNotice(t('Invalid input'))
        return
      }
      command = { grant_id: target.id, request: request.data }
      if (!remember(command)) return
    }
    running.current = true
    setBusy(true)
    setRefused(false)
    setNotice('')
    try {
      await invalidateNativeGrant(scope, command)
      if (current()) {
        remember(null)
        setTarget(null)
        setReason('')
        setRefresh((value) => value + 1)
        onChanged?.()
      }
    } catch (error) {
      if (current()) {
        const rejected = error instanceof CommerceFailure && !error.uncertain
        setRefused(rejected)
        setNotice(
          t(
            rejected
              ? 'Request rejected. Refresh and review before trying again.'
              : 'Result unknown. Retry the saved request before starting another.'
          )
        )
      }
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }
  function close() {
    if (busy || (pending && !refused)) return
    if (pending && !remember(null)) return
    setTarget(null)
    setReason('')
    setNotice('')
  }
  const selectedId = pending?.grant_id ?? target?.id
  const selectedUser = pending?.request.user_id ?? target?.user_id
  return (
    <div className='space-y-4'>
      <form
        className='flex flex-col gap-3 sm:flex-row'
        onSubmit={(event) => {
          event.preventDefault()
          setPage(1)
          setQuery(search.trim())
        }}
      >
        <Input
          aria-label={t('Search subscription records')}
          placeholder={t('Search by user, email, or plan')}
          value={search}
          maxLength={200}
          onChange={(event) => setSearch(event.target.value)}
        />
        <NativeSelect
          aria-label={t('Filter by status')}
          value={status}
          className='min-h-11 sm:w-44'
          onChange={(event) => {
            setPage(1)
            setStatus(event.target.value)
          }}
        >
          <NativeSelectOption value='all'>
            {t('All statuses')}
          </NativeSelectOption>
          {Object.entries(statusLabels).map(([value, label]) => (
            <NativeSelectOption key={value} value={value}>
              {t(label)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <Button type='submit' variant='outline'>
          {t('Search')}
        </Button>
        <Button
          type='button'
          variant='outline'
          disabled={loading || busy}
          onClick={() => setRefresh((value) => value + 1)}
        >
          {t('Refresh')}
        </Button>
      </form>
      {storageBlocked && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Recovery storage is unavailable. No new request can be sent.')}
        </p>
      )}
      {loading ? (
        <LoadingState />
      ) : loadFailed || !data ? (
        <ErrorState
          title={t('Failed to load subscription records')}
          onRetry={() => setRefresh((value) => value + 1)}
        />
      ) : (
        <>
          <ol className='divide-y border-y'>
            {data.items.map((record) => (
              <li
                key={record.id}
                className='grid gap-3 py-4 sm:grid-cols-[minmax(0,1fr)_minmax(12rem,1fr)_auto]'
              >
                <div className='min-w-0 space-y-1'>
                  <p className='font-medium break-words'>
                    {record.plan?.terms.title ??
                      t(
                        record.source === 'imported'
                          ? 'Imported subscription'
                          : 'Subscriptions'
                      )}{' '}
                    · #{record.id}
                  </p>
                  <p className='text-muted-foreground text-sm break-words'>
                    {record.username || t('User')} · #{record.user_id}
                  </p>
                  <Badge variant='secondary'>
                    {t(statusLabels[record.status])}
                  </Badge>
                </div>
                <dl className='grid grid-cols-2 gap-3 text-sm'>
                  <div>
                    <dt className='text-muted-foreground'>{t('Used quota')}</dt>
                    <dd className='tabular-nums'>
                      {formatCredits(record.used_units, i18n.language)} /{' '}
                      {record.unlimited
                        ? t('Unlimited')
                        : formatCredits(record.limit_units, i18n.language)}
                    </dd>
                  </div>
                  <div>
                    <dt className='text-muted-foreground'>
                      {t('Reserved quota')}
                    </dt>
                    <dd className='tabular-nums'>
                      {formatCredits(record.reserved_units, i18n.language)}
                    </dd>
                  </div>
                  <div className='col-span-2'>
                    <dt className='text-muted-foreground'>{t('Expires at')}</dt>
                    <dd>{formatTimestamp(record.ends_at)}</dd>
                  </div>
                </dl>
                {canInvalidate && (
                  <div className='flex items-start'>
                    <Button
                      variant='outline'
                      disabled={
                        !record.active ||
                        record.status === 'refunded' ||
                        !!pending ||
                        busy ||
                        storageBlocked
                      }
                      onClick={() => {
                        setTarget(record)
                        setReason('')
                        setRefused(false)
                        setNotice('')
                      }}
                    >
                      {t('Invalidate')}
                    </Button>
                  </div>
                )}
              </li>
            ))}
          </ol>
          {!data.items.length && (
            <p className='text-muted-foreground py-4'>
              {t('No subscription records')}
            </p>
          )}
          <div className='flex items-center justify-between gap-3'>
            <Button
              variant='outline'
              disabled={page <= 1}
              onClick={() => setPage((value) => value - 1)}
            >
              {t('Previous')}
            </Button>
            <span className='text-muted-foreground text-sm'>
              {t('Page')} {data.page} /{' '}
              {Math.max(1, Math.ceil(data.total / data.page_size))}
            </span>
            <Button
              variant='outline'
              disabled={page * data.page_size >= data.total}
              onClick={() => setPage((value) => value + 1)}
            >
              {t('Next')}
            </Button>
          </div>
        </>
      )}
      <Dialog
        open={!!target || !!pending}
        onOpenChange={(open) => {
          if (!open) close()
        }}
      >
        <DialogContent className='sm:max-w-lg'>
          <DialogHeader>
            <DialogTitle>{t('Invalidate subscription')}</DialogTitle>
            <DialogDescription>
              {t(
                'Invalidation stops new subscription usage and renewals. Existing usage and payment history remain.'
              )}
            </DialogDescription>
          </DialogHeader>
          <form
            onSubmit={(event) => void invalidate(event)}
            className='space-y-4'
          >
            <p className='text-sm'>
              {t('Subscriptions')} #{selectedId} · {t('User')} #{selectedUser}
            </p>
            <div className='space-y-2'>
              <Label htmlFor='subscription-invalidation-reason'>
                {t('Reason')}
              </Label>
              <Input
                id='subscription-invalidation-reason'
                value={reason}
                required
                maxLength={500}
                disabled={busy || !!pending}
                onChange={(event) => setReason(event.target.value)}
              />
            </div>
            {notice && (
              <p role='alert' className='text-destructive text-sm'>
                {notice}
              </p>
            )}
            <DialogFooter>
              <Button
                type='button'
                variant='outline'
                disabled={busy || (!!pending && !refused)}
                onClick={close}
              >
                {t('Cancel')}
              </Button>
              <Button
                type='submit'
                disabled={
                  busy ||
                  storageBlocked ||
                  !canInvalidate ||
                  refused ||
                  !reason.trim()
                }
              >
                {t(pending ? 'Retry saved request' : 'Confirm')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  )
}
