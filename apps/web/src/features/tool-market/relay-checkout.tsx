/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { useMutation, useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import type { z } from 'zod'

import { Button } from '@/components/ui/button'
import { toIntlLocale } from '@/i18n/languages'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { formatRelayUSD, readRelayOrder, relayOrderStates } from './relay-order'

export function RelayCheckout({ orderID }: { orderID: string }) {
  const { t, i18n } = useTranslation()
  const userID = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const authScope = { userId: userID, sessionId }
  const path = '/api/ssmcpd/orders/' + encodeURIComponent(orderID)
  const order = useQuery({
    queryKey: ['ssmcpd', userID, sessionId, 'order', orderID],
    enabled: !!userID,
    queryFn: async ({ signal }) =>
      readRelayOrder(
        (await api.get(path, { signal, authScope, skipErrorHandler: true }))
          .data,
        orderID,
        userID
      ),
    gcTime: 0,
  })
  const confirm = useMutation({
    mutationFn: async () => {
      const { data } = await api.post(
        path + '/confirm',
        { order_id: orderID },
        { authScope, skipErrorHandler: true }
      )
      return readRelayOrder(data, orderID, userID)
    },
    retry: false,
    onSettled: async () => {
      await order.refetch()
    },
  })
  const labels: Record<z.infer<typeof relayOrderStates>, string> = {
    preview: t('Awaiting confirmation'),
    payment_pending: t('Payment pending'),
    grant_pending: t('Relay quota pending'),
    settlement_pending: t('Pending settlement'),
    completed: t('Completed'),
    declined: t('Declined'),
    expired: t('Expired'),
    refund_required: t('Refund required'),
    refund_pending: t('Refund pending'),
    needs_review: t('Pending review'),
    refunded: t('Refunded'),
  }
  const value =
    order.data?.user_id === userID && order.data?.order_id === orderID
      ? order.data
      : undefined
  const refund =
    value?.state === 'refund_required' ||
    value?.state === 'refund_pending' ||
    value?.state === 'refunded'
  const support = refund || value?.state === 'needs_review'
  const confirmable =
    value &&
    [
      'preview',
      'payment_pending',
      'grant_pending',
      'settlement_pending',
    ].includes(value.state)
  const locale = toIntlLocale(i18n.language) ?? 'en-US'
  return (
    <main className='mx-auto max-w-lg space-y-5 p-6'>
      <h1 className='text-2xl font-semibold'>{t('SSMCP relay traffic')}</h1>
      {order.isPending && <p role='status'>{t('Loading...')}</p>}
      {value && (
        <>
          <p>
            {value.gib} GiB ×{' '}
            {formatRelayUSD(value.unit_price_quota_per_gib, locale)}/GiB
          </p>
          <p className='font-medium'>
            {t('Total')} · {formatRelayUSD(value.total_price_quota, locale)} (
            {new Intl.NumberFormat(locale).format(value.total_price_quota)}{' '}
            {t('Credits')})
          </p>
          <p>
            {t('Billing period')} · {value.period}
          </p>
          <p>
            {t('Status')} · {labels[value.state]}
          </p>
          {support && (
            <div className='space-y-2'>
              {value.state === 'refund_required' && (
                <p>
                  {t(
                    'Your payment was recorded, but the relay quota was not granted.'
                  )}
                </p>
              )}
              <p>
                {t('Order ID')} ·{' '}
                <code className='break-all'>{value.order_id}</code>
              </p>
              {refund && value.refund_ref && (
                <p>
                  {t('Refund reference')} ·{' '}
                  <code className='break-all'>{value.refund_ref}</code>
                </p>
              )}
              <a
                className='text-primary underline'
                href={
                  '/support?' +
                  new URLSearchParams({
                    category: refund ? 'refund' : 'billing',
                    referenceId: value.order_id,
                  })
                }
              >
                {t('Contact support')}
              </a>
            </div>
          )}
          {confirmable && (
            <Button
              disabled={confirm.isPending || order.isFetching || order.isError}
              onClick={() => confirm.mutate()}
            >
              {value.state === 'preview'
                ? t('Pay with account balance')
                : t('Retry')}
            </Button>
          )}
        </>
      )}
      {(order.isError || (confirmable && confirm.isError)) && (
        <p role='alert'>
          {order.isError
            ? t('Could not load records.')
            : t(
                'Could not verify the order. Check its status before retrying.'
              )}
        </p>
      )}
      <Button
        variant='outline'
        disabled={order.isFetching}
        onClick={() => void order.refetch()}
      >
        {t('Refresh')}
      </Button>
    </main>
  )
}
