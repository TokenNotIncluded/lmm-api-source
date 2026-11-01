/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import axios from 'axios'
import { Check, RefreshCw, ShieldAlert, X } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { appealStatusKeys } from '@/features/auth/lib/account-appeal'
import { toIntlLocale } from '@/i18n/languages'

import { listAccountActionRequests, reviewAccountActionRequest } from '../api'
import type {
  AccountActionRequestAdmin,
  AccountActionRequestStatus,
} from '../types'

function isNotFound(error: unknown): boolean {
  return (
    (error as { response?: { status?: number } } | null)?.response?.status ===
    404
  )
}

export function AccountActionRequestsPanel(props: { focusRequestId?: number }) {
  const { t, i18n } = useTranslation()
  const [requests, setRequests] = useState<AccountActionRequestAdmin[]>([])
  const [loading, setLoading] = useState(true)
  const [status, setStatus] = useState<AccountActionRequestStatus | 'all'>(
    'pending'
  )
  const [page, setPage] = useState(1)
  const [pagination, setPagination] = useState({
    page: 1,
    page_size: 20,
    total: 0,
  })
  const [available, setAvailable] = useState(true)
  const [reviewing, setReviewing] = useState<number | null>(null)
  const [notes, setNotes] = useState<Record<number, string>>({})

  const loadRequests = useCallback(async () => {
    setLoading(true)
    try {
      const response = await listAccountActionRequests(status, page)
      if (!response.success) {
        throw new Error(
          response.message || t('Unable to load account action requests')
        )
      }
      setRequests(response.data ?? [])
      setPagination(response.pagination)
      setAvailable(true)
    } catch (error) {
      if (isNotFound(error)) {
        setAvailable(false)
      } else {
        toast.error(
          error instanceof Error
            ? error.message
            : t('Unable to load account action requests')
        )
      }
    } finally {
      setLoading(false)
    }
  }, [t, status, page])

  useEffect(() => {
    void loadRequests()
  }, [loadRequests])

  useEffect(() => {
    if (
      props.focusRequestId === undefined ||
      loading ||
      typeof document === 'undefined'
    ) {
      return
    }
    document
      .getElementById(`account-action-request-${props.focusRequestId}`)
      ?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }, [loading, props.focusRequestId, requests])

  const review = async (
    request: AccountActionRequestAdmin,
    approve: boolean
  ) => {
    if (reviewing !== null) return
    const note = (notes[request.id] ?? '').trim()
    if (!approve && [...note].length < 2) {
      toast.error(
        t(
          'A rejection reason is required and must contain at least 2 characters.'
        )
      )
      return
    }
    setReviewing(request.id)
    try {
      const response = await reviewAccountActionRequest(
        request.id,
        approve ? 'approve' : 'reject',
        {
          note,
          expected_revision: request.revision,
          expected_auth_version: request.auth_version,
        }
      )
      if (!response.success) {
        throw new Error(
          response.message || t('Unable to review account action request')
        )
      }
      let successMessage = t('Account action request rejected')
      if (approve) {
        successMessage =
          request.kind === 'appeal'
            ? t('Account appeal approved')
            : t('Account disable request approved')
      }
      toast.success(successMessage)
      await loadRequests()
      setNotes((current) => {
        const next = { ...current }
        delete next[request.id]
        return next
      })
    } catch (error) {
      if (axios.isAxiosError(error) && error.response?.status === 409) {
        toast.error(
          t(
            'This request changed. Review the refreshed details before trying again.'
          )
        )
        if (status === 'all') await loadRequests()
        else {
          setPage(1)
          setStatus('all')
        }
        return
      }
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to review account action request')
      )
    } finally {
      setReviewing(null)
    }
  }

  if (!available) return null

  const dateTimeFormatter = new Intl.DateTimeFormat(
    toIntlLocale(i18n.language),
    {
      dateStyle: 'medium',
      timeStyle: 'short',
    }
  )

  return (
    <section className='border-border border-t pt-10'>
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <div>
          <div className='flex items-center gap-2'>
            <h2 className='text-sm font-semibold'>
              {t('Account safety review')}
            </h2>
            <Badge variant='secondary'>{pagination.total}</Badge>
          </div>
          <p className='text-muted-foreground mt-1 text-sm'>
            {t('Review account appeals. Approved users must sign in again.')}
          </p>
        </div>
        <Button
          variant='outline'
          size='sm'
          onClick={() => void loadRequests()}
          disabled={loading || reviewing !== null}
        >
          <RefreshCw
            data-icon='inline-start'
            className={loading ? 'animate-spin' : undefined}
          />
          {t('Refresh')}
        </Button>
      </div>

      <label className='mt-4 flex items-center gap-2 text-sm'>
        {t('Status')}
        <select
          aria-label={t('Status')}
          className='border-input bg-background rounded-md border px-3 py-2'
          value={status}
          disabled={loading || reviewing !== null}
          onChange={(event) => {
            const value = event.target.value
            if (
              value === 'all' ||
              value === 'pending' ||
              value === 'approved' ||
              value === 'rejected' ||
              value === 'superseded'
            ) {
              setPage(1)
              setStatus(value)
            }
          }}
        >
          <option value='all'>{t('All')}</option>
          {Object.entries(appealStatusKeys).map(([value, key]) => (
            <option key={value} value={value}>
              {t(key)}
            </option>
          ))}
        </select>
      </label>
      <div className='mt-4 flex items-center gap-3'>
        <Button
          variant='outline'
          size='sm'
          disabled={loading || reviewing !== null || pagination.page <= 1}
          onClick={() => setPage((value) => value - 1)}
        >
          {t('Previous')}
        </Button>
        <span className='text-muted-foreground text-sm'>
          {pagination.page} /{' '}
          {Math.max(1, Math.ceil(pagination.total / pagination.page_size))}
        </span>
        <Button
          variant='outline'
          size='sm'
          disabled={
            loading ||
            reviewing !== null ||
            pagination.page * pagination.page_size >= pagination.total
          }
          onClick={() => setPage((value) => value + 1)}
        >
          {t('Next')}
        </Button>
      </div>
      {loading && requests.length === 0 ? (
        <p className='text-muted-foreground mt-5 text-sm'>{t('Loading...')}</p>
      ) : null}
      {!loading && requests.length === 0 ? (
        <p className='text-muted-foreground mt-5 text-sm'>
          {t('No account safety requests.')}
        </p>
      ) : null}

      {Object.entries(notes)
        .filter(
          ([id]) => !requests.some((request) => String(request.id) === id)
        )
        .map(([id, note]) => (
          <label key={id} className='mt-5 grid max-w-3xl gap-2 text-sm'>
            {t('Administrator review note')} · #{id}
            <Textarea
              aria-label={`${t('Administrator review note')} #${id}`}
              value={note}
              onChange={(event) =>
                setNotes((current) => ({
                  ...current,
                  [id]: event.target.value,
                }))
              }
              maxLength={2000}
            />
          </label>
        ))}
      {requests.length > 0 ? (
        <div className='mt-6'>
          {requests.map((request) => {
            const isAppeal = request.kind === 'appeal'
            return (
              <article
                key={request.id}
                id={`account-action-request-${request.id}`}
                className='border-border border-b py-7'
              >
                <div className='flex flex-wrap items-start justify-between gap-3'>
                  <div className='min-w-0'>
                    <div className='flex items-center gap-2'>
                      <ShieldAlert className='size-4' aria-hidden='true' />
                      <p className='font-medium'>
                        {request.target_username || t('Unknown user')}
                      </p>
                    </div>
                    <p className='text-muted-foreground text-xs'>
                      {request.target_email || t('No email provided')} · #
                      {request.id} ·{' '}
                      {dateTimeFormatter.format(
                        new Date(request.created_at * 1000)
                      )}
                    </p>
                  </div>
                  <div className='flex flex-wrap items-center gap-2'>
                    <Badge variant={isAppeal ? 'secondary' : 'destructive'}>
                      {isAppeal
                        ? t('Account appeal')
                        : t('Disable recommendation')}
                    </Badge>
                    <Badge variant='outline'>
                      {t(appealStatusKeys[request.status])}
                    </Badge>
                  </div>
                </div>

                <div className='mt-5 max-w-3xl'>
                  <p className='text-xs font-medium'>{t('Request reason')}</p>
                  <p className='text-muted-foreground mt-1 text-sm whitespace-pre-wrap'>
                    {request.reason}
                  </p>
                  <p className='text-muted-foreground mt-2 text-xs'>
                    {t('Submitted by')}:{' '}
                    {request.requested_by_username || t('Unknown user')}
                  </p>
                </div>

                {request.status === 'pending' ||
                notes[request.id] !== undefined ? (
                  <>
                    <Textarea
                      className='mt-5 max-w-3xl rounded-xl'
                      rows={2}
                      maxLength={2000}
                      aria-label={t('Administrator review note')}
                      readOnly={request.status !== 'pending'}
                      placeholder={t(
                        'Add an administrator note; a rejection note is required.'
                      )}
                      value={notes[request.id] ?? ''}
                      onChange={(event) =>
                        setNotes((current) => ({
                          ...current,
                          [request.id]: event.target.value,
                        }))
                      }
                    />
                    {request.status === 'pending' ? (
                      <div className='mt-3 flex flex-wrap justify-end gap-2'>
                        <Button
                          size='sm'
                          variant='destructive'
                          onClick={() => void review(request, false)}
                          disabled={
                            loading ||
                            reviewing !== null ||
                            [...(notes[request.id] ?? '').trim()].length < 2
                          }
                        >
                          <X data-icon='inline-start' />
                          {t('Reject')}
                        </Button>
                        <Button
                          size='sm'
                          onClick={() => void review(request, true)}
                          disabled={loading || reviewing !== null}
                        >
                          <Check data-icon='inline-start' />
                          {isAppeal
                            ? t('Approve and unblock')
                            : t('Approve disable')}
                        </Button>
                      </div>
                    ) : null}
                  </>
                ) : null}
              </article>
            )
          })}
        </div>
      ) : null}
    </section>
  )
}
